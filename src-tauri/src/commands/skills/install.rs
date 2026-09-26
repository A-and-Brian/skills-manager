//! Installing skills from a local path, git or skills.sh, including the git preview flow.

use std::path::{Path, PathBuf};
use tauri::State;

use super::types::{CancelRegistrationGuard, GitPreviewResult, GitSkillPreview, SkillInstallItem};

use crate::core::{
    audit_log::AuditDraft,
    error::AppError,
    git_fetcher,
    host::HostCtx,
    installer,
    repo_lock::RepoLock,
    scanner,
    skill_install::{
        resolve_skill_dir, resolve_skillssh_install_target, store_installed_skill_unlocked,
        InstallSourceMetadata,
    },
    skill_metadata::{self, is_valid_skill_dir},
    skill_store::SkillStore,
};

/// Append an audit log entry summarising an install attempt.
/// `source_label` is short text identifying the source (e.g. "local", "git", "skillssh").
fn log_install_outcome(
    store: &SkillStore,
    source_label: &str,
    outcome: Result<&(String, String), &AppError>,
) {
    let draft = AuditDraft::new("install").detail(source_label);
    let draft = match outcome {
        Ok((id, name)) => draft.skill(id.clone(), name.clone()).ok(),
        Err(e) => draft.fail(e.to_string()),
    };
    store.log_audit(draft);
}

#[tauri::command]
pub async fn install_local(
    source_path: String,
    name: Option<String>,
    ctx: State<'_, HostCtx>,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || install_local_core(&ctx, source_path, name))
        .await?
}

pub fn install_local_core(
    ctx: &HostCtx,
    source_path: String,
    name: Option<String>,
) -> Result<(), AppError> {
    let store = ctx.store.clone();
    let outcome = (|| -> Result<(String, String), AppError> {
        let path = PathBuf::from(&source_path);
        let metadata = InstallSourceMetadata {
            source_type: "local".to_string(),
            source_ref: Some(source_path.clone()),
            source_ref_resolved: None,
            source_subpath: None,
            source_branch: None,
            source_revision: None,
            remote_revision: None,
            update_status: "local_only".to_string(),
        };
        let _lock = RepoLock::acquire_foreground("install local skill").map_err(AppError::db)?;
        let result = installer::install_from_local(&path, name.as_deref()).map_err(AppError::io)?;
        let skill_name = result.name.clone();
        // Install only adds the skill to the central library; preset
        // membership is an explicit action (see issue #213).
        let skill_id = store_installed_skill_unlocked(&store, &result, &metadata, None)?;
        Ok((skill_id, skill_name))
    })();
    log_install_outcome(&store, "local", outcome.as_ref());
    outcome.map(|_| ())
}

#[tauri::command]
pub async fn install_git(
    repo_url: String,
    name: Option<String>,
    ctx: State<'_, HostCtx>,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || install_git_core(&ctx, repo_url, name)).await?
}

pub fn install_git_core(
    ctx: &HostCtx,
    repo_url: String,
    name: Option<String>,
) -> Result<(), AppError> {
    let store = ctx.store.clone();
    let proxy_url = store.proxy_url();
    let registry = ctx.cancel.clone();
    let cancel_key = repo_url.clone();
    let cancel = registry.register(&cancel_key);
    let _cancel_guard = CancelRegistrationGuard::new(registry.clone(), cancel_key);

    let emit_progress = |phase: &str| {
        ctx.events.emit(
            "install-progress",
            serde_json::json!({
                "skill_id": repo_url,
                "phase": phase,
            }),
        );
    };

    let outcome = (|| -> Result<(String, String), AppError> {
        git_fetcher::validate_git_url(&repo_url).map_err(AppError::git)?;
        emit_progress("cloning");
        let parsed = git_fetcher::parse_git_source_resolved(&repo_url, proxy_url.as_deref());
        let events_for_progress = ctx.events.clone();
        let url_for_progress = repo_url.clone();
        let progress_cb: git_fetcher::ProgressCallback = Box::new(move |msg: &str| {
            events_for_progress.emit(
                "install-progress",
                serde_json::json!({
                    "skill_id": url_for_progress,
                    "phase": "cloning",
                    "detail": msg,
                }),
            );
        });
        let temp_dir = git_fetcher::clone_repo_ref_scoped(
            &parsed.clone_url,
            parsed.branch.as_deref(),
            parsed.subpath.as_deref(),
            Some(&cancel),
            proxy_url.as_deref(),
            Some(progress_cb),
        )
        .map_err(AppError::classify_git_error)?;

        emit_progress("installing");
        let install_result = (|| -> Result<(String, String), AppError> {
            let _lock = RepoLock::acquire_foreground("install git skill").map_err(AppError::db)?;
            let skill_dir = resolve_skill_dir(&temp_dir, parsed.subpath.as_deref(), None)?;
            let revision = git_fetcher::get_head_revision(&temp_dir).map_err(AppError::git)?;
            let result = installer::install_from_git_dir(&skill_dir, name.as_deref())
                .map_err(AppError::io)?;
            let metadata = InstallSourceMetadata {
                source_type: "git".to_string(),
                source_ref: Some(parsed.original_url.clone()),
                source_ref_resolved: Some(parsed.clone_url.clone()),
                source_subpath: git_fetcher::relative_subpath(&temp_dir, &skill_dir),
                source_branch: parsed.branch.clone(),
                source_revision: Some(revision.clone()),
                remote_revision: Some(revision),
                update_status: "up_to_date".to_string(),
            };
            let skill_name = result.name.clone();
            let skill_id = store_installed_skill_unlocked(&store, &result, &metadata, None)?;
            Ok((skill_id, skill_name))
        })();

        git_fetcher::cleanup_temp(&temp_dir);
        install_result
    })();

    log_install_outcome(&store, "git", outcome.as_ref());
    outcome?;

    emit_progress("done");
    Ok(())
}

#[tauri::command]
pub async fn install_from_skillssh(
    source: String,
    skill_id: String,
    ctx: State<'_, HostCtx>,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || install_from_skillssh_core(&ctx, source, skill_id))
        .await?
}

pub fn install_from_skillssh_core(
    ctx: &HostCtx,
    source: String,
    skill_id: String,
) -> Result<(), AppError> {
    let store = ctx.store.clone();
    let proxy_url = store.proxy_url();
    let registry = ctx.cancel.clone();
    let cancel_key_owned = format!("{}/{}", source, skill_id);
    let cancel = registry.register(&cancel_key_owned);
    let _cancel_guard = CancelRegistrationGuard::new(registry.clone(), cancel_key_owned);

    let skill_key = format!("{}/{}", source, skill_id);
    let emit_progress = |phase: &str| {
        ctx.events.emit(
            "install-progress",
            serde_json::json!({
                "skill_id": skill_key,
                "phase": phase,
            }),
        );
    };

    let outcome = (|| -> Result<(String, String), AppError> {
        emit_progress("cloning");
        let repo_url = format!("https://github.com/{}.git", source);
        let events_for_progress = ctx.events.clone();
        let skill_key_for_progress = skill_key.clone();
        let progress_cb: git_fetcher::ProgressCallback = Box::new(move |msg: &str| {
            events_for_progress.emit(
                "install-progress",
                serde_json::json!({
                    "skill_id": skill_key_for_progress,
                    "phase": "cloning",
                    "detail": msg,
                }),
            );
        });
        let temp_dir = git_fetcher::clone_repo_ref_with_progress(
            &repo_url,
            None,
            Some(&cancel),
            proxy_url.as_deref(),
            Some(progress_cb),
        )
        .map_err(AppError::classify_git_error)?;

        emit_progress("installing");
        let install_result = (|| -> Result<(String, String), AppError> {
            let _lock =
                RepoLock::acquire_foreground("install skillssh skill").map_err(AppError::db)?;
            let skill_dir = resolve_skill_dir(&temp_dir, None, Some(&skill_id))?;
            let revision = git_fetcher::get_head_revision(&temp_dir).map_err(AppError::git)?;
            let source_ref = format!("{}/{}", source, skill_id);
            let (install_name, destination) =
                resolve_skillssh_install_target(&store, &source_ref, &skill_id)?;
            let result = installer::install_skill_dir_to_destination(
                &skill_dir,
                &install_name,
                &destination,
            )
            .map_err(AppError::io)?;
            let metadata = InstallSourceMetadata {
                source_type: "skillssh".to_string(),
                source_ref: Some(source_ref),
                source_ref_resolved: Some(repo_url.clone()),
                source_subpath: git_fetcher::relative_subpath(&temp_dir, &skill_dir),
                source_branch: None,
                source_revision: Some(revision.clone()),
                remote_revision: Some(revision),
                update_status: "up_to_date".to_string(),
            };
            let skill_name = result.name.clone();
            let new_id = store_installed_skill_unlocked(&store, &result, &metadata, None)?;
            Ok((new_id, skill_name))
        })();

        git_fetcher::cleanup_temp(&temp_dir);
        install_result
    })();

    log_install_outcome(&store, "skillssh", outcome.as_ref());
    outcome?;

    emit_progress("done");
    Ok(())
}

/// Clone a git repo and return a preview list of skills found, without installing.
/// The caller must follow up with `confirm_git_install` using the returned `temp_dir`.
#[tauri::command]
pub async fn preview_git_install(
    repo_url: String,
    ctx: State<'_, HostCtx>,
) -> Result<GitPreviewResult, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || preview_git_install_core(&ctx, repo_url)).await?
}

pub fn preview_git_install_core(
    ctx: &HostCtx,
    repo_url: String,
) -> Result<GitPreviewResult, AppError> {
    let store = ctx.store.clone();
    let proxy_url = store.get_setting("proxy_url").ok().flatten();
    let registry = ctx.cancel.clone();
    let cancel_key = repo_url.clone();
    let cancel = registry.register(&cancel_key);
    let _cancel_guard = CancelRegistrationGuard::new(registry.clone(), cancel_key);

    ctx.events.emit(
        "install-progress",
        serde_json::json!({
            "skill_id": repo_url,
            "phase": "cloning",
        }),
    );

    let parsed = git_fetcher::parse_git_source_resolved(&repo_url, proxy_url.as_deref());
    let events_for_progress = ctx.events.clone();
    let url_for_progress = repo_url.clone();
    let progress_cb: git_fetcher::ProgressCallback = Box::new(move |msg: &str| {
        events_for_progress.emit(
            "install-progress",
            serde_json::json!({
                "skill_id": url_for_progress,
                "phase": "cloning",
                "detail": msg,
            }),
        );
    });
    let temp_dir = git_fetcher::clone_repo_ref_scoped(
        &parsed.clone_url,
        parsed.branch.as_deref(),
        parsed.subpath.as_deref(),
        Some(&cancel),
        proxy_url.as_deref(),
        Some(progress_cb),
    )
    .map_err(AppError::classify_git_error)?;

    let build_preview = || -> Result<GitPreviewResult, AppError> {
        let skill_dir = resolve_skill_dir(&temp_dir, parsed.subpath.as_deref(), None)?;
        let dirs = collect_git_skill_dirs(&skill_dir);

        let skills: Vec<GitSkillPreview> = dirs
            .iter()
            .map(|dir| {
                let meta = skill_metadata::parse_skill_md(dir);
                let rel_path = skill_rel_key(&skill_dir, dir);
                let basename = dir
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| rel_path.clone());
                let name = meta
                    .name
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| basename.clone());
                GitSkillPreview {
                    rel_path,
                    name,
                    description: meta.description,
                }
            })
            .collect();

        Ok(GitPreviewResult {
            temp_dir: temp_dir.to_string_lossy().to_string(),
            skills,
        })
    };

    build_preview().inspect_err(|_e| {
        git_fetcher::cleanup_temp(&temp_dir);
    })
}

/// Install selected skills from a previously cloned temp directory.
#[tauri::command]
pub async fn confirm_git_install(
    repo_url: String,
    temp_dir: String,
    items: Vec<SkillInstallItem>,
    ctx: State<'_, HostCtx>,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        confirm_git_install_core(&ctx, repo_url, temp_dir, items)
    })
    .await?
}

pub fn confirm_git_install_core(
    ctx: &HostCtx,
    repo_url: String,
    temp_dir: String,
    items: Vec<SkillInstallItem>,
) -> Result<(), AppError> {
    let store = ctx.store.clone();
    let proxy_url = store.proxy_url();
    let temp_path = validate_clone_temp_path(&temp_dir)?;

    let result: Result<(), AppError> = (|| {
        if items.is_empty() {
            return Ok(());
        }

        let parsed = git_fetcher::parse_git_source_resolved(&repo_url, proxy_url.as_deref());
        let skill_dir = resolve_skill_dir(&temp_path, parsed.subpath.as_deref(), None)?;
        let all_dirs = collect_git_skill_dirs(&skill_dir);
        let revision = git_fetcher::get_head_revision(&temp_path).map_err(AppError::git)?;
        let _lock = RepoLock::acquire_foreground("confirm git install").map_err(AppError::db)?;

        for dir in &all_dirs {
            let rel_key = skill_rel_key(&skill_dir, dir);
            let item = match items.iter().find(|i| i.rel_path == rel_key) {
                Some(i) => i,
                None => continue,
            };
            let custom_name = item.name.trim();
            let install_name = if custom_name.is_empty() {
                None
            } else {
                Some(custom_name)
            };
            let result =
                installer::install_from_git_dir(dir, install_name).map_err(AppError::io)?;
            let subpath = git_fetcher::relative_subpath(&temp_path, dir);
            let metadata = InstallSourceMetadata {
                source_type: "git".to_string(),
                source_ref: Some(repo_url.clone()),
                source_ref_resolved: Some(parsed.clone_url.clone()),
                source_subpath: subpath,
                source_branch: parsed.branch.clone(),
                source_revision: Some(revision.clone()),
                remote_revision: Some(revision.clone()),
                update_status: "up_to_date".to_string(),
            };
            store_installed_skill_unlocked(&store, &result, &metadata, None)?;
        }
        Ok(())
    })();

    // Always clean up temp directory, regardless of success or failure.
    git_fetcher::cleanup_temp(&temp_path);
    result
}

/// Clean up temp directory from a cancelled preview session.
#[tauri::command]
pub async fn cancel_git_preview(temp_dir: String) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || cancel_git_preview_core(temp_dir)).await?
}

pub fn cancel_git_preview_core(temp_dir: String) -> Result<(), AppError> {
    if let Ok(temp_path) = validate_clone_temp_path(&temp_dir) {
        git_fetcher::cleanup_temp(&temp_path);
    }
    Ok(())
}

/// Return the list of individual skill directories to install from a resolved repo dir.
/// If `skill_dir` is itself a valid skill, returns `[skill_dir]`.
/// Otherwise recursively walks for skill dirs (e.g. `category/<skill>` layouts).
/// Returns an empty Vec when nothing is found — callers must handle that.
pub fn collect_git_skill_dirs(skill_dir: &Path) -> Vec<PathBuf> {
    if is_valid_skill_dir(skill_dir) {
        return vec![skill_dir.to_path_buf()];
    }
    let mut dirs = scanner::collect_skill_dirs(skill_dir);
    dirs.sort();
    dirs
}

/// Stable identifier for a discovered skill within a preview/confirm cycle.
/// Uses forward slashes regardless of platform so the frontend sees consistent keys.
pub fn skill_rel_key(skill_dir: &Path, dir: &Path) -> String {
    let rel = dir.strip_prefix(skill_dir).unwrap_or(dir);
    if rel.as_os_str().is_empty() {
        dir.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    } else {
        rel.to_string_lossy().replace('\\', "/")
    }
}

/// Validate and canonicalize a temp directory path used by the git preview/install flow.
/// Returns the canonicalized path if it passes security checks.
pub fn validate_clone_temp_path(temp_dir: &str) -> Result<PathBuf, AppError> {
    let raw_path = PathBuf::from(temp_dir);
    if !raw_path.exists() {
        return Err(AppError::invalid_input(
            "Clone session expired, please try again",
        ));
    }
    // Canonicalize to resolve symlinks and `..` segments before checking prefix.
    let temp_path = raw_path
        .canonicalize()
        .map_err(|_| AppError::invalid_input("Invalid temp directory"))?;

    // Preview confirmation must operate on an isolated checkout, never the repo cache.
    let expected_prefix = std::env::temp_dir()
        .canonicalize()
        .unwrap_or_else(|_| std::env::temp_dir());
    if temp_path.starts_with(&expected_prefix) {
        let dir_name_str = temp_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if dir_name_str.starts_with(git_fetcher::CLONE_TEMP_PREFIX) {
            return Ok(temp_path);
        }
    }

    Err(AppError::invalid_input("Invalid temp directory"))
}

#[tauri::command]
pub async fn cancel_install(key: String, ctx: State<'_, HostCtx>) -> Result<bool, AppError> {
    cancel_install_core(&ctx, key)
}

pub fn cancel_install_core(ctx: &HostCtx, key: String) -> Result<bool, AppError> {
    Ok(ctx.cancel.cancel(&key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_support::write_skill;
    use std::fs;
    use tempfile::tempdir;

    fn write_skill_at(root: &Path, rel: &str) -> PathBuf {
        let dir = root.join(rel);
        fs::create_dir_all(&dir).unwrap();
        let basename = dir.file_name().unwrap().to_string_lossy().to_string();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {basename}\n---\n"),
        )
        .unwrap();
        dir
    }

    #[test]
    fn collect_git_skill_dirs_finds_nested_categories() {
        // Mirrors mattpocock/skills layout: skills/<category>/<skill>/SKILL.md.
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        write_skill_at(root, "in-progress/foo");
        write_skill_at(root, "in-progress/bar");
        write_skill_at(root, "stable/baz");

        let dirs = collect_git_skill_dirs(root);
        let keys: Vec<String> = dirs.iter().map(|d| skill_rel_key(root, d)).collect();
        assert_eq!(dirs.len(), 3, "should find skills two levels deep");
        assert!(keys.contains(&"in-progress/foo".to_string()));
        assert!(keys.contains(&"in-progress/bar".to_string()));
        assert!(keys.contains(&"stable/baz".to_string()));
    }

    #[test]
    fn collect_git_skill_dirs_returns_self_when_root_is_skill() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("SKILL.md"), "---\nname: x\n---").unwrap();
        let dirs = collect_git_skill_dirs(root);
        assert_eq!(dirs, vec![root.to_path_buf()]);
    }

    #[test]
    fn collect_git_skill_dirs_returns_empty_when_no_skills() {
        // Previously this case returned [skill_dir] as a bogus fallback,
        // which then surfaced a non-skill category dir as installable.
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("empty-category")).unwrap();
        let dirs = collect_git_skill_dirs(root);
        assert!(dirs.is_empty(), "no fallback to scan root when empty");
    }

    #[test]
    fn skill_rel_key_uses_forward_slashes() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("repo");
        let nested = root.join("a").join("b");
        let key = skill_rel_key(&root, &nested);
        assert_eq!(key, "a/b");
    }

    #[test]
    fn skill_rel_key_disambiguates_same_basename_across_categories() {
        // Two skills with the same dir basename in different categories must
        // produce distinct rel keys — that's the point of using rel paths.
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let a_foo = write_skill_at(root, "category-a/foo");
        let b_foo = write_skill_at(root, "category-b/foo");

        let dirs = collect_git_skill_dirs(root);
        assert_eq!(dirs.len(), 2);

        let k_a = skill_rel_key(root, &a_foo);
        let k_b = skill_rel_key(root, &b_foo);
        assert_ne!(k_a, k_b);
        assert_eq!(k_a, "category-a/foo");
        assert_eq!(k_b, "category-b/foo");
    }

    #[test]
    fn resolve_still_returns_a_container_for_enumeration() {
        // preview/confirm install walk a container to list the skills inside
        // it, so an existing non-skill directory must keep resolving.
        let tmp = tempdir().unwrap();
        write_skill(&tmp.path().join("skills").join("pdf"), "pdf");
        write_skill(&tmp.path().join("skills").join("docx"), "docx");

        let resolved = resolve_skill_dir(tmp.path(), Some("skills"), None).unwrap();
        assert_eq!(resolved, tmp.path().join("skills"));
        // What preview/confirm actually do with that container.
        assert_eq!(collect_git_skill_dirs(&resolved).len(), 2);
    }
}
