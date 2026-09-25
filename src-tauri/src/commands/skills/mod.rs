//! Tauri commands for the managed skills library, grouped by concern.

mod install;
mod library;
mod query;
mod types;
mod update;
mod update_check;

pub use install::*;
pub use library::*;
pub use query::*;
pub use types::*;
pub use update::*;
pub use update_check::*;

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
    skill_update::{
        pending_removals_for, removal_approval_token, remove_path_if_exists, resync_copy_targets,
        staged_path_for, swap_skill_directory, PendingRemoval, ReimportSkillResult,
        StagedPathGuard,
    },
    sync_metadata,
};

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
