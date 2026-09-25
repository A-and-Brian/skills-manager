//! Tauri commands for the managed skills library, grouped by concern.

mod install;
mod library;
mod query;
mod types;
mod update;

pub use install::*;
pub use library::*;
pub use query::*;
pub use types::*;
pub use update::*;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tauri::State;

use crate::core::{
    central_repo,
    error::AppError,
    host::HostCtx,
    installer,
    managed_skill::{managed_skill_by_id, ManagedSkillDto},
    repo_lock::RepoLock,
    skill_install::{store_installed_skill_unlocked, InstallSourceMetadata},
    skill_metadata::{self, is_valid_skill_dir},
    skill_source::git_source_from_skill,
    skill_update::{
        pending_removals_for, removal_approval_token, remove_path_if_exists, resync_copy_targets,
        staged_path_for, swap_skill_directory, PendingRemoval, ReimportSkillResult,
        StagedPathGuard,
    },
    skill_update_check::{
        check_skill_update_internal_with_remote, prefetch_skill_remote, resolve_remotes_concurrent,
        should_skip_update_check, PrefetchedRemote, RemoteKey,
    },
    sync_metadata,
};

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
