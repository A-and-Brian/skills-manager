//! Recording an installed skill in the store, and choosing where it comes
//! from and goes to.

use std::path::{Path, PathBuf};

use crate::core::{
    central_repo,
    error::AppError,
    git_fetcher, installer, path_guard, scenario_service,
    skill_metadata::is_valid_skill_dir,
    skill_store::{SkillRecord, SkillStore},
    sync_metadata,
};

#[derive(Debug, Clone)]
pub struct InstallSourceMetadata {
    pub source_type: String,
    pub source_ref: Option<String>,
    pub source_ref_resolved: Option<String>,
    pub source_subpath: Option<String>,
    pub source_branch: Option<String>,
    pub source_revision: Option<String>,
    pub remote_revision: Option<String>,
    pub update_status: String,
}

pub fn store_installed_skill_unlocked(
    store: &SkillStore,
    result: &installer::InstallResult,
    metadata: &InstallSourceMetadata,
    active_scenario_id: Option<&str>,
) -> Result<String, AppError> {
    let now = chrono::Utc::now().timestamp_millis();
    let central_path = result.central_path.to_string_lossy().to_string();

    if let Some(existing) = store
        .get_skill_by_central_path(&central_path)
        .map_err(AppError::db)?
    {
        store
            .update_skill_after_reinstall(
                &existing.id,
                &result.name,
                result.description.as_deref(),
                &metadata.source_type,
                metadata.source_ref.as_deref(),
                metadata.source_ref_resolved.as_deref(),
                metadata.source_subpath.as_deref(),
                metadata.source_branch.as_deref(),
                metadata.source_revision.as_deref(),
                metadata.remote_revision.as_deref(),
                Some(&result.content_hash),
                &metadata.update_status,
            )
            .map_err(AppError::db)?;
        if let Some(scenario_id) = active_scenario_id {
            store
                .add_skill_to_scenario(scenario_id, &existing.id)
                .map_err(AppError::db)?;
        }
        sync_metadata::write_all_from_db_unlocked(store).map_err(AppError::db)?;

        if let Some(scenario_id) = active_scenario_id {
            if let Err(e) = sync_skill_to_active_preset(store, scenario_id, &existing.id) {
                log::warn!("Failed to sync reinstalled skill to preset: {e}");
            }
        }

        return Ok(existing.id);
    }

    let id = uuid::Uuid::new_v4().to_string();

    let record = SkillRecord {
        id: id.clone(),
        name: result.name.clone(),
        description: result.description.clone(),
        source_type: metadata.source_type.clone(),
        source_ref: metadata.source_ref.clone(),
        source_ref_resolved: metadata.source_ref_resolved.clone(),
        source_subpath: metadata.source_subpath.clone(),
        source_branch: metadata.source_branch.clone(),
        source_revision: metadata.source_revision.clone(),
        remote_revision: metadata.remote_revision.clone(),
        central_path,
        content_hash: Some(result.content_hash.clone()),
        enabled: true,
        created_at: now,
        updated_at: now,
        status: "ok".to_string(),
        update_status: metadata.update_status.clone(),
        last_checked_at: Some(now),
        last_check_error: None,
    };

    store.insert_skill(&record).map_err(AppError::db)?;
    if let Some(scenario_id) = active_scenario_id {
        store
            .add_skill_to_scenario(scenario_id, &id)
            .map_err(AppError::db)?;
    }
    sync_metadata::write_all_from_db_unlocked(store).map_err(AppError::db)?;

    if let Some(scenario_id) = active_scenario_id {
        if let Err(e) = sync_skill_to_active_preset(store, scenario_id, &id) {
            log::warn!("Failed to sync newly installed skill to preset: {e}");
        }
    }

    Ok(id)
}

/// Resolve which directory of a fresh checkout holds the skill.
///
/// Both inputs are attacker-reachable. `subpath` comes from the path segment of
/// a `…/tree/<branch>/<path>` URL the user pasted, and `skill_id` from the part
/// after `@` in a skills.sh shorthand, which `parse_skillssh_shorthand` does not
/// constrain to a single path segment. `Path::join` returns an absolute argument
/// verbatim and `..` segments climb out of the checkout, so both are checked for
/// containment: without that, `install` copies the resolved directory into the
/// library — which for a git-backed library can then be pushed to the user's
/// backup remote.
///
/// A subpath that stays inside the checkout but does not exist is a different
/// case: it is only recoverable when a skills.sh locator can find the skill by
/// id, which is how a skill that moved upstream is picked up again (#278).
/// Without a locator, falling through to repo-wide discovery would install or
/// update whatever that discovery happens to return — in a repository that
/// groups its skills, the entire `skills/` container.
pub fn resolve_skill_dir(
    repo_dir: &Path,
    subpath: Option<&str>,
    skill_id: Option<&str>,
) -> Result<PathBuf, AppError> {
    if let Some(subpath) = subpath {
        let candidate = repo_dir.join(subpath);
        if !path_guard::is_path_safe(repo_dir, &candidate) {
            return Err(AppError::invalid_input(format!(
                "Path '{subpath}' resolves outside the repository"
            )));
        }
        // With a locator to fall back on, the stored path is only taken when it
        // still holds a skill. An upstream reorganization can leave the path
        // occupied by a container or an unrelated directory, and copying that
        // over the installed skill is the same mistake as guessing — let the
        // locator look the skill up at its new home instead.
        let usable = if skill_id.is_some() {
            is_valid_skill_dir(&candidate)
        } else {
            candidate.is_dir()
        };
        if usable {
            return Ok(candidate);
        }
        if skill_id.is_none() {
            return Err(AppError::not_found(format!(
                "Path '{subpath}' does not exist in the repository"
            )));
        }
    }

    // `find_skill_dir` joins the locator id onto the checkout in several places
    // before falling back to a recursive search, so its answer is checked too.
    let resolved = git_fetcher::find_skill_dir(repo_dir, skill_id).map_err(AppError::git)?;
    if !path_guard::is_path_safe(repo_dir, &resolved) {
        return Err(AppError::invalid_input(
            "Resolved skill directory is outside the repository",
        ));
    }
    Ok(resolved)
}

pub fn resolve_skillssh_install_target(
    store: &SkillStore,
    source_ref: &str,
    skill_id: &str,
) -> Result<(String, PathBuf), AppError> {
    if let Some(existing) = store
        .get_skill_by_source_ref("skillssh", source_ref)
        .map_err(AppError::db)?
    {
        return Ok((existing.name, PathBuf::from(existing.central_path)));
    }

    let base_name = skill_id.trim();
    if base_name.is_empty() {
        return Err(AppError::invalid_input("Skill id is empty"));
    }

    let mut attempt = 1;
    loop {
        let candidate_name = if attempt == 1 {
            base_name.to_string()
        } else {
            format!("{base_name}-{attempt}")
        };
        let candidate_path = central_repo::skills_dir().join(&candidate_name);
        let candidate_path_str = candidate_path.to_string_lossy().to_string();
        let occupied = store
            .get_skill_by_central_path(&candidate_path_str)
            .map_err(AppError::db)?
            .is_some();

        if !occupied {
            return Ok((candidate_name, candidate_path));
        }

        attempt += 1;
    }
}

/// Sync a skill's files to all enabled tool adapter directories for the given preset.
/// Only performs sync if the preset is the currently active one.
pub(crate) fn sync_skill_to_active_preset(
    store: &SkillStore,
    scenario_id: &str,
    skill_id: &str,
) -> Result<(), AppError> {
    scenario_service::sync_skill_to_active_scenario(store, scenario_id, skill_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_support::write_skill;
    use std::fs;
    use tempfile::tempdir;

    // ── resolve_skill_dir: install / update / preview resolution ───────────
    //
    // Both inputs reach this from a URL the user pasted. The cases below are
    // the ones that let a crafted or merely wrong URL resolve to something the
    // caller did not ask for.

    #[test]
    fn resolve_accepts_a_subpath_inside_the_checkout() {
        let tmp = tempdir().unwrap();
        write_skill(&tmp.path().join("skills").join("pdf"), "pdf");

        let resolved = resolve_skill_dir(tmp.path(), Some("skills/pdf"), None).unwrap();
        assert_eq!(resolved, tmp.path().join("skills").join("pdf"));
    }

    #[test]
    fn resolve_rejects_parent_traversal_with_and_without_a_locator() {
        let tmp = tempdir().unwrap();
        write_skill(&tmp.path().join("outside"), "outside");
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();

        // A locator must not soften the traversal check: the escaping path is
        // refused either way, never quietly ignored in favour of discovery.
        for locator in [None, Some("outside")] {
            let err = resolve_skill_dir(&repo, Some("../outside"), locator).unwrap_err();
            assert!(
                err.message.contains("outside the repository"),
                "locator {locator:?}: {}",
                err.message
            );
        }
    }

    #[test]
    fn resolve_rejects_absolute_subpath() {
        let tmp = tempdir().unwrap();
        let outside = tmp.path().join("outside");
        write_skill(&outside, "outside");
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();

        let err = resolve_skill_dir(&repo, Some(outside.to_str().unwrap()), None).unwrap_err();
        assert!(
            err.message.contains("outside the repository"),
            "{}",
            err.message
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_rejects_subpath_symlinked_out_of_the_checkout() {
        let tmp = tempdir().unwrap();
        let outside = tmp.path().join("outside");
        write_skill(&outside, "outside");
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        std::os::unix::fs::symlink(&outside, repo.join("link")).unwrap();

        let err = resolve_skill_dir(&repo, Some("link"), None).unwrap_err();
        assert!(
            err.message.contains("outside the repository"),
            "{}",
            err.message
        );
    }

    #[test]
    fn resolve_rejects_a_locator_that_escapes_the_checkout() {
        // `owner/repo@../../x` survives parse_skillssh_shorthand, which only
        // checks the owner/repo half, so the locator itself can climb out.
        let tmp = tempdir().unwrap();
        write_skill(&tmp.path().join("outside"), "outside");
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();

        let err = resolve_skill_dir(&repo, None, Some("../outside")).unwrap_err();
        assert!(
            err.message.contains("outside the repository"),
            "{}",
            err.message
        );
    }

    #[test]
    fn resolve_refuses_a_missing_subpath_instead_of_discovering_the_container() {
        // The measured bug: a tree URL naming a directory that does not exist
        // installed the whole `skills/` container as one skill.
        let tmp = tempdir().unwrap();
        write_skill(&tmp.path().join("skills").join("pdf"), "pdf");

        let err = resolve_skill_dir(tmp.path(), Some("artifacts-builder"), None).unwrap_err();
        assert!(err.message.contains("does not exist"), "{}", err.message);
    }

    #[test]
    fn resolve_lets_a_locator_recover_a_skill_that_moved_upstream() {
        // #278's recovery path: the stored subpath is stale because upstream
        // reorganized, and the locator finds the skill at its new home.
        let tmp = tempdir().unwrap();
        write_skill(&tmp.path().join("skills").join("db"), "db");

        let resolved = resolve_skill_dir(tmp.path(), Some("db"), Some("db")).unwrap();
        assert_eq!(resolved, tmp.path().join("skills").join("db"));
    }

    #[test]
    fn resolve_lets_a_locator_override_a_path_that_is_no_longer_the_skill() {
        // The harder half of a reorganization: the stored path still exists,
        // but upstream turned it into a container and moved the skill. Taking
        // the path would copy the container over the installed skill.
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("db")).unwrap();
        write_skill(&tmp.path().join("db").join("nested"), "nested");
        write_skill(&tmp.path().join("skills").join("db"), "db");

        let resolved = resolve_skill_dir(tmp.path(), Some("db"), Some("db")).unwrap();
        assert_eq!(resolved, tmp.path().join("skills").join("db"));
    }

    #[test]
    fn resolve_errors_when_the_locator_finds_nothing() {
        // Still #278: no match must not fall through to a container or root.
        let tmp = tempdir().unwrap();
        write_skill(&tmp.path().join("skills").join("db"), "db");

        let err = resolve_skill_dir(tmp.path(), Some("gone"), Some("nope-not-here")).unwrap_err();
        assert!(err.message.contains("not found"), "{}", err.message);
    }
}
