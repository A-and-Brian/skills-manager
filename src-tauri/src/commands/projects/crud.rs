//! Listing, adding, removing, reordering and scanning for projects.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use tauri::State;

use super::agents_model::agent_skill_configs;
use super::types::{project_to_dto, ProjectDto};
use crate::core::project_scanner::VENDORED_SKILLS_DIR;
use crate::core::project_skill_match::slugify_skill_dir_name;
use crate::core::skill_store::{ProjectRecord, SkillStore};
use crate::core::timing::should_log_first_or_slow;
use crate::core::{error::AppError, host::HostCtx, project_scanner};

fn ensure_distinct_linked_workspace_roots(
    skills_root: &Path,
    disabled_root: &Path,
) -> Result<(), AppError> {
    let skills_canonical = std::fs::canonicalize(skills_root)?;
    let disabled_canonical = std::fs::canonicalize(disabled_root)?;

    if skills_canonical == disabled_canonical
        || skills_canonical.starts_with(&disabled_canonical)
        || disabled_canonical.starts_with(&skills_canonical)
    {
        return Err(AppError::invalid_input(
            "Skills directory and disabled skills directory must not overlap",
        ));
    }

    Ok(())
}

static GET_PROJECTS_FIRST_CALL: AtomicBool = AtomicBool::new(true);

#[tauri::command]
pub async fn get_projects(ctx: State<'_, HostCtx>) -> Result<Vec<ProjectDto>, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || get_projects_core(&ctx)).await?
}

pub fn get_projects_core(ctx: &HostCtx) -> Result<Vec<ProjectDto>, AppError> {
    let store = ctx.store.clone();
    let start = Instant::now();
    let records = store.get_all_projects().map_err(AppError::db)?;
    let all_managed = store.get_all_skills().map_err(AppError::db)?;
    let configs = agent_skill_configs(&store);
    let count = records.len();
    let dtos: Vec<ProjectDto> = records
        .iter()
        .map(|r| project_to_dto(r, &all_managed, &configs))
        .collect();
    let elapsed_ms = start.elapsed().as_millis();
    if should_log_first_or_slow(&GET_PROJECTS_FIRST_CALL, elapsed_ms, 100) {
        log::info!("get_projects: {count} projects in {elapsed_ms} ms");
    }
    Ok(dtos)
}

/// The mode a new project gets: the caller's choice, else the user's
/// `default_project_deploy_mode` setting. An unset or unknown stored value
/// means link, so the setting can never make adding a project fail.
fn new_project_deploy_mode(
    store: &SkillStore,
    requested: Option<String>,
) -> Result<String, AppError> {
    if let Some(mode) = requested {
        return Ok(mode);
    }
    let stored = store
        .get_setting("default_project_deploy_mode")
        .map_err(AppError::db)?;
    Ok(match stored.as_deref() {
        Some("copy") => "copy",
        _ => "link",
    }
    .to_string())
}

#[tauri::command]
pub async fn add_project(
    ctx: State<'_, HostCtx>,
    path: String,
    deploy_mode: Option<String>,
) -> Result<ProjectDto, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || add_project_core(&ctx, path, deploy_mode)).await?
}

pub fn add_project_core(
    ctx: &HostCtx,
    path: String,
    deploy_mode: Option<String>,
) -> Result<ProjectDto, AppError> {
    let store = ctx.store.clone();
    let project_path = Path::new(&path);
    if !project_path.is_dir() {
        return Err(AppError::invalid_input("Directory does not exist"));
    }
    let deploy_mode = new_project_deploy_mode(&store, deploy_mode)?;
    let relative_skills_dir = match deploy_mode.as_str() {
        "link" => ".claude/skills",
        "copy" => VENDORED_SKILLS_DIR,
        _ => return Err(AppError::invalid_input("Deploy mode must be link or copy")),
    };
    let skills_dir = project_path.join(relative_skills_dir);
    let disabled_dir = project_path.join(format!("{relative_skills_dir}-disabled"));

    // Support initializing an empty project directory as a managed project.
    std::fs::create_dir_all(&skills_dir)?;
    std::fs::create_dir_all(&disabled_dir)?;

    let name = project_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let now = chrono::Utc::now().timestamp_millis();
    let record = ProjectRecord {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        path: path.clone(),
        workspace_type: "project".to_string(),
        linked_agent_key: None,
        linked_agent_name: None,
        disabled_path: None,
        sort_order: 0,
        created_at: now,
        updated_at: now,
        agent_keys: None,
        deploy_mode,
    };

    store.insert_project(&record).map_err(AppError::db)?;
    let all_managed = store.get_all_skills().map_err(AppError::db)?;
    let configs = agent_skill_configs(&store);
    Ok(project_to_dto(&record, &all_managed, &configs))
}

#[tauri::command]
pub async fn add_linked_workspace(
    ctx: State<'_, HostCtx>,
    name: String,
    path: String,
    disabled_path: Option<String>,
) -> Result<ProjectDto, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        add_linked_workspace_core(&ctx, name, path, disabled_path)
    })
    .await?
}

pub fn add_linked_workspace_core(
    ctx: &HostCtx,
    name: String,
    path: String,
    disabled_path: Option<String>,
) -> Result<ProjectDto, AppError> {
    let store = ctx.store.clone();
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::invalid_input("Workspace name is required"));
    }

    let skills_root = PathBuf::from(path.trim());
    if !skills_root.is_dir() {
        return Err(AppError::invalid_input("Skills directory does not exist"));
    }

    let disabled_path = disabled_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let disabled_path = if let Some(disabled) = disabled_path {
        let disabled_root = PathBuf::from(&disabled);
        if !disabled_root.is_dir() {
            return Err(AppError::invalid_input(
                "Disabled skills directory does not exist",
            ));
        }
        ensure_distinct_linked_workspace_roots(&skills_root, &disabled_root)?;
        Some(disabled)
    } else {
        let mut disabled_root = skills_root.clone();
        let derived = disabled_root
            .file_name()
            .and_then(|n| n.to_str())
            .map(|name| format!("{}-disabled", name));
        match derived {
            Some(name) => {
                disabled_root.set_file_name(name);
                match std::fs::create_dir_all(&disabled_root) {
                    Ok(()) => {
                        ensure_distinct_linked_workspace_roots(&skills_root, &disabled_root)?;
                        Some(disabled_root.to_string_lossy().to_string())
                    }
                    Err(_) => None,
                }
            }
            None => None,
        }
    };

    let now = chrono::Utc::now().timestamp_millis();
    let record = ProjectRecord {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        path: skills_root.to_string_lossy().to_string(),
        workspace_type: "linked".to_string(),
        linked_agent_key: Some(slugify_skill_dir_name(&name)),
        linked_agent_name: Some(name),
        disabled_path,
        sort_order: 0,
        created_at: now,
        updated_at: now,
        agent_keys: None,
        deploy_mode: "link".to_string(),
    };

    store.insert_project(&record).map_err(AppError::db)?;
    let all_managed = store.get_all_skills().map_err(AppError::db)?;
    let configs = agent_skill_configs(&store);
    Ok(project_to_dto(&record, &all_managed, &configs))
}

#[tauri::command]
pub async fn remove_project(ctx: State<'_, HostCtx>, id: String) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || remove_project_core(&ctx, id)).await?
}

pub fn remove_project_core(ctx: &HostCtx, id: String) -> Result<(), AppError> {
    let store = ctx.store.clone();
    store.delete_project(&id).map_err(AppError::db)
}

#[tauri::command]
pub async fn reorder_projects(ids: Vec<String>, ctx: State<'_, HostCtx>) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || reorder_projects_core(&ctx, ids)).await?
}

pub fn reorder_projects_core(ctx: &HostCtx, ids: Vec<String>) -> Result<(), AppError> {
    let store = ctx.store.clone();
    store.reorder_projects(&ids).map_err(AppError::db)
}

#[tauri::command]
pub async fn scan_projects(root: String, ctx: State<'_, HostCtx>) -> Result<Vec<String>, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || scan_projects_core(&ctx, root)).await?
}

pub fn scan_projects_core(ctx: &HostCtx, root: String) -> Result<Vec<String>, AppError> {
    let store = ctx.store.clone();
    let root_path = Path::new(&root);
    if !root_path.is_dir() {
        return Err(AppError::invalid_input("Directory does not exist"));
    }
    let configs = agent_skill_configs(&store);
    Ok(project_scanner::scan_projects_in_dir(
        root_path, 4, &configs,
    ))
}

#[cfg(test)]
mod tests {
    use super::{ensure_distinct_linked_workspace_roots, new_project_deploy_mode};
    use crate::core::skill_store::SkillStore;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn a_new_project_follows_the_default_deploy_mode_unless_one_is_named() {
        let tmp = tempdir().unwrap();
        let store = SkillStore::new(&tmp.path().join("test.db")).unwrap();
        let mode = |requested: Option<&str>| {
            new_project_deploy_mode(&store, requested.map(str::to_string)).unwrap()
        };

        assert_eq!(mode(None), "link");

        store
            .set_setting("default_project_deploy_mode", "copy")
            .unwrap();
        assert_eq!(mode(None), "copy");
        assert_eq!(mode(Some("link")), "link");

        store
            .set_setting("default_project_deploy_mode", "bogus")
            .unwrap();
        assert_eq!(mode(None), "link");
    }

    #[test]
    fn linked_workspace_roots_reject_same_directory() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("skills");
        fs::create_dir_all(&root).unwrap();

        let err = ensure_distinct_linked_workspace_roots(&root, &root).unwrap_err();
        assert!(
            err.to_string().contains("must not overlap"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn linked_workspace_roots_reject_nested_directory() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("skills");
        let nested = root.join("disabled");
        fs::create_dir_all(&nested).unwrap();

        let err = ensure_distinct_linked_workspace_roots(&root, &nested).unwrap_err();
        assert!(
            err.to_string().contains("must not overlap"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn linked_workspace_roots_allow_distinct_directories() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("skills");
        let disabled = tmp.path().join("skills-disabled");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&disabled).unwrap();

        ensure_distinct_linked_workspace_roots(&root, &disabled).unwrap();
    }
}
