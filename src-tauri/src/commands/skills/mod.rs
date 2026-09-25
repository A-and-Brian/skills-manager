//! Tauri commands for the managed skills library, grouped by concern.

mod library;
mod query;
mod types;

pub use library::*;
pub use query::*;
pub use types::*;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tauri::State;

use crate::core::{
    audit_log::AuditDraft,
    central_repo,
    error::AppError,
    git_fetcher,
    host::HostCtx,
    installer,
    managed_skill::{managed_skill_by_id, ManagedSkillDto},
    repo_lock::RepoLock,
    scanner,
    skill_install::{
        resolve_skill_dir, resolve_skillssh_install_target, store_installed_skill_unlocked,
        InstallSourceMetadata,
    },
    skill_metadata::{self, is_valid_skill_dir},
    skill_source::git_source_from_skill,
    skill_store::SkillStore,
    skill_update::{
        pending_removals_for, reimport_local_skill_internal, removal_approval_token,
        remove_path_if_exists, resync_copy_targets, staged_path_for, swap_skill_directory,
        update_git_skill_internal, PendingRemoval, ReimportSkillResult, StagedPathGuard,
        UpdateSkillResult,
    },
    skill_update_check::{
        check_skill_update_internal_with_remote, prefetch_skill_remote, resolve_remotes_concurrent,
        should_skip_update_check, PrefetchedRemote, RemoteKey,
    },
    sync_metadata,
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

fn log_update_outcome(
    store: &SkillStore,
    skill_id: &str,
    source_label: &str,
    outcome: Result<&UpdateSkillResult, &AppError>,
) {
    let mut draft = AuditDraft::new("update").detail(source_label);
    match outcome {
        Ok(result) if !result.pending_removals.is_empty() => {
            // Held back, not applied. Recording it as a successful "unchanged"
            // would make the audit trail disagree with what actually happened.
            draft = draft
                .skill(result.skill.id.clone(), result.skill.name.clone())
                .detail(format!(
                    "{source_label}; held back — would remove {} path(s)",
                    result.pending_removals.len()
                ))
                .ok();
        }
        Ok(result) => {
            draft = draft
                .skill(result.skill.id.clone(), result.skill.name.clone())
                .detail(if result.content_changed {
                    format!("{source_label}; content changed")
                } else {
                    format!("{source_label}; unchanged")
                })
                .ok();
        }
        Err(e) => {
            let name = store
                .get_skill_by_id(skill_id)
                .ok()
                .flatten()
                .map(|s| s.name)
                .unwrap_or_default();
            draft = draft.skill(skill_id.to_string(), name).fail(e.to_string());
        }
    }
    store.log_audit(draft);
}

fn log_reimport_outcome(
    store: &SkillStore,
    skill_id: &str,
    outcome: Result<&ReimportSkillResult, &AppError>,
) {
    let mut draft = AuditDraft::new("update").detail("local");
    match outcome {
        Ok(result) if !result.pending_removals.is_empty() => {
            draft = draft
                .skill(result.skill.id.clone(), result.skill.name.clone())
                .detail(format!(
                    "local; held back — would remove {} path(s)",
                    result.pending_removals.len()
                ))
                .ok();
        }
        Ok(result) => {
            draft = draft
                .skill(result.skill.id.clone(), result.skill.name.clone())
                .ok();
        }
        Err(e) => {
            let name = store
                .get_skill_by_id(skill_id)
                .ok()
                .flatten()
                .map(|s| s.name)
                .unwrap_or_default();
            draft = draft.skill(skill_id.to_string(), name).fail(e.to_string());
        }
    }
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

#[tauri::command]
pub async fn check_skill_update(
    skill_id: String,
    force: Option<bool>,
    ctx: State<'_, HostCtx>,
) -> Result<ManagedSkillDto, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || check_skill_update_core(&ctx, skill_id, force))
        .await?
}

pub fn check_skill_update_core(
    ctx: &HostCtx,
    skill_id: String,
    force: Option<bool>,
) -> Result<ManagedSkillDto, AppError> {
    let store = ctx.store.clone();
    let proxy_url = store.proxy_url();
    let force = force.unwrap_or(false);
    // Resolve first, take the lock second. Holding it across `ls-remote`
    // meant one check of a slow remote could occupy the repository for the
    // whole round-trip and fail every concurrent operation (#315).
    let prefetched = prefetch_skill_remote(&store, &skill_id, force, proxy_url.as_deref());
    let _lock = RepoLock::acquire_foreground("check skill update").map_err(AppError::db)?;
    check_skill_update_internal_with_remote(&store, &skill_id, force, prefetched)
}

#[tauri::command]
pub async fn check_all_skill_updates(
    force: Option<bool>,
    ctx: State<'_, HostCtx>,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || check_all_skill_updates_core(&ctx, force)).await?
}

pub fn check_all_skill_updates_core(ctx: &HostCtx, force: Option<bool>) -> Result<(), AppError> {
    let store = ctx.store.clone();
    let proxy_url = store.proxy_url();
    let force_check = force.unwrap_or(false);
    let skills = store.get_all_skills().map_err(AppError::db)?;

    // ── Phase A: resolve every distinct remote once, concurrently ──
    // Collect the git-backed skills that still need a network check keyed by
    // (clone_url, branch). Skills installed from subdirectories of the same
    // monorepo collapse to a single `ls-remote`, and each remote is queried
    // off the central-repo lock so a slow remote (e.g. vercel/ai's ref
    // advertisement runs ~30s) never starves a concurrent check into a 20s
    // lock-timeout "busy" failure — the reason "检查全部" both crawled and
    // popped failures.
    let mut remotes: HashSet<RemoteKey> = HashSet::new();
    for skill in &skills {
        if !matches!(skill.source_type.as_str(), "git" | "skillssh") {
            continue;
        }
        match should_skip_update_check(&store, skill, force_check) {
            Ok(true) => continue,
            Ok(false) => {}
            // A transient skip-decision error (e.g. a settings read) must not
            // abort the whole batch: fall through so Phase B still checks this
            // skill and collects any real failure per-skill, as before.
            Err(err) => log::warn!(
                "check all: skip-decision for {} failed, checking anyway: {}",
                skill.id,
                err.message
            ),
        }
        if let Ok(source) = git_source_from_skill(skill) {
            remotes.insert(RemoteKey::from(source));
        }
    }
    let remote_revisions = if remotes.is_empty() {
        HashMap::new()
    } else {
        resolve_remotes_concurrent(remotes.into_iter().collect(), proxy_url.clone())
    };

    // ── Phase B: apply the resolved revisions + local-source checks ──
    // Phase A already did every network read, so this loop only computes and
    // writes each skill's status columns. Re-take the central-repo lock per
    // skill around that write — the same guard the pre-concurrent code used so
    // a concurrent manual install/update can't race the `update_status` write
    // — but now the lock is never held across a slow `ls-remote`, because the
    // network happened off the lock in Phase A, and the apply step itself
    // can't reach the network. A skill whose source moved (or whose TTL
    // expired) between the two phases has no usable prefetch and is simply
    // left for the next round. Lock contention is still reported per skill
    // so the caller knows the check didn't complete for it.
    let mut failed = Vec::new();
    for skill in &skills {
        let prefetched = if matches!(skill.source_type.as_str(), "git" | "skillssh") {
            git_source_from_skill(skill).ok().and_then(|source| {
                let key = RemoteKey::from(source);
                remote_revisions
                    .get(&key)
                    .cloned()
                    .map(|result| PrefetchedRemote { key, result })
            })
        } else {
            None
        };
        let _lock = match RepoLock::acquire("check skill update") {
            Ok(lock) => lock,
            Err(err) => {
                failed.push(format!("{}: {}", skill.id, err));
                continue;
            }
        };
        if let Err(err) =
            check_skill_update_internal_with_remote(&store, &skill.id, force_check, prefetched)
        {
            // Surface the real per-skill reason so a batch that "just fails"
            // is diagnosable from the logs, not only the aggregated toast.
            log::warn!("check all: {} failed: {}", skill.id, err.message);
            failed.push(format!("{}: {}", skill.id, err));
        }
    }

    if failed.is_empty() {
        Ok(())
    } else {
        Err(AppError::internal(format!(
            "Failed to check {} skill(s): {}",
            failed.len(),
            failed.join("; ")
        )))
    }
}

/// Update one skill.
///
/// `approved_removals` carries back `removal_approval` from a call that
/// declined. The first call from the UI passes `None`; if it comes back with
/// `pending_removals`, the user is shown exactly what would disappear and only
/// then is it called again with that token.
#[tauri::command]
pub async fn update_skill(
    skill_id: String,
    approved_removals: Option<String>,
    ctx: State<'_, HostCtx>,
) -> Result<UpdateSkillResult, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        update_skill_core(&ctx, skill_id, approved_removals)
    })
    .await?
}

pub fn update_skill_core(
    ctx: &HostCtx,
    skill_id: String,
    approved_removals: Option<String>,
) -> Result<UpdateSkillResult, AppError> {
    let store = ctx.store.clone();
    let proxy_url = store.proxy_url();
    let registry = ctx.cancel.clone();
    let cancel_key = format!("update:{}", skill_id);
    let cancel = registry.register(&cancel_key);
    let _cancel_guard = CancelRegistrationGuard::new(registry.clone(), cancel_key);

    let outcome = update_git_skill_internal(
        &store,
        &skill_id,
        proxy_url.as_deref(),
        Some(&cancel),
        approved_removals.as_deref(),
    );
    log_update_outcome(&store, &skill_id, "git", outcome.as_ref());
    outcome
}

#[tauri::command]
pub async fn reimport_local_skill(
    skill_id: String,
    approved_removals: Option<String>,
    ctx: State<'_, HostCtx>,
) -> Result<ReimportSkillResult, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        reimport_local_skill_core(&ctx, skill_id, approved_removals)
    })
    .await?
}

pub fn reimport_local_skill_core(
    ctx: &HostCtx,
    skill_id: String,
    approved_removals: Option<String>,
) -> Result<ReimportSkillResult, AppError> {
    let store = ctx.store.clone();
    let outcome = reimport_local_skill_internal(&store, &skill_id, approved_removals.as_deref());
    log_reimport_outcome(&store, &skill_id, outcome.as_ref());
    outcome
}

#[tauri::command]
pub async fn batch_update_skills(
    skill_ids: Vec<String>,
    ctx: State<'_, HostCtx>,
) -> Result<BatchUpdateSkillsResult, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || batch_update_skills_core(&ctx, skill_ids)).await?
}

pub fn batch_update_skills_core(
    ctx: &HostCtx,
    skill_ids: Vec<String>,
) -> Result<BatchUpdateSkillsResult, AppError> {
    let store = ctx.store.clone();
    let proxy_url = store.proxy_url();
    let mut refreshed = 0usize;
    let mut unchanged = 0usize;
    let mut failed = Vec::new();
    let mut held_back = Vec::new();

    for skill_id in skill_ids {
        let skill = match store.get_skill_by_id(&skill_id).map_err(AppError::db)? {
            Some(skill) => skill,
            None => {
                failed.push(format!("{skill_id}: Skill not found"));
                continue;
            }
        };

        match skill.source_type.as_str() {
            "git" | "skillssh" => {
                let outcome =
                    update_git_skill_internal(&store, &skill_id, proxy_url.as_deref(), None, None);
                log_update_outcome(&store, &skill_id, "git", outcome.as_ref());
                match outcome {
                    Ok(result) if !result.pending_removals.is_empty() => {
                        // Held back rather than applied: it would have taken
                        // away files the new version does not have, and a
                        // batch has nobody to ask.
                        held_back.push(skill.name.clone());
                    }
                    Ok(result) => {
                        if result.content_changed {
                            refreshed += 1;
                        } else {
                            unchanged += 1;
                        }
                    }
                    Err(err) => failed.push(format!("{}: {}", skill.name, err.message)),
                }
            }
            "local" | "import" => {
                let outcome = reimport_local_skill_internal(&store, &skill_id, None);
                log_reimport_outcome(&store, &skill_id, outcome.as_ref());
                match outcome {
                    Ok(result) if !result.pending_removals.is_empty() => {
                        held_back.push(skill.name.clone());
                    }
                    Ok(_) => refreshed += 1,
                    Err(err) => failed.push(format!("{}: {}", skill.name, err.message)),
                }
            }
            _ => failed.push(format!("{}: Source type cannot be refreshed", skill.name)),
        }
    }

    Ok(BatchUpdateSkillsResult {
        refreshed,
        unchanged,
        failed,
        held_back,
    })
}

#[tauri::command]
/// Re-point a local skill at a different source directory.
///
/// `approved_removals` behaves as on the update paths: choosing a new source is
/// not a statement about discarding what the library has accumulated, so a
/// replacement that would take files away stops and reports them first.
pub async fn relink_local_skill_source(
    skill_id: String,
    source_path: String,
    approved_removals: Option<String>,
    ctx: State<'_, HostCtx>,
) -> Result<ReimportSkillResult, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        relink_local_skill_source_core(&ctx, skill_id, source_path, approved_removals)
    })
    .await?
}

pub fn relink_local_skill_source_core(
    ctx: &HostCtx,
    skill_id: String,
    source_path: String,
    approved_removals: Option<String>,
) -> Result<ReimportSkillResult, AppError> {
    let store = ctx.store.clone();
    let skill = store
        .get_skill_by_id(&skill_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Skill not found"))?;

    if !matches!(skill.source_type.as_str(), "local" | "import") {
        return Err(AppError::invalid_input(
            "Only local skills can relink source paths",
        ));
    }

    let path = PathBuf::from(&source_path);
    if !path.exists() {
        return Err(AppError::not_found("Selected source path does not exist"));
    }
    if !is_valid_skill_dir(&path) {
        return Err(AppError::invalid_input(
            "Selected source path is not a valid skill directory",
        ));
    }

    store
        .update_skill_update_status(&skill_id, "updating")
        .map_err(AppError::db)?;

    let result = (|| -> Result<(Vec<PendingRemoval>, Option<String>), AppError> {
        let _lock = RepoLock::acquire_foreground("relink local skill").map_err(AppError::db)?;
        let staged_path = staged_path_for(&skill.central_path);
        let install_result =
            installer::install_from_local_to_destination(&path, Some(&skill.name), &staged_path)
                .inspect_err(|_| {
                    let _ = remove_path_if_exists(&staged_path);
                })
                .map_err(AppError::io)?;
        let staged_guard = StagedPathGuard::new(&staged_path, true);

        // Picking a new source says which source to follow. It does not say
        // to discard whatever has accumulated in the library since — same
        // replacement, same guard.
        let pending = pending_removals_for(&store, &skill, Some(&staged_path))?;
        let approval = removal_approval_token(&source_path, &pending);
        if !pending.is_empty() && approved_removals.as_deref() != Some(approval.as_str()) {
            // Put back exactly what was there. Hardcoding a status loses
            // `source_missing` — the only state relink is reachable from —
            // so declining would hide the Relink and Detach buttons on the
            // next refresh, and `check_state` would also clear the recorded
            // error and check time that nothing here has re-established.
            store
                .update_skill_update_status(&skill.id, &skill.update_status)
                .map_err(AppError::db)?;
            return Ok((pending, Some(approval)));
        }

        swap_skill_directory(&staged_path, Path::new(&skill.central_path))?;
        staged_guard.release();
        store
            .update_skill_after_reinstall(
                &skill.id,
                &skill.name,
                install_result.description.as_deref(),
                &skill.source_type,
                Some(&source_path),
                None,
                None,
                None,
                None,
                None,
                Some(&install_result.content_hash),
                "local_only",
            )
            .map_err(AppError::db)?;
        resync_copy_targets(&store, &skill.id)?;
        sync_metadata::write_all_from_db_unlocked(&store).map_err(AppError::db)?;
        Ok((Vec::new(), None))
    })();

    match result {
        Ok((pending_removals, removal_approval)) => Ok(ReimportSkillResult {
            skill: managed_skill_by_id(&store, &skill_id)?,
            pending_removals,
            removal_approval,
        }),
        Err(e) => {
            let _ = store.update_skill_check_state(&skill_id, None, "error", Some(&e.message));
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn detach_local_skill_source(
    skill_id: String,
    ctx: State<'_, HostCtx>,
) -> Result<ManagedSkillDto, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || detach_local_skill_source_core(&ctx, skill_id))
        .await?
}

pub fn detach_local_skill_source_core(
    ctx: &HostCtx,
    skill_id: String,
) -> Result<ManagedSkillDto, AppError> {
    let store = ctx.store.clone();
    let skill = store
        .get_skill_by_id(&skill_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Skill not found"))?;

    if !matches!(skill.source_type.as_str(), "local" | "import") {
        return Err(AppError::invalid_input(
            "Only local skills can detach source paths",
        ));
    }

    {
        let _lock = RepoLock::acquire_foreground("detach local skill").map_err(AppError::db)?;
        store
            .update_skill_after_reinstall(
                &skill.id,
                &skill.name,
                skill.description.as_deref(),
                &skill.source_type,
                None,
                None,
                None,
                None,
                None,
                None,
                skill.content_hash.as_deref(),
                "local_only",
            )
            .map_err(AppError::db)?;
        sync_metadata::write_all_from_db_unlocked(&store).map_err(AppError::db)?;
    }

    managed_skill_by_id(&store, &skill_id)
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

#[tauri::command]
pub async fn batch_import_folder(
    folder_path: String,
    ctx: State<'_, HostCtx>,
) -> Result<BatchImportResult, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || batch_import_folder_core(&ctx, folder_path))
        .await?
}

pub fn batch_import_folder_core(
    ctx: &HostCtx,
    folder_path: String,
) -> Result<BatchImportResult, AppError> {
    let store = ctx.store.clone();

    let root = PathBuf::from(&folder_path);
    if !root.is_dir() {
        return Err(AppError::invalid_input("Selected path is not a directory"));
    }

    // Collect valid skill subdirectories (depth=1)
    let mut skill_dirs: Vec<PathBuf> = Vec::new();
    let entries = std::fs::read_dir(&root)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if is_valid_skill_dir(&path) {
            skill_dirs.push(path);
        }
    }

    if skill_dirs.is_empty() {
        return Ok(BatchImportResult {
            imported: 0,
            skipped: 0,
            errors: vec![],
        });
    }

    let total = skill_dirs.len();
    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut errors = Vec::new();

    for (i, dir) in skill_dirs.iter().enumerate() {
        let name = skill_metadata::infer_skill_name(dir);

        ctx.events.emit(
            "batch-import-progress",
            serde_json::json!({
                "current": i + 1,
                "total": total,
                "name": &name,
            }),
        );

        // Check if already imported by prospective central path
        let prospective_central = central_repo::skills_dir().join(&name);
        let central_str = prospective_central.to_string_lossy().to_string();
        if let Ok(Some(_)) = store.get_skill_by_central_path(&central_str) {
            skipped += 1;
            continue;
        }

        let install_result = (|| -> Result<String, AppError> {
            let _lock = RepoLock::acquire_foreground("batch import skill").map_err(AppError::db)?;
            let result = installer::install_from_local(dir, Some(&name)).map_err(AppError::io)?;
            let metadata = InstallSourceMetadata {
                source_type: "local".to_string(),
                source_ref: Some(dir.to_string_lossy().to_string()),
                source_ref_resolved: None,
                source_subpath: None,
                source_branch: None,
                source_revision: None,
                remote_revision: None,
                update_status: "local_only".to_string(),
            };
            store_installed_skill_unlocked(&store, &result, &metadata, None)
        })();

        match install_result {
            Ok(_) => imported += 1,
            Err(e) => errors.push(format!("{}: {}", name, e)),
        }
    }

    Ok(BatchImportResult {
        imported,
        skipped,
        errors,
    })
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
