//! Where an installed skill's content comes from, and re-pointing it at a new
//! git source.

use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::core::{
    error::AppError,
    git_fetcher, installer, path_guard,
    repo_lock::RepoLock,
    skill_metadata::is_valid_skill_dir,
    skill_store::{SkillRecord, SkillStore},
    skill_update::{resync_copy_targets, staged_path_for, swap_skill_directory},
    sync_metadata,
};

#[derive(Debug, Clone)]
pub struct GitSkillSource {
    pub clone_url: String,
    pub branch: Option<String>,
    pub subpath: Option<String>,
    pub locator_skill_id: Option<String>,
}

pub fn git_source_from_skill(skill: &SkillRecord) -> Result<GitSkillSource, AppError> {
    if let Some(resolved) = &skill.source_ref_resolved {
        return Ok(GitSkillSource {
            clone_url: resolved.clone(),
            branch: skill.source_branch.clone(),
            subpath: skill.source_subpath.clone(),
            locator_skill_id: skill_ssh_id(skill),
        });
    }

    match skill.source_type.as_str() {
        "git" => {
            let source_ref = skill
                .source_ref
                .as_ref()
                .ok_or_else(|| AppError::invalid_input("Git skill is missing its source URL"))?;
            let parsed = git_fetcher::parse_git_source(source_ref);
            Ok(GitSkillSource {
                clone_url: parsed.clone_url,
                // Prefer the branch resolved at install time — it survives
                // slash-branch tree URLs that the sync parse can't disambiguate.
                branch: skill.source_branch.clone().or(parsed.branch),
                subpath: skill.source_subpath.clone().or(parsed.subpath),
                locator_skill_id: None,
            })
        }
        "skillssh" => {
            let source_ref = skill.source_ref.as_ref().ok_or_else(|| {
                AppError::invalid_input("skills.sh skill is missing its source reference")
            })?;
            let (repo_source, fallback_skill_id) = source_ref
                .rsplit_once('/')
                .ok_or_else(|| AppError::invalid_input("Invalid skills.sh source reference"))?;
            Ok(GitSkillSource {
                clone_url: format!("https://github.com/{}.git", repo_source),
                branch: skill.source_branch.clone(),
                subpath: skill.source_subpath.clone(),
                locator_skill_id: Some(fallback_skill_id.to_string()),
            })
        }
        _ => Err(AppError::invalid_input(
            "Skill does not support git-based updates",
        )),
    }
}

fn skill_ssh_id(skill: &SkillRecord) -> Option<String> {
    if skill.source_type != "skillssh" {
        return None;
    }

    skill.source_ref.as_deref().and_then(|source_ref| {
        source_ref
            .rsplit_once('/')
            .map(|(_, skill_id)| skill_id.to_string())
    })
}

#[derive(Debug, Serialize)]
pub struct SetSourceResult {
    pub skill_id: String,
    pub name: String,
    /// Source type before the change (`local`, `import`, `git`, `skillssh`).
    pub previous_source_type: String,
    pub previous_source_ref: Option<String>,
    pub clone_url: String,
    pub subpath: Option<String>,
    pub branch: Option<String>,
    pub revision: String,
    /// Whether the new source's content differs from the hash recorded for the
    /// library copy. False means the re-point is metadata-only — no file is
    /// rewritten. Compared against the recorded hash, not a fresh hash of the
    /// central directory, so hand-edits made after install do not count as a
    /// difference (and are left in place, since no file work runs).
    pub content_changed: bool,
    pub dry_run: bool,
}

/// Resolve the skill directory inside a fresh checkout, strictly.
///
/// Unlike [`resolve_skill_dir`], an explicit subpath that does not land on a
/// valid skill directory inside `repo_dir` is an error rather than a silent
/// fallback to repo-wide discovery. Re-pointing establishes a *new* source of
/// truth for an already-installed skill, so guessing is worse than failing: a
/// typo would otherwise install some unrelated directory — or the whole repo —
/// over the existing central copy.
fn resolve_repoint_skill_dir(repo_dir: &Path, subpath: Option<&str>) -> Result<PathBuf, AppError> {
    let Some(subpath) = subpath else {
        return if is_valid_skill_dir(repo_dir) {
            Ok(repo_dir.to_path_buf())
        } else {
            Err(AppError::invalid_input(
                "Repository root is not a skill directory (no SKILL.md); pass --subpath",
            ))
        };
    };

    // `Path::join` returns the argument verbatim when it is absolute, and `..`
    // segments climb out, so the candidate must be checked before it is used.
    // `is_path_safe` canonicalizes both sides, which also catches symlinks that
    // point outside the checkout.
    let candidate = repo_dir.join(subpath);
    if !path_guard::is_path_safe(repo_dir, &candidate) {
        return Err(AppError::invalid_input(format!(
            "Subpath '{subpath}' resolves outside the repository"
        )));
    }
    if !candidate.is_dir() {
        return Err(AppError::not_found(format!(
            "Subpath '{subpath}' does not exist in the repository"
        )));
    }
    if !is_valid_skill_dir(&candidate) {
        return Err(AppError::invalid_input(format!(
            "Subpath '{subpath}' is not a skill directory (no SKILL.md)"
        )));
    }
    Ok(candidate)
}

/// Re-point an installed skill at a git source **in place**.
///
/// The skill row is updated by id, so the skill id, tags, preset membership and
/// deployment targets all survive. This is the only safe way to convert a
/// `local` skill to a `git` one: `install` reuses a central directory only when
/// the content hash matches exactly (see `installer::unique_skill_dest`) and
/// otherwise silently allocates `<name>-2`, while `remove` + `install` drops the
/// id and everything keyed to it.
///
/// When the new source's content differs from the current central copy the
/// command refuses unless `force` is set. Re-pointing is not an update: the
/// remote is not yet known to be the authoritative copy, so overwriting local
/// content that may exist nowhere else has to be a deliberate choice.
#[allow(clippy::too_many_arguments)]
pub fn set_git_source_internal(
    store: &SkillStore,
    skill_id: &str,
    git_url: &str,
    subpath: Option<&str>,
    branch: Option<&str>,
    proxy_url: Option<&str>,
    force: bool,
    dry_run: bool,
) -> Result<SetSourceResult, AppError> {
    let skill = store
        .get_skill_by_id(skill_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Skill not found"))?;

    // Validate before parsing: resolving a GitHub tree URL runs `ls-remote`, so
    // an unvalidated URL would reach the network first. `install` validates its
    // raw input the same way.
    git_fetcher::validate_git_url(git_url).map_err(AppError::git)?;
    let parsed = git_fetcher::parse_git_source_resolved(git_url, proxy_url);

    // An explicit flag wins over whatever the URL encodes. `--subpath ""` is the
    // caller saying "the skill is at the repo root", which is distinct from
    // omitting the flag and letting the URL decide.
    let branch = branch.map(str::to_string).or_else(|| parsed.branch.clone());
    let subpath = match subpath {
        Some("") => None,
        Some(value) => Some(value.to_string()),
        None => parsed.subpath.clone(),
    };

    let remote_revision =
        git_fetcher::resolve_remote_revision(&parsed.clone_url, branch.as_deref(), proxy_url)
            .map_err(|e| AppError::git(e.to_string()))?;

    let temp_dir = git_fetcher::clone_repo_ref_scoped(
        &parsed.clone_url,
        branch.as_deref(),
        subpath.as_deref(),
        None,
        proxy_url,
        None,
    )
    .map_err(AppError::classify_git_error)?;

    // Nothing before this point has written to the store, so a failure during
    // the network phase leaves no state to unwind — in particular the skill is
    // never left stuck in `updating`.
    let marked_updating = std::cell::Cell::new(false);
    let source_committed = std::cell::Cell::new(false);
    let outcome = (|| -> Result<(String, bool), AppError> {
        git_fetcher::checkout_revision(&temp_dir, &remote_revision).map_err(AppError::git)?;
        let skill_dir = resolve_repoint_skill_dir(&temp_dir, subpath.as_deref())?;
        let resolved_subpath = git_fetcher::relative_subpath(&temp_dir, &skill_dir);

        let new_hash =
            crate::core::content_hash::hash_directory(&skill_dir).map_err(AppError::io)?;
        let content_changed = skill.content_hash.as_deref() != Some(new_hash.as_str());

        // Report before refusing: inspecting a skill whose content differs is
        // exactly what --dry-run is for, so it must not need --force to run.
        if dry_run {
            return Ok((resolved_subpath.unwrap_or_default(), content_changed));
        }
        if content_changed && !force {
            return Err(AppError::invalid_input(
                "New source content differs from the current library copy; \
                 re-run with --dry-run to inspect, or --force to overwrite",
            ));
        }

        let _lock = RepoLock::acquire_foreground("set skill source").map_err(AppError::db)?;

        // The clone happened outside the lock, so the skill may have been
        // removed or re-pointed meanwhile. Re-read and refuse to apply a
        // decision made against a stale snapshot.
        let current = store
            .get_skill_by_id(skill_id)
            .map_err(AppError::db)?
            .ok_or_else(|| AppError::not_found("Skill was removed while fetching the source"))?;
        if current.central_path != skill.central_path
            || current.content_hash != skill.content_hash
            || current.source_type != skill.source_type
            || current.source_ref != skill.source_ref
        {
            return Err(AppError::invalid_input(
                "Skill changed while fetching the source; re-run the command",
            ));
        }

        store
            .update_skill_update_status(skill_id, "updating")
            .map_err(AppError::db)?;
        marked_updating.set(true);

        // Identical content needs no file work — swapping would rewrite the
        // central copy for a metadata-only change, and `installer` does not
        // copy exactly the set of files `content_hash` covers, so the rewrite
        // could alter files while still reporting `content_changed: false`.
        let description = if content_changed {
            let staged_path = staged_path_for(&skill.central_path);
            let install_result =
                installer::install_skill_dir_to_destination(&skill_dir, &skill.name, &staged_path)
                    .inspect_err(|_| {
                        let _ = std::fs::remove_dir_all(&staged_path);
                    })
                    .map_err(AppError::io)?;
            swap_skill_directory(&staged_path, Path::new(&skill.central_path))?;
            install_result.description
        } else {
            skill.description.clone()
        };

        store
            .update_skill_after_reinstall(
                &skill.id,
                &skill.name,
                description.as_deref(),
                "git",
                Some(&parsed.original_url),
                Some(&parsed.clone_url),
                resolved_subpath.as_deref(),
                branch.as_deref(),
                Some(&remote_revision),
                Some(&remote_revision),
                Some(&new_hash),
                "up_to_date",
            )
            .map_err(AppError::db)?;
        source_committed.set(true);
        resync_copy_targets(store, &skill.id)?;
        sync_metadata::write_all_from_db_unlocked(store).map_err(AppError::db)?;
        Ok((resolved_subpath.unwrap_or_default(), content_changed))
    })();

    git_fetcher::cleanup_temp(&temp_dir);

    match outcome {
        Ok((resolved_subpath, content_changed)) => Ok(SetSourceResult {
            skill_id: skill.id,
            name: skill.name,
            previous_source_type: skill.source_type,
            previous_source_ref: skill.source_ref,
            clone_url: parsed.clone_url,
            subpath: (!resolved_subpath.is_empty()).then_some(resolved_subpath),
            branch,
            revision: remote_revision,
            content_changed,
            dry_run,
        }),
        Err(e) => {
            // Only clear `updating` if this call actually set it. A refusal
            // (bad subpath, content differs without --force) touched nothing,
            // so marking the skill as errored would be a lie.
            //
            // `update_skill_check_state` always writes the revision column, so
            // it has to be given the one that matches whichever source the row
            // now describes: the new source's revision once the re-point
            // committed, otherwise the revision the old source already had.
            // Passing the newly resolved revision unconditionally would file a
            // commit from the new repo under a skill still pointing at the old
            // one; passing None would blank a revision that is still valid.
            if marked_updating.get() {
                let revision = if source_committed.get() {
                    Some(remote_revision.as_str())
                } else {
                    skill.remote_revision.as_deref()
                };
                let _ =
                    store.update_skill_check_state(skill_id, revision, "error", Some(&e.message));
            }
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_support::write_skill;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn repoint_accepts_a_valid_subpath() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path();
        write_skill(&repo.join("note-manager"), "note-manager");

        let resolved = resolve_repoint_skill_dir(repo, Some("note-manager")).unwrap();
        assert_eq!(resolved, repo.join("note-manager"));
    }

    #[test]
    fn repoint_accepts_repo_root_when_it_is_a_skill() {
        let tmp = tempdir().unwrap();
        write_skill(tmp.path(), "root-skill");

        let resolved = resolve_repoint_skill_dir(tmp.path(), None).unwrap();
        assert_eq!(resolved, tmp.path());
    }

    #[test]
    fn repoint_rejects_repo_root_without_skill_md() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("some-dir")).unwrap();

        let err = resolve_repoint_skill_dir(tmp.path(), None).unwrap_err();
        assert!(
            err.message.contains("not a skill directory"),
            "{}",
            err.message
        );
    }

    #[test]
    fn repoint_rejects_missing_subpath_instead_of_falling_back() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path();
        // A real skill exists elsewhere: the lenient resolver would discover it.
        write_skill(&repo.join("other"), "other");

        let err = resolve_repoint_skill_dir(repo, Some("typo")).unwrap_err();
        assert!(err.message.contains("does not exist"), "{}", err.message);
    }

    #[test]
    fn repoint_rejects_subpath_that_is_not_a_skill_dir() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path();
        fs::create_dir_all(repo.join("docs")).unwrap();

        let err = resolve_repoint_skill_dir(repo, Some("docs")).unwrap_err();
        assert!(
            err.message.contains("not a skill directory"),
            "{}",
            err.message
        );
    }

    #[test]
    fn repoint_rejects_absolute_subpath() {
        let tmp = tempdir().unwrap();
        let outside = tmp.path().join("outside");
        write_skill(&outside, "outside");
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();

        // `Path::join` returns an absolute argument verbatim, so without the
        // guard this would install a directory from outside the checkout.
        let err = resolve_repoint_skill_dir(&repo, Some(outside.to_str().unwrap())).unwrap_err();
        assert!(
            err.message.contains("outside the repository"),
            "{}",
            err.message
        );
    }

    #[test]
    fn repoint_rejects_parent_traversal_subpath() {
        let tmp = tempdir().unwrap();
        write_skill(&tmp.path().join("outside"), "outside");
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();

        let err = resolve_repoint_skill_dir(&repo, Some("../outside")).unwrap_err();
        assert!(
            err.message.contains("outside the repository"),
            "{}",
            err.message
        );
    }

    #[cfg(unix)]
    #[test]
    fn repoint_rejects_symlink_escaping_the_checkout() {
        let tmp = tempdir().unwrap();
        let outside = tmp.path().join("outside");
        write_skill(&outside, "outside");
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        std::os::unix::fs::symlink(&outside, repo.join("link")).unwrap();

        let err = resolve_repoint_skill_dir(&repo, Some("link")).unwrap_err();
        assert!(
            err.message.contains("outside the repository"),
            "{}",
            err.message
        );
    }
}
