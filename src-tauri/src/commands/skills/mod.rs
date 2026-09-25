//! Tauri commands for the managed skills library, grouped by concern.

mod install;
mod library;
mod query;
mod source;
mod types;
mod update;
mod update_check;

pub use install::*;
pub use library::*;
pub use query::*;
pub use source::*;
pub use types::*;
pub use update::*;
pub use update_check::*;

use std::path::PathBuf;
use tauri::State;

use crate::core::{
    central_repo,
    error::AppError,
    host::HostCtx,
    installer,
    repo_lock::RepoLock,
    skill_install::{store_installed_skill_unlocked, InstallSourceMetadata},
    skill_metadata::{self, is_valid_skill_dir},
};

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
