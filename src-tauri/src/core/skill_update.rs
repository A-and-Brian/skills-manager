//! Replacing an installed skill's library copy: git updates, local
//! re-imports, and the removal preflight that guards both.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::core::{
    error::AppError,
    git_fetcher, installer,
    managed_skill::{managed_skill_by_id, ManagedSkillDto},
    repo_lock::RepoLock,
    skill_install::resolve_skill_dir,
    skill_source::git_source_from_skill,
    skill_store::{SkillRecord, SkillStore, SkillTargetRecord},
    sync_engine, sync_metadata,
};

#[derive(Debug, Serialize)]
pub struct UpdateSkillResult {
    pub skill: ManagedSkillDto,
    /// Whether the skill's file content actually changed.
    /// False when a monorepo commit didn't touch this skill's subdirectory.
    pub content_changed: bool,
    /// What the update would remove, when it declined because of it (#256).
    /// Non-empty means **nothing was changed**: show these and call again with
    /// `approved_removals` set to `removal_approval` if the user accepts.
    ///
    /// Empty on every ordinary update, including approved ones.
    pub pending_removals: Vec<PendingRemoval>,
    /// Identifies exactly what `pending_removals` describes. Passing it back
    /// approves *that* list against *that* revision and nothing else — if the
    /// remote moves on, or the skill writes another file while the dialog is
    /// open, the approval no longer matches and the user is asked again.
    pub removal_approval: Option<String>,
}

/// Stands in for a revision when binding a re-import's approval: there is no
/// remote to move on, but the removal set still has to be bound.
const REIMPORT_APPROVAL_DOMAIN: &str = "reimport";

/// Result of re-importing a local skill from its source path.
#[derive(Debug, Serialize)]
pub struct ReimportSkillResult {
    pub skill: ManagedSkillDto,
    /// Non-empty means **nothing was changed** — see [`UpdateSkillResult`].
    pub pending_removals: Vec<PendingRemoval>,
    /// Approves exactly `pending_removals` — see [`UpdateSkillResult`].
    pub removal_approval: Option<String>,
}

/// Where a path about to be removed lives.
#[derive(Debug, Clone, Serialize)]
pub struct PendingRemoval {
    /// [`LIBRARY_LOCATION`], or the key of the agent whose deployed copy holds
    /// it. The user needs to know which directory to go and rescue.
    pub location: String,
    pub path: String,
}

/// `PendingRemoval::location` for the central library, as opposed to an agent's
/// deployed copy.
pub const LIBRARY_LOCATION: &str = "library";

enum UpdateOutcome {
    Applied {
        content_changed: bool,
    },
    /// Declined, having changed nothing.
    Held {
        pending: Vec<PendingRemoval>,
        approval: String,
    },
}

/// Everything a replacement would take away — from the library and from every
/// copy-mode deployment of this skill.
///
/// `staged` is the tree about to be installed, or `None` when the library keeps
/// what it already has. Even then the deployments are torn down and rebuilt from
/// it, which loses files just as effectively, so they are always checked.
///
/// Compared against the *staged* tree rather than the source it came from: the
/// installer drops `.git` and every symlink, so anything else would report a
/// path as surviving that the swap goes on to remove.
pub(crate) fn pending_removals_for(
    store: &SkillStore,
    skill: &SkillRecord,
    staged: Option<&Path>,
) -> Result<Vec<PendingRemoval>, AppError> {
    let library = Path::new(&skill.central_path);
    let mut pending = Vec::new();

    if let Some(staged) = staged {
        for path in crate::core::removals::removed_paths(library, staged).map_err(AppError::io)? {
            pending.push(PendingRemoval {
                location: LIBRARY_LOCATION.to_string(),
                path,
            });
        }
    }

    let effective_new = staged.unwrap_or(library);
    for target in store
        .get_targets_for_skill(&skill.id)
        .map_err(AppError::db)?
    {
        if target.mode != "copy" {
            continue;
        }
        for path in
            crate::core::removals::removed_paths(Path::new(&target.target_path), effective_new)
                .map_err(AppError::io)?
        {
            pending.push(PendingRemoval {
                location: target.tool.clone(),
                path,
            });
        }
    }
    Ok(pending)
}

/// A stable name for one exact set of removals at one exact revision.
pub(crate) fn removal_approval_token(revision: &str, pending: &[PendingRemoval]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(revision.as_bytes());
    let mut rows: Vec<String> = pending
        .iter()
        .map(|p| format!("{}\u{0}{}", p.location, p.path))
        .collect();
    rows.sort();
    for row in rows {
        hasher.update(row.as_bytes());
        hasher.update([0]);
    }
    hex::encode(hasher.finalize())
}

/// Removes a staged directory unless the swap claimed it.
pub(crate) struct StagedPathGuard<'a> {
    path: &'a Path,
    armed: std::cell::Cell<bool>,
}

impl<'a> StagedPathGuard<'a> {
    pub(crate) fn new(path: &'a Path, armed: bool) -> Self {
        Self {
            path,
            armed: std::cell::Cell::new(armed),
        }
    }

    /// The swap has taken ownership of it; there is nothing left to clean.
    pub(crate) fn release(&self) {
        self.armed.set(false);
    }
}

impl Drop for StagedPathGuard<'_> {
    fn drop(&mut self) {
        if self.armed.get() {
            // Declining an update must leave nothing behind — a stray
            // `.name.staged-<uuid>` inside the library is picked up by the
            // metadata rebuild scan as a skill of its own.
            let _ = remove_path_if_exists(self.path);
        }
    }
}

/// Update an installed git-sourced skill.
///
/// `approved_removals` carries back the token from a previous call that
/// declined, approving exactly the list it reported at exactly that revision.
/// Without it — or with a stale one — an update that would take away files the
/// new version does not have stops and reports them instead, having changed
/// nothing. See [`crate::core::removals`].
///
/// Unattended callers pass `None` and simply do not update: nobody is there to
/// be asked, and applying anyway is what #256 was.
pub fn update_git_skill_internal(
    store: &SkillStore,
    skill_id: &str,
    proxy_url: Option<&str>,
    cancel: Option<&Arc<AtomicBool>>,
    approved_removals: Option<&str>,
) -> Result<UpdateSkillResult, AppError> {
    let skill = store
        .get_skill_by_id(skill_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Skill not found"))?;

    if !matches!(skill.source_type.as_str(), "git" | "skillssh") {
        return Err(AppError::invalid_input(
            "Only git-based skills can be updated",
        ));
    }

    let git_source = git_source_from_skill(&skill)?;
    git_fetcher::validate_git_url(&git_source.clone_url).map_err(AppError::git)?;
    let remote_revision = git_fetcher::resolve_remote_revision(
        &git_source.clone_url,
        git_source.branch.as_deref(),
        proxy_url,
    )
    .map_err(|e| {
        let message = e.to_string();
        let _ = store.update_skill_check_state(
            skill_id,
            skill.remote_revision.as_deref(),
            "error",
            Some(&message),
        );
        AppError::git(message)
    })?;

    store
        .update_skill_update_status(skill_id, "updating")
        .map_err(AppError::db)?;

    let temp_dir = git_fetcher::clone_repo_ref_scoped(
        &git_source.clone_url,
        git_source.branch.as_deref(),
        git_source.subpath.as_deref(),
        cancel,
        proxy_url,
        None,
    )
    .map_err(AppError::classify_git_error)?;
    let update_result = (|| -> Result<UpdateOutcome, AppError> {
        git_fetcher::checkout_revision(&temp_dir, &remote_revision).map_err(AppError::git)?;
        let skill_dir = resolve_skill_dir(
            &temp_dir,
            git_source.subpath.as_deref(),
            git_source.locator_skill_id.as_deref(),
        )?;

        let new_hash =
            crate::core::content_hash::hash_directory(&skill_dir).map_err(AppError::io)?;
        let content_changed = skill.content_hash.as_deref() != Some(new_hash.as_str());
        let source_subpath = git_fetcher::relative_subpath(&temp_dir, &skill_dir);
        let _lock = RepoLock::acquire_foreground("update installed skill").map_err(AppError::db)?;

        // Stage first, then compare. The tree that lands in the library is the
        // installer's output, not the raw checkout — it drops `.git` and every
        // symlink — so comparing against the checkout would report a path as
        // surviving that the swap then removes.
        let staged_path = staged_path_for(&skill.central_path);
        let install_result = if content_changed {
            Some(
                installer::install_skill_dir_to_destination(&skill_dir, &skill.name, &staged_path)
                    .inspect_err(|_| {
                        let _ = remove_path_if_exists(&staged_path);
                    })
                    .map_err(AppError::io)?,
            )
        } else {
            None
        };
        let staged_guard = StagedPathGuard::new(&staged_path, install_result.is_some());

        let pending = pending_removals_for(
            store,
            &skill,
            install_result.is_some().then_some(staged_path.as_path()),
        )?;

        // A confirmation answers one exact question: this revision, this list
        // as shown. It closes the window while the dialog is open — a push, or
        // a file that changes the list, re-asks. Note a directory the new
        // version drops is one entry, so a file created *inside* it afterwards
        // does not change the list; approving `outputs/` approves the subtree. It cannot close the window
        // between this scan and the removal itself: the repo lock holds off
        // Skills Manager, not the agent processes writing into these very
        // directories. Narrowing that further needs the directories frozen
        // before the scan, not another scan.
        let approval = removal_approval_token(&remote_revision, &pending);
        if !pending.is_empty() && approved_removals != Some(approval.as_str()) {
            // Declining is not a failure: nothing was touched and the update is
            // still waiting. Clear the `updating` marker here, inside the lock,
            // rather than after releasing it — doing it later lets a concurrent
            // update overwrite the state, and swallowing the error would leave
            // the skill showing "updating" forever.
            store
                .update_skill_check_state(
                    &skill.id,
                    Some(&remote_revision),
                    "update_available",
                    None,
                )
                .map_err(AppError::db)?;
            return Ok(UpdateOutcome::Held { pending, approval });
        }

        if let Some(install_result) = install_result {
            swap_skill_directory(&staged_path, Path::new(&skill.central_path))?;
            // Only now is it the library's. Releasing before the swap left the
            // staged directory behind whenever its first rename failed.
            staged_guard.release();

            store
                .update_skill_source_metadata(
                    &skill.id,
                    Some(&git_source.clone_url),
                    source_subpath.as_deref(),
                    git_source.branch.as_deref(),
                    Some(&remote_revision),
                )
                .map_err(AppError::db)?;
            store
                .update_skill_after_install(
                    &skill.id,
                    &skill.name,
                    install_result.description.as_deref(),
                    Some(&remote_revision),
                    Some(&remote_revision),
                    Some(&install_result.content_hash),
                    "up_to_date",
                )
                .map_err(AppError::db)?;
            resync_copy_targets(store, &skill.id)?;
            sync_metadata::write_all_from_db_unlocked(store).map_err(AppError::db)?;
        } else {
            store
                .update_skill_source_metadata(
                    &skill.id,
                    Some(&git_source.clone_url),
                    source_subpath.as_deref(),
                    git_source.branch.as_deref(),
                    Some(&remote_revision),
                )
                .map_err(AppError::db)?;
            store
                .update_skill_check_state(&skill.id, Some(&remote_revision), "up_to_date", None)
                .map_err(AppError::db)?;
            resync_copy_targets(store, &skill.id)?;
            sync_metadata::write_all_from_db_unlocked(store).map_err(AppError::db)?;
        }
        Ok(UpdateOutcome::Applied { content_changed })
    })();
    git_fetcher::cleanup_temp(&temp_dir);

    match update_result {
        Ok(outcome) => {
            let (content_changed, pending_removals, removal_approval) = match outcome {
                UpdateOutcome::Applied { content_changed } => (content_changed, Vec::new(), None),
                UpdateOutcome::Held { pending, approval } => (false, pending, Some(approval)),
            };
            let skill = managed_skill_by_id(store, skill_id)?;
            Ok(UpdateSkillResult {
                skill,
                content_changed,
                pending_removals,
                removal_approval,
            })
        }
        Err(e) => {
            let _ = store.update_skill_check_state(
                skill_id,
                Some(&remote_revision),
                "error",
                Some(&e.message),
            );
            Err(e)
        }
    }
}

/// Re-import a local skill from its recorded source path.
///
/// `approved_removals` mirrors the git path: without it — or with one that no
/// longer matches the recomputed list — a re-import that would take away files
/// the source does not have stops and reports them.
pub fn reimport_local_skill_internal(
    store: &SkillStore,
    skill_id: &str,
    approved_removals: Option<&str>,
) -> Result<ReimportSkillResult, AppError> {
    let skill = store
        .get_skill_by_id(skill_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Skill not found"))?;

    if !matches!(skill.source_type.as_str(), "local" | "import") {
        return Err(AppError::invalid_input(
            "Only local skills can be reimported",
        ));
    }

    let source_path = skill
        .source_ref
        .clone()
        .ok_or_else(|| AppError::not_found("Local skill is missing its original source path"))?;
    let path = PathBuf::from(&source_path);
    if !path.exists() {
        store
            .update_skill_check_state(
                &skill.id,
                None,
                "source_missing",
                Some("Original source path no longer exists"),
            )
            .map_err(AppError::db)?;
        return Err(AppError::not_found("Original source path no longer exists"));
    }

    store
        .update_skill_update_status(skill_id, "updating")
        .map_err(AppError::db)?;

    let result = (|| -> Result<(Vec<PendingRemoval>, Option<String>), AppError> {
        let _lock = RepoLock::acquire_foreground("reimport local skill").map_err(AppError::db)?;
        let staged_path = staged_path_for(&skill.central_path);
        let install_result =
            installer::install_from_local_to_destination(&path, Some(&skill.name), &staged_path)
                .inspect_err(|_| {
                    let _ = remove_path_if_exists(&staged_path);
                })
                .map_err(AppError::io)?;
        let staged_guard = StagedPathGuard::new(&staged_path, true);

        // Same replacement, same guard. Re-importing is explicit about the
        // *source*, not about discarding whatever has accumulated in the
        // library since — and for a local skill the "update" button runs this,
        // so leaving it uncovered would guard one path and not its twin.
        let pending = pending_removals_for(store, &skill, Some(&staged_path))?;
        // Bound to the set itself, not to a constant. A constant would match on
        // the approving call no matter what the recomputed list said, so a file
        // written while the dialog was open would be deleted having never been
        // shown — which is the whole failure this is here to prevent.
        let approval = removal_approval_token(REIMPORT_APPROVAL_DOMAIN, &pending);
        if !pending.is_empty() && approved_removals != Some(approval.as_str()) {
            // Restore the status this started from rather than asserting one:
            // declining changed nothing, so nothing about the skill's state
            // should read differently afterwards.
            store
                .update_skill_update_status(&skill.id, &skill.update_status)
                .map_err(AppError::db)?;
            return Ok((pending, Some(approval)));
        }

        swap_skill_directory(&staged_path, Path::new(&skill.central_path))?;
        // Only now is it the library's; before this the guard still owns it.
        staged_guard.release();
        store
            .update_skill_after_install(
                &skill.id,
                &skill.name,
                install_result.description.as_deref(),
                None,
                None,
                Some(&install_result.content_hash),
                "local_only",
            )
            .map_err(AppError::db)?;
        resync_copy_targets(store, &skill.id)?;
        sync_metadata::write_all_from_db_unlocked(store).map_err(AppError::db)?;
        Ok((Vec::new(), None))
    })();

    match result {
        Ok((pending_removals, removal_approval)) => Ok(ReimportSkillResult {
            skill: managed_skill_by_id(store, skill_id)?,
            pending_removals,
            removal_approval,
        }),
        Err(e) => {
            let _ = store.update_skill_check_state(skill_id, None, "error", Some(&e.message));
            Err(e)
        }
    }
}

pub fn staged_path_for(central_path: &str) -> PathBuf {
    let path = PathBuf::from(central_path);
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "skill".to_string());
    path.with_file_name(format!(".{file_name}.staged-{}", uuid::Uuid::new_v4()))
}

pub fn swap_skill_directory(staged_path: &Path, current_path: &Path) -> Result<(), AppError> {
    let backup_path = current_path.with_file_name(format!(
        ".{}.backup-{}",
        current_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "skill".to_string()),
        uuid::Uuid::new_v4()
    ));

    if current_path.exists() {
        std::fs::rename(current_path, &backup_path)?;
    }

    if let Err(err) = std::fs::rename(staged_path, current_path) {
        if backup_path.exists() {
            let _ = std::fs::rename(&backup_path, current_path);
        }
        let _ = remove_path_if_exists(staged_path);
        return Err(err.into());
    }

    remove_path_if_exists(&backup_path)?;
    Ok(())
}

pub fn resync_copy_targets(store: &SkillStore, skill_id: &str) -> Result<(), AppError> {
    let skill = store
        .get_skill_by_id(skill_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Skill not found"))?;
    let source = PathBuf::from(&skill.central_path);
    let targets = store
        .get_targets_for_skill(skill_id)
        .map_err(AppError::db)?;

    for target in targets {
        if target.mode != "copy" {
            continue;
        }

        // Recorded: this walks existing rows, so each path is one we wrote.
        // The row's mode is filtered to "copy" above, and sync_engine still
        // refuses if what is on disk no longer matches that record.
        sync_engine::sync_skill(
            &source,
            Path::new(&target.target_path),
            sync_engine::SyncMode::Copy,
            sync_engine::ReplacePolicy::Recorded {
                mode: target.mode.as_str(),
            },
        )
        .map_err(AppError::io)?;

        let updated_target = SkillTargetRecord {
            synced_at: Some(chrono::Utc::now().timestamp_millis()),
            status: "ok".to_string(),
            last_error: None,
            // Refresh the hash so the startup freshness check (#153)
            // sees this resync as up-to-date instead of stale.
            source_hash: skill.content_hash.clone(),
            ..target
        };
        store.insert_target(&updated_target).map_err(AppError::db)?;
    }

    Ok(())
}

pub(crate) fn remove_path_if_exists(path: &Path) -> Result<(), AppError> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_support::{sample_skill, test_repo, write_skill_dir};
    use std::fs;

    /// The whole point of the preflight: it must see the user's file in the
    /// library *and* the one in an agent's deployed copy, and say which is
    /// which — a bare filename does not tell anyone where to go and rescue it.
    #[test]
    fn the_preflight_covers_the_library_and_every_deployed_copy() {
        let repo = test_repo();
        let central = write_skill_dir("ppt-master");
        fs::create_dir_all(central.join("templates")).unwrap();
        fs::write(central.join("templates/mine.pptx"), "user work").unwrap();
        repo.store
            .insert_skill(&sample_skill("skill-1", "ppt-master", &central))
            .unwrap();

        // A copy-mode deployment the user has also written into.
        let target_dir = repo._tmp.path().join("agent/ppt-master");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("SKILL.md"), "x").unwrap();
        fs::write(target_dir.join("notes.md"), "notes in the agent copy").unwrap();
        repo.store
            .insert_target(&SkillTargetRecord {
                id: "t1".to_string(),
                skill_id: "skill-1".to_string(),
                tool: "claude_code".to_string(),
                target_path: target_dir.to_string_lossy().to_string(),
                mode: "copy".to_string(),
                status: "ok".to_string(),
                synced_at: Some(1),
                last_error: None,
                source_hash: None,
            })
            .unwrap();

        // The new version carries only SKILL.md.
        let staged = repo._tmp.path().join("staged");
        fs::create_dir_all(&staged).unwrap();
        fs::write(staged.join("SKILL.md"), "v2").unwrap();

        let skill = repo.store.get_skill_by_id("skill-1").unwrap().unwrap();
        let pending = pending_removals_for(&repo.store, &skill, Some(&staged)).unwrap();

        let found: Vec<(String, String)> = pending
            .iter()
            .map(|p| (p.location.clone(), p.path.replace('\\', "/")))
            .collect();
        assert!(
            found.contains(&(LIBRARY_LOCATION.to_string(), "templates/".to_string())),
            "the library's own directory must be reported: {found:?}"
        );
        assert!(
            found.contains(&("claude_code".to_string(), "notes.md".to_string())),
            "the agent copy is torn down and rebuilt too: {found:?}"
        );
    }

    /// With no content change nothing is swapped, so the library keeps what it
    /// has — but the deployments are still rebuilt from it, which is its own way
    /// to lose a file.
    #[test]
    fn a_metadata_only_update_still_checks_the_deployed_copies() {
        let repo = test_repo();
        let central = write_skill_dir("stable");
        repo.store
            .insert_skill(&sample_skill("skill-1", "stable", &central))
            .unwrap();

        let target_dir = repo._tmp.path().join("agent/stable");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("SKILL.md"), "x").unwrap();
        fs::write(target_dir.join("mine.txt"), "only in the agent copy").unwrap();
        repo.store
            .insert_target(&SkillTargetRecord {
                id: "t1".to_string(),
                skill_id: "skill-1".to_string(),
                tool: "cursor".to_string(),
                target_path: target_dir.to_string_lossy().to_string(),
                mode: "copy".to_string(),
                status: "ok".to_string(),
                synced_at: Some(1),
                last_error: None,
                source_hash: None,
            })
            .unwrap();

        let skill = repo.store.get_skill_by_id("skill-1").unwrap().unwrap();
        // `None` staged: the library is unchanged, and is itself the baseline.
        let pending = pending_removals_for(&repo.store, &skill, None).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].location, "cursor");
        assert_eq!(pending[0].path, "mine.txt");
    }

    /// Symlink-mode deployments are not copied over, so they are not at risk and
    /// must not generate noise.
    #[test]
    fn symlink_deployments_are_not_reported() {
        let repo = test_repo();
        let central = write_skill_dir("linked");
        repo.store
            .insert_skill(&sample_skill("skill-1", "linked", &central))
            .unwrap();

        let target_dir = repo._tmp.path().join("agent/linked");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("whatever.md"), "x").unwrap();
        repo.store
            .insert_target(&SkillTargetRecord {
                id: "t1".to_string(),
                skill_id: "skill-1".to_string(),
                tool: "grok".to_string(),
                target_path: target_dir.to_string_lossy().to_string(),
                mode: "symlink".to_string(),
                status: "ok".to_string(),
                synced_at: Some(1),
                last_error: None,
                source_hash: None,
            })
            .unwrap();

        let skill = repo.store.get_skill_by_id("skill-1").unwrap().unwrap();
        assert!(pending_removals_for(&repo.store, &skill, None)
            .unwrap()
            .is_empty());
    }

    /// An approval answers one exact question: this revision, this list.
    #[test]
    fn an_approval_does_not_carry_to_a_different_revision_or_list() {
        let a = vec![PendingRemoval {
            location: LIBRARY_LOCATION.to_string(),
            path: "templates/mine.pptx".to_string(),
        }];
        let mut b = a.clone();
        b.push(PendingRemoval {
            location: LIBRARY_LOCATION.to_string(),
            path: "templates/another.pptx".to_string(),
        });

        assert_eq!(
            removal_approval_token("rev1", &a),
            removal_approval_token("rev1", &a),
            "the same question must produce the same token"
        );
        assert_ne!(
            removal_approval_token("rev1", &a),
            removal_approval_token("rev2", &a),
            "upstream moved on"
        );
        assert_ne!(
            removal_approval_token("rev1", &a),
            removal_approval_token("rev1", &b),
            "the skill wrote another file while the dialog was open"
        );
    }

    /// Drives the real `reimport_local_skill_internal`, because the bug this
    /// guards against was in the wiring, not the hash: the approval was compared
    /// against a constant, so the recomputed list was never consulted. A test
    /// that only calls the token function twice passes either way.
    #[test]
    fn a_stale_reimport_approval_does_not_authorize_a_grown_list() {
        let repo = test_repo();
        let source = repo._tmp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("SKILL.md"), "---\nname: gen\n---\n").unwrap();

        let central = write_skill_dir("gen");
        fs::write(central.join("mine.txt"), "user work").unwrap();
        let mut record = sample_skill("skill-1", "gen", &central);
        record.source_ref = Some(source.to_string_lossy().to_string());
        repo.store.insert_skill(&record).unwrap();

        // First attempt: held, with a token for the list the user is shown.
        let first = reimport_local_skill_internal(&repo.store, "skill-1", None).unwrap();
        assert_eq!(first.pending_removals.len(), 1);
        let shown = first.removal_approval.clone().unwrap();
        assert!(central.join("mine.txt").is_file(), "nothing may be touched");

        // The skill writes another file while the dialog is open.
        fs::write(central.join("appeared-later.txt"), "also mine").unwrap();

        // The old approval must not cover it.
        let second = reimport_local_skill_internal(&repo.store, "skill-1", Some(&shown)).unwrap();
        assert_eq!(
            second.pending_removals.len(),
            2,
            "the grown list must be shown again, not silently applied"
        );
        assert!(central.join("appeared-later.txt").is_file());
        assert!(central.join("mine.txt").is_file());

        // Approving the list actually shown does go through.
        let approved = second.removal_approval.clone().unwrap();
        let third = reimport_local_skill_internal(&repo.store, "skill-1", Some(&approved)).unwrap();
        assert!(third.pending_removals.is_empty());
        assert!(
            !central.join("mine.txt").exists(),
            "the approved removal applies"
        );
    }
}
