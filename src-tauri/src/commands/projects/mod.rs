use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use tauri::State;

use crate::core::project_deploy::{
    self, AgentChangePlan, ConversionOutcome, SkillConversion, SkillOutcome,
};
use crate::core::project_scanner::VENDORED_SKILLS_DIR;
use crate::core::project_skill_match::{
    classify_sync_status, find_best_center_match, slugify_skill_dir_name,
    source_ref_matches_skill_path,
};
use crate::core::skill_store::{ProjectRecord, SkillRecord, SkillStore};
use crate::core::timing::should_log_first_or_slow;
use crate::core::{error::AppError, host::HostCtx, installer, project_scanner, sync_engine};

mod types;
pub use types::*;

mod agents_model;
use agents_model::*;

mod fs_safety;
use fs_safety::*;
pub(crate) use fs_safety::{ensure_dir_within_root, ensure_safe_skill_relative_path};

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

#[tauri::command]
pub async fn get_project_agent_targets(
    ctx: State<'_, HostCtx>,
    project_id: String,
) -> Result<Vec<ProjectAgentTargetDto>, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || get_project_agent_targets_core(&ctx, project_id))
        .await?
}

pub fn get_project_agent_targets_core(
    ctx: &HostCtx,
    project_id: String,
) -> Result<Vec<ProjectAgentTargetDto>, AppError> {
    let store = ctx.store.clone();
    let record = store
        .get_project_by_id(&project_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Workspace not found"))?;
    Ok(project_agent_targets_for_record(&store, &record))
}

#[tauri::command]
pub async fn get_project_skills(
    ctx: State<'_, HostCtx>,
    project_id: String,
) -> Result<Vec<project_scanner::ProjectSkillInfo>, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || get_project_skills_core(&ctx, project_id)).await?
}

pub fn get_project_skills_core(
    ctx: &HostCtx,
    project_id: String,
) -> Result<Vec<project_scanner::ProjectSkillInfo>, AppError> {
    let store = ctx.store.clone();
    let record = store
        .get_project_by_id(&project_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Workspace not found"))?;

    let configs = agent_skill_configs(&store);
    let mut skills = read_workspace_skills(&record, &configs);

    let all_managed = store.get_all_skills().unwrap_or_default();
    let tags_map = store.get_tags_map().unwrap_or_default();
    let overridden: HashSet<String> = store
        .get_project_skill_agent_overrides(&record.id)
        .unwrap_or_default()
        .into_keys()
        .map(|path| path.to_lowercase())
        .collect();
    for skill in &mut skills {
        skill.agents_overridden = overridden.contains(&skill.relative_path.to_lowercase());
        let matched = find_best_center_match(skill, &all_managed);
        skill.in_center = matched.is_some();
        skill.center_skill_id = matched.map(|m| m.id.clone());
        skill.tags = skill
            .center_skill_id
            .as_ref()
            .and_then(|skill_id| tags_map.get(skill_id).cloned())
            .unwrap_or_default();
        skill.sync_status = classify_sync_status(skill, matched);
    }

    Ok(skills)
}

#[tauri::command]
pub async fn get_project_skill_document(
    project_id: String,
    skill_relative_path: String,
    agent: String,
    ctx: State<'_, HostCtx>,
) -> Result<ProjectSkillDocumentDto, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        get_project_skill_document_core(&ctx, project_id, skill_relative_path, agent)
    })
    .await?
}

pub fn get_project_skill_document_core(
    ctx: &HostCtx,
    project_id: String,
    skill_relative_path: String,
    agent: String,
) -> Result<ProjectSkillDocumentDto, AppError> {
    let store = ctx.store.clone();
    ensure_safe_skill_relative_path(&skill_relative_path)?;

    let record = store
        .get_project_by_id(&project_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Workspace not found"))?;

    let (skills_root, disabled_root) = resolve_agent_skills_roots(&store, &record, &agent)
        .ok_or_else(|| AppError::not_found(format!("Unknown workspace agent: {}", agent)))?;
    let disabled_root_copy = disabled_root.clone();
    let skill_dir = skills_root.join(&skill_relative_path);
    let skill_dir = if skill_dir.is_dir() {
        ensure_dir_within_root(&skill_dir, &skills_root)?;
        skill_dir
    } else if let Some(disabled_root) = disabled_root {
        let disabled = disabled_root.join(&skill_relative_path);
        if disabled.is_dir() {
            ensure_dir_within_root(&disabled, &disabled_root)?;
            disabled
        } else {
            return Err(AppError::not_found("Skill directory not found"));
        }
    } else {
        return Err(AppError::not_found("Skill directory not found"));
    };

    // Collect all allowed roots for symlink target validation
    let mut allowed_roots: Vec<PathBuf> = vec![skills_root.clone()];
    if let Some(dr) = disabled_root_copy {
        allowed_roots.push(dr);
    }
    // For project workspaces, also allow the project root itself
    if record.workspace_type != "linked" {
        allowed_roots.push(PathBuf::from(&record.path));
    }

    let candidates = ["SKILL.md", "skill.md", "CLAUDE.md", "README.md"];
    for candidate in &candidates {
        let file_path = skill_dir.join(candidate);
        if !file_path.exists() {
            continue;
        }
        // For symlinks, verify the resolved target stays within an allowed root
        if let Ok(meta) = std::fs::symlink_metadata(&file_path) {
            if meta.file_type().is_symlink() {
                let resolved = match std::fs::canonicalize(&file_path) {
                    Ok(r) => r,
                    Err(_) => continue, // broken symlink
                };
                let in_allowed_root = allowed_roots.iter().any(|root| {
                    std::fs::canonicalize(root)
                        .map(|canon| resolved.starts_with(&canon))
                        .unwrap_or(false)
                });
                if !in_allowed_root {
                    continue;
                }
            }
        }
        if file_path.is_file() {
            let content = std::fs::read_to_string(&file_path)?;
            return Ok(ProjectSkillDocumentDto {
                skill_name: skill_relative_path,
                filename: candidate.to_string(),
                content,
            });
        }
    }

    Err(AppError::not_found(
        "No document file found in skill directory",
    ))
}

#[tauri::command]
pub async fn import_project_skill_to_center(
    ctx: State<'_, HostCtx>,
    project_id: String,
    skill_relative_path: String,
    agent: String,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        import_project_skill_to_center_core(&ctx, project_id, skill_relative_path, agent)
    })
    .await?
}

pub fn import_project_skill_to_center_core(
    ctx: &HostCtx,
    project_id: String,
    skill_relative_path: String,
    agent: String,
) -> Result<(), AppError> {
    let store = ctx.store.clone();
    ensure_safe_skill_relative_path(&skill_relative_path)?;

    let record = store
        .get_project_by_id(&project_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Workspace not found"))?;

    let configs = agent_skill_configs(&store);
    let skills = read_workspace_skills(&record, &configs);
    let skill = skills
        .iter()
        .find(|s| s.relative_path == skill_relative_path && s.agent == agent)
        .ok_or_else(|| AppError::not_found("Skill not found in workspace"))?;
    // A link to a vendored copy imports the vendored copy, and binds to it.
    let skill = vendored_variant(&record, &skills, skill).unwrap_or(skill);

    let source_path = PathBuf::from(&skill.path);
    let all_managed = store.get_all_skills().unwrap_or_default();
    // Use the same matching logic as the UI (find_best_center_match) to
    // stay consistent with sync-status display. After updating, bind
    // source_ref so future imports match by exact path.
    if let Some(existing) = find_best_center_match(skill, &all_managed) {
        let result = installer::install_from_local_to_destination(
            &source_path,
            Some(&existing.name),
            Path::new(&existing.central_path),
        )
        .map_err(AppError::io)?;
        store
            .update_skill_after_install(
                &existing.id,
                &existing.name,
                result.description.as_deref(),
                existing.source_revision.as_deref(),
                existing.remote_revision.as_deref(),
                Some(&result.content_hash),
                "local_only",
            )
            .map_err(AppError::db)?;
        // Only update source_ref when the match was already by source_ref
        // path (not by hash or name). This avoids permanently rebinding
        // unrelated center skills that merely share a name or content.
        let already_matched_by_ref = source_ref_matches_skill_path(
            &skill.path,
            std::fs::canonicalize(&skill.path).ok().as_ref(),
            existing,
        );
        if existing.source_type == "local" && already_matched_by_ref {
            store
                .update_skill_source_ref(&existing.id, &skill.path)
                .map_err(AppError::db)?;
        }
        return Ok(());
    }

    let result =
        installer::install_from_local(&source_path, Some(&skill.name)).map_err(AppError::io)?;

    let now = chrono::Utc::now().timestamp_millis();
    let id = uuid::Uuid::new_v4().to_string();

    let skill_record = SkillRecord {
        id: id.clone(),
        name: result.name.clone(),
        description: result.description.clone(),
        source_type: "local".to_string(),
        source_ref: Some(skill.path.clone()),
        source_ref_resolved: None,
        source_subpath: None,
        source_branch: None,
        source_revision: None,
        remote_revision: None,
        central_path: result.central_path.to_string_lossy().to_string(),
        content_hash: Some(result.content_hash.clone()),
        enabled: true,
        created_at: now,
        updated_at: now,
        status: "ok".to_string(),
        update_status: "local_only".to_string(),
        last_checked_at: Some(now),
        last_check_error: None,
    };

    store.insert_skill(&skill_record).map_err(AppError::db)?;

    Ok(())
}

#[tauri::command]
pub async fn update_project_skill_to_center(
    ctx: State<'_, HostCtx>,
    project_id: String,
    skill_relative_path: String,
    agent: String,
) -> Result<(), AppError> {
    import_project_skill_to_center(ctx, project_id, skill_relative_path, agent).await
}

#[tauri::command]
pub fn slugify_skill_names(names: Vec<String>) -> Vec<String> {
    names.iter().map(|n| slugify_skill_dir_name(n)).collect()
}

#[tauri::command]
pub async fn export_skill_to_project(
    ctx: State<'_, HostCtx>,
    skill_id: String,
    project_id: String,
    agents: Option<Vec<String>>,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        export_skill_to_project_core(&ctx, skill_id, project_id, agents)
    })
    .await?
}

pub fn export_skill_to_project_core(
    ctx: &HostCtx,
    skill_id: String,
    project_id: String,
    agents: Option<Vec<String>>,
) -> Result<(), AppError> {
    let store = ctx.store.clone();
    let project = store
        .get_project_by_id(&project_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Workspace not found"))?;

    let skill = store
        .get_skill_by_id(&skill_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Skill not found"))?;

    let source = PathBuf::from(&skill.central_path);
    let dir_name = sync_engine::target_dir_name(&source, &skill.name);
    ensure_safe_skill_relative_path(&dir_name)?;
    let agent_keys = export_agent_keys(&store, &project, agents)?;

    if is_copy_project(&project) {
        let failed = project_deploy::deploy_copy_mode(
            Path::new(&project.path),
            &agent_skill_configs(&store),
            &source,
            &dir_name,
            &agent_keys,
        )
        .map_err(AppError::io)?;
        if !failed.is_empty() {
            let failures: Vec<String> = failed
                .iter()
                .map(|failure| format!("{}: {}", failure.agent, failure.error))
                .collect();
            return Err(AppError::io(format!(
                "\"{}\" was vendored into {VENDORED_SKILLS_DIR}, but links were not created for {}",
                skill.name,
                failures.join("; ")
            )));
        }
        return Ok(());
    }

    for agent_key in &agent_keys {
        let (skills_root, disabled_root) = resolve_agent_skills_roots(&store, &project, agent_key)
            .ok_or_else(|| AppError::not_found(format!("Unknown agent: {}", agent_key)))?;
        let target_dir = skills_root.join(&dir_name);

        if target_dir.strip_prefix(&skills_root).is_err() {
            return Err(AppError::invalid_input("Invalid skill directory path"));
        }

        if target_dir.exists()
            || disabled_root
                .as_ref()
                .map(|path| path.join(&dir_name).exists())
                .unwrap_or(false)
        {
            return Err(AppError::invalid_input(format!(
                "Skill \"{}\" already exists in this workspace for agent {}",
                skill.name, agent_key
            )));
        }
    }

    let configured_mode = store.get_setting("sync_mode").map_err(AppError::db)?;
    // Two agents can resolve to the same project skills root, in which case
    // the second pass would find the directory the first just wrote and
    // refuse it. The artifact is already correct, so skip instead.
    let mut written: HashSet<PathBuf> = HashSet::new();
    for agent_key in &agent_keys {
        let (skills_root, _) = resolve_agent_skills_roots(&store, &project, agent_key)
            .ok_or_else(|| AppError::not_found(format!("Unknown agent: {}", agent_key)))?;
        if !written.insert(skills_root.join(&dir_name)) {
            continue;
        }
        let mode = sync_engine::sync_mode_for_tool(agent_key, configured_mode.as_deref());
        project_deploy::deploy_skill(&source, &skills_root, &dir_name, mode)
            .map_err(AppError::io)?;
    }

    Ok(())
}

#[tauri::command]
pub async fn update_project_skill_from_center(
    ctx: State<'_, HostCtx>,
    project_id: String,
    skill_relative_path: String,
    agent: String,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        update_project_skill_from_center_core(&ctx, project_id, skill_relative_path, agent)
    })
    .await?
}

pub fn update_project_skill_from_center_core(
    ctx: &HostCtx,
    project_id: String,
    skill_relative_path: String,
    agent: String,
) -> Result<(), AppError> {
    let store = ctx.store.clone();
    ensure_safe_skill_relative_path(&skill_relative_path)?;

    let record = store
        .get_project_by_id(&project_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Workspace not found"))?;

    let configs = agent_skill_configs(&store);
    let skills = read_workspace_skills(&record, &configs);
    let skill = skills
        .iter()
        .find(|s| s.relative_path == skill_relative_path && s.agent == agent)
        .ok_or_else(|| AppError::not_found("Skill not found in workspace"))?;

    let all_managed = store.get_all_skills().unwrap_or_default();
    // Copy mode updates the vendored copy only; its links follow.
    if let Some(vendored) = vendored_variant(&record, &skills, skill) {
        return update_vendored_from_center(vendored, &all_managed);
    }
    let managed = find_best_center_match(skill, &all_managed)
        .ok_or_else(|| AppError::not_found("No matching skill in center"))?;
    ensure_not_project_newer(skill, managed)?;

    let (skills_root, disabled_root) = resolve_agent_skills_roots(&store, &record, &agent)
        .ok_or_else(|| AppError::not_found(format!("Unknown agent: {}", agent)))?;
    let target_path = PathBuf::from(&skill.path);
    if target_path.starts_with(&skills_root) {
        ensure_dir_within_root(&target_path, &skills_root)?;
    } else if disabled_root
        .as_ref()
        .map(|root| target_path.starts_with(root))
        .unwrap_or(false)
    {
        let disabled_root = disabled_root.expect("checked above");
        ensure_dir_within_root(&target_path, &disabled_root)?;
    } else {
        return Err(AppError::invalid_input("Invalid skill directory path"));
    }

    let source = PathBuf::from(&managed.central_path);
    let configured_mode = store.get_setting("sync_mode").map_err(AppError::db)?;
    // A copy-mode project holds files, never links into the library.
    let mode = if is_copy_project(&record) {
        sync_engine::SyncMode::Copy
    } else {
        sync_engine::sync_mode_for_tool(&agent, configured_mode.as_deref())
    };
    // UserConfirmed: this intentionally replaces an existing project copy
    // the user chose to update, and project deployments never create
    // `skill_targets` rows, so no record could vouch for it. The
    // project_newer check above is the guard that makes this safe.
    sync_engine::sync_skill(
        &source,
        &target_path,
        mode,
        sync_engine::ReplacePolicy::UserConfirmed,
    )
    .map_err(AppError::io)?;
    Ok(())
}

/// Mirror the global-workspace protection (agent_workspace.rs): never
/// overwrite a project copy that has unsynced local edits (#225 review).
fn ensure_not_project_newer(
    skill: &project_scanner::ProjectSkillInfo,
    managed: &SkillRecord,
) -> Result<(), AppError> {
    if classify_sync_status(skill, Some(managed)) == "project_newer" {
        return Err(AppError::invalid_input(
            "Project skill is newer than the Skills Center version",
        ));
    }
    Ok(())
}

/// Pull the library version into a vendored copy, replacing it in place so
/// the agent links to it keep resolving.
fn update_vendored_from_center(
    vendored: &project_scanner::ProjectSkillInfo,
    all_managed: &[SkillRecord],
) -> Result<(), AppError> {
    let managed = find_best_center_match(vendored, all_managed)
        .ok_or_else(|| AppError::not_found("No matching skill in center"))?;
    ensure_not_project_newer(vendored, managed)?;
    // UserConfirmed for the same reason as a link-mode update: the user asked
    // for this copy to be replaced, and the check above guards their edits.
    sync_engine::sync_skill(
        Path::new(&managed.central_path),
        Path::new(&vendored.path),
        sync_engine::SyncMode::Copy,
        sync_engine::ReplacePolicy::UserConfirmed,
    )
    .map_err(AppError::io)?;
    Ok(())
}

#[tauri::command]
pub async fn toggle_project_skill(
    ctx: State<'_, HostCtx>,
    project_id: String,
    skill_relative_path: String,
    agent: String,
    enabled: bool,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        toggle_project_skill_core(&ctx, project_id, skill_relative_path, agent, enabled)
    })
    .await?
}

pub fn toggle_project_skill_core(
    ctx: &HostCtx,
    project_id: String,
    skill_relative_path: String,
    agent: String,
    enabled: bool,
) -> Result<(), AppError> {
    let store = ctx.store.clone();
    ensure_safe_skill_relative_path(&skill_relative_path)?;

    let record = store
        .get_project_by_id(&project_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Workspace not found"))?;
    toggle_skill_copy(&store, &record, &skill_relative_path, &agent, enabled)
}

/// Enable or disable one agent's copy of a skill. A vendored copy, or a link
/// to one, moves together with every link to it, whatever the project's
/// deploy mode: it only decides how skills are added.
fn toggle_skill_copy(
    store: &SkillStore,
    record: &ProjectRecord,
    relative_path: &str,
    agent: &str,
    enabled: bool,
) -> Result<(), AppError> {
    if record.workspace_type != "linked" {
        let configs = agent_skill_configs(store);
        let project_root = Path::new(&record.path);
        if project_deploy::shares_vendored_copy(project_root, &configs, agent, relative_path) {
            return project_deploy::set_vendored_enabled(
                project_root,
                &configs,
                relative_path,
                enabled,
            )
            .map_err(AppError::io);
        }
    }

    let (skills_dir, disabled_dir) = resolve_agent_skills_roots(store, record, agent)
        .ok_or_else(|| AppError::not_found(format!("Unknown agent: {}", agent)))?;
    let disabled_dir = disabled_dir.ok_or_else(|| {
        AppError::invalid_input("This workspace does not support disabling skills")
    })?;

    set_project_skill_enabled_state(&skills_dir, &disabled_dir, relative_path, enabled)
}

#[tauri::command]
pub async fn delete_project_skill(
    ctx: State<'_, HostCtx>,
    project_id: String,
    skill_relative_path: String,
    agent: String,
    whole_skill: Option<bool>,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        delete_project_skill_core(&ctx, project_id, skill_relative_path, agent, whole_skill)
    })
    .await?
}

pub fn delete_project_skill_core(
    ctx: &HostCtx,
    project_id: String,
    skill_relative_path: String,
    agent: String,
    whole_skill: Option<bool>,
) -> Result<(), AppError> {
    let store = ctx.store.clone();
    ensure_safe_skill_relative_path(&skill_relative_path)?;

    let record = store
        .get_project_by_id(&project_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Workspace not found"))?;
    delete_skill_copy(
        &store,
        &record,
        &skill_relative_path,
        &agent,
        whole_skill.unwrap_or(false),
    )
}

/// Delete one agent's copy of a skill. A vendored copy goes only as part of
/// deleting the `whole_skill`, and takes every link to it along; one agent
/// cannot take away the files the others read, or edits made in the repo.
fn delete_skill_copy(
    store: &SkillStore,
    record: &ProjectRecord,
    relative_path: &str,
    agent: &str,
    whole_skill: bool,
) -> Result<(), AppError> {
    if record.workspace_type != "linked" {
        let configs = agent_skill_configs(store);
        let project_root = Path::new(&record.path);
        if project_deploy::is_vendored_agent(&configs, agent)
            && project_deploy::vendored_copy(project_root, relative_path).is_some()
        {
            if !whole_skill {
                return Err(AppError::invalid_input(format!(
                    "\"{relative_path}\" is the vendored copy in {VENDORED_SKILLS_DIR} that other \
                     agents read; delete the whole skill to remove it"
                )));
            }
            project_deploy::delete_vendored(project_root, &configs, relative_path)
                .map_err(AppError::io)?;
            return clear_override_without_copies(store, record, relative_path);
        }
    }

    let (skills_root, disabled_root) = resolve_agent_skills_roots(store, record, agent)
        .ok_or_else(|| AppError::not_found(format!("Unknown agent: {}", agent)))?;
    let skills_dir = skills_root.join(relative_path);
    let disabled_dir = disabled_root.as_ref().map(|root| root.join(relative_path));

    let (target, target_root) = if skills_dir.is_dir() {
        (skills_dir, skills_root)
    } else if let Some(disabled_dir) = disabled_dir.filter(|path| path.is_dir()) {
        (
            disabled_dir,
            disabled_root.expect("present when disabled_dir exists"),
        )
    } else {
        return Err(AppError::not_found("Skill directory not found"));
    };

    ensure_dir_within_root(&target, &target_root)?;
    remove_workspace_skill_target(&target)?;
    clear_override_without_copies(store, record, relative_path)
}

/// A hand-picked agent set outlives none of the skill's copies.
fn clear_override_without_copies(
    store: &SkillStore,
    record: &ProjectRecord,
    relative_path: &str,
) -> Result<(), AppError> {
    if record.workspace_type != "linked"
        && !skill_has_any_copy(
            &agent_skill_configs(store),
            Path::new(&record.path),
            relative_path,
        )
    {
        store
            .clear_project_skill_agent_override(&record.id, relative_path)
            .map_err(AppError::db)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_project_agent_keys(
    ctx: State<'_, HostCtx>,
    project_id: String,
    agent_keys: Option<Vec<String>>,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        set_project_agent_keys_core(&ctx, project_id, agent_keys)
    })
    .await?
}

pub fn set_project_agent_keys_core(
    ctx: &HostCtx,
    project_id: String,
    agent_keys: Option<Vec<String>>,
) -> Result<(), AppError> {
    let store = ctx.store.clone();
    let record = get_agent_selectable_project(&store, &project_id)?;
    let agent_keys = agent_keys
        .map(|keys| validated_agent_keys(&agent_skill_configs(&store), keys))
        .transpose()?;
    store
        .set_project_agent_keys(&record.id, agent_keys.as_deref())
        .map_err(AppError::db)
}

#[tauri::command]
pub async fn preview_project_agent_change(
    ctx: State<'_, HostCtx>,
    project_id: String,
    agent_keys: Vec<String>,
) -> Result<AgentChangePlan, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        preview_project_agent_change_core(&ctx, project_id, agent_keys)
    })
    .await?
}

pub fn preview_project_agent_change_core(
    ctx: &HostCtx,
    project_id: String,
    agent_keys: Vec<String>,
) -> Result<AgentChangePlan, AppError> {
    let store = ctx.store.clone();
    let record = get_agent_selectable_project(&store, &project_id)?;
    let configs = agent_skill_configs(&store);
    let desired = validated_agent_keys(&configs, agent_keys)?;
    plan_project_agent_change(&store, &record, &configs, &desired)
}

#[tauri::command]
pub async fn apply_project_agent_change(
    ctx: State<'_, HostCtx>,
    project_id: String,
    agent_keys: Vec<String>,
) -> Result<Vec<SkillOutcome>, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        apply_project_agent_change_core(&ctx, project_id, agent_keys)
    })
    .await?
}

pub fn apply_project_agent_change_core(
    ctx: &HostCtx,
    project_id: String,
    agent_keys: Vec<String>,
) -> Result<Vec<SkillOutcome>, AppError> {
    let store = ctx.store.clone();
    let record = get_agent_selectable_project(&store, &project_id)?;
    let configs = agent_skill_configs(&store);
    let desired = validated_agent_keys(&configs, agent_keys)?;
    // Plan again: the disk may have moved on since the preview.
    let plan = plan_project_agent_change(&store, &record, &configs, &desired)?;
    let configured_mode = store.get_setting("sync_mode").map_err(AppError::db)?;
    let outcomes = project_deploy::apply_agent_change(
        Path::new(&record.path),
        &configs,
        &plan.skills,
        configured_mode.as_deref(),
        false,
    );
    // Saved even when some agents failed: it is still what the user
    // chose, and the outcomes say what did not happen.
    store
        .set_project_agent_keys(&record.id, Some(&desired))
        .map_err(AppError::db)?;
    Ok(outcomes)
}

/// Put one skill on `desired` by hand and record the agents it is on after:
/// an agent that failed or was kept is recorded as it ended up, not as asked,
/// and the outcome says what did not happen.
fn choose_skill_agents(
    store: &SkillStore,
    record: &ProjectRecord,
    relative_path: &str,
    desired: &[String],
) -> Result<SkillOutcome, AppError> {
    // Unticking an agent on one skill deletes that copy, real directory
    // or not, exactly as the per-agent toggle always has.
    let outcome = reconcile_skill_agents(store, record, relative_path, desired, true)?;
    let mut achieved: Vec<String> = Vec::new();
    for skill in read_workspace_skills(record, &agent_skill_configs(store)) {
        if skill.relative_path.eq_ignore_ascii_case(relative_path)
            && !achieved.contains(&skill.agent)
        {
            achieved.push(skill.agent);
        }
    }
    if achieved.is_empty() {
        store.clear_project_skill_agent_override(&record.id, relative_path)
    } else {
        store.set_project_skill_agent_override(&record.id, relative_path, &achieved)
    }
    .map_err(AppError::db)?;
    Ok(outcome)
}

/// Choose one skill's agents by hand. The skill is then left out of bulk
/// agent changes until its override is cleared.
#[tauri::command]
pub async fn set_project_skill_agents(
    ctx: State<'_, HostCtx>,
    project_id: String,
    skill_relative_path: String,
    agent_keys: Vec<String>,
) -> Result<SkillOutcome, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        set_project_skill_agents_core(&ctx, project_id, skill_relative_path, agent_keys)
    })
    .await?
}

pub fn set_project_skill_agents_core(
    ctx: &HostCtx,
    project_id: String,
    skill_relative_path: String,
    agent_keys: Vec<String>,
) -> Result<SkillOutcome, AppError> {
    let store = ctx.store.clone();
    ensure_safe_skill_relative_path(&skill_relative_path)?;
    let record = get_agent_selectable_project(&store, &project_id)?;
    let desired = validated_agent_keys(&agent_skill_configs(&store), agent_keys)?;
    choose_skill_agents(&store, &record, &skill_relative_path, &desired)
}

/// Drop a skill's hand-picked agents and put it back on the project's.
#[tauri::command]
pub async fn clear_project_skill_agents(
    ctx: State<'_, HostCtx>,
    project_id: String,
    skill_relative_path: String,
) -> Result<SkillOutcome, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        clear_project_skill_agents_core(&ctx, project_id, skill_relative_path)
    })
    .await?
}

pub fn clear_project_skill_agents_core(
    ctx: &HostCtx,
    project_id: String,
    skill_relative_path: String,
) -> Result<SkillOutcome, AppError> {
    let store = ctx.store.clone();
    ensure_safe_skill_relative_path(&skill_relative_path)?;
    let record = get_agent_selectable_project(&store, &project_id)?;
    store
        .clear_project_skill_agent_override(&record.id, &skill_relative_path)
        .map_err(AppError::db)?;
    let desired = record
        .agent_keys
        .clone()
        .unwrap_or_else(|| effective_project_agent_keys(&store, &record));
    reconcile_skill_agents(&store, &record, &skill_relative_path, &desired, false)
}

/// Switch a project back to linking skills from the library. Only affects
/// skills added from now on: vendored copies stay, being committed files.
/// Switching to copy mode goes through the convert commands.
#[tauri::command]
pub async fn set_project_deploy_mode(
    ctx: State<'_, HostCtx>,
    project_id: String,
    deploy_mode: String,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        set_project_deploy_mode_core(&ctx, project_id, deploy_mode)
    })
    .await?
}

pub fn set_project_deploy_mode_core(
    ctx: &HostCtx,
    project_id: String,
    deploy_mode: String,
) -> Result<(), AppError> {
    let store = ctx.store.clone();
    let record = get_deploy_mode_project(&store, &project_id)?;
    if deploy_mode != "link" {
        return Err(AppError::invalid_input(
            "Convert the project to switch it to copy mode",
        ));
    }
    store
        .set_project_deploy_mode(&record.id, "link")
        .map_err(AppError::db)
}

fn plan_project_convert(store: &SkillStore, rec: &ProjectRecord) -> Vec<SkillConversion> {
    let configs = agent_skill_configs(store);
    let skills = read_workspace_skills(rec, &configs);
    project_deploy::plan_convert_to_copy(Path::new(&rec.path), &configs, &skills)
}

#[tauri::command]
pub async fn preview_project_convert_to_copy(
    ctx: State<'_, HostCtx>,
    project_id: String,
) -> Result<Vec<SkillConversion>, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        preview_project_convert_to_copy_core(&ctx, project_id)
    })
    .await?
}

pub fn preview_project_convert_to_copy_core(
    ctx: &HostCtx,
    project_id: String,
) -> Result<Vec<SkillConversion>, AppError> {
    let store = ctx.store.clone();
    let record = get_deploy_mode_project(&store, &project_id)?;
    Ok(plan_project_convert(&store, &record))
}

#[tauri::command]
pub async fn apply_project_convert_to_copy(
    ctx: State<'_, HostCtx>,
    project_id: String,
) -> Result<Vec<ConversionOutcome>, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        apply_project_convert_to_copy_core(&ctx, project_id)
    })
    .await?
}

pub fn apply_project_convert_to_copy_core(
    ctx: &HostCtx,
    project_id: String,
) -> Result<Vec<ConversionOutcome>, AppError> {
    let store = ctx.store.clone();
    let record = get_deploy_mode_project(&store, &project_id)?;
    convert_project_to_copy(&store, &record)
}

/// Vendor every skill, relink what can be relinked, and switch the project to
/// copy mode. The mode is saved even when some skills failed: the outcomes
/// say which, and later adds are vendored either way.
fn convert_project_to_copy(
    store: &SkillStore,
    rec: &ProjectRecord,
) -> Result<Vec<ConversionOutcome>, AppError> {
    // Plan again: the disk may have moved on since the preview.
    let conversions = plan_project_convert(store, rec);
    let project_root = Path::new(&rec.path);
    std::fs::create_dir_all(project_root.join(VENDORED_SKILLS_DIR))?;
    std::fs::create_dir_all(project_root.join(format!("{VENDORED_SKILLS_DIR}-disabled")))?;
    let outcomes = project_deploy::apply_convert_to_copy(project_root, &conversions);
    store
        .set_project_deploy_mode(&rec.id, "copy")
        .map_err(AppError::db)?;
    Ok(outcomes)
}

#[cfg(test)]
mod test_fixtures;

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::test_fixtures::{agent_selection_fixture, keys, update_vendored_x};
    #[cfg(unix)]
    use super::{
        agent_skill_configs, convert_project_to_copy, delete_skill_copy, plan_project_agent_change,
        toggle_skill_copy,
    };
    use super::{ensure_distinct_linked_workspace_roots, new_project_deploy_mode};
    use crate::core::error::ErrorKind;
    #[cfg(unix)]
    use crate::core::project_deploy::{self, SkipReason};
    use crate::core::skill_store::{ProjectRecord, SkillStore};
    use std::fs;
    use std::path::Path;
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

    /// A hand-picked agent the skill could not be put on is reported, and not
    /// recorded as one of its agents.
    #[cfg(unix)]
    #[test]
    fn a_hand_picked_agent_is_recorded_only_once_the_skill_is_on_it() {
        let tmp = tempdir().unwrap();
        let (store, record) = agent_selection_fixture(tmp.path(), None);
        let project = Path::new(&record.path);
        fs::create_dir_all(project.join(".b")).unwrap();
        fs::write(project.join(".b/skills"), "not a folder").unwrap();

        let outcome =
            super::choose_skill_agents(&store, &record, "x", &keys(&["agent_a", "agent_b"]))
                .unwrap();

        assert_eq!(outcome.failed.len(), 1);
        assert_eq!(outcome.failed[0].agent, "agent_b");
        let overrides = store.get_project_skill_agent_overrides(&record.id).unwrap();
        assert_eq!(overrides.get("x"), Some(&keys(&["agent_a"])));
    }

    #[cfg(unix)]
    #[test]
    fn converting_vendors_the_project_and_later_changes_plan_in_copy_mode() {
        let tmp = tempdir().unwrap();
        let (store, record) = agent_selection_fixture(tmp.path(), None);

        let outcomes = convert_project_to_copy(&store, &record).unwrap();

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].vendored);
        assert_eq!(outcomes[0].relinked, keys(&["agent_a"]));
        let record = store.get_project_by_id(&record.id).unwrap().unwrap();
        assert_eq!(record.deploy_mode, "copy");
        let project = Path::new(&record.path);
        assert!(project.join(".agents/skills/x/SKILL.md").is_file());
        assert!(project.join(".agents/skills-disabled").is_dir());
        assert_eq!(
            fs::read_link(project.join(".a/skills/x")).unwrap(),
            Path::new("../../.agents/skills/x")
        );

        let configs = agent_skill_configs(&store);
        let plan =
            plan_project_agent_change(&store, &record, &configs, &keys(&["agent_b"])).unwrap();
        assert_eq!(plan.skills[0].adds, keys(&["agent_b"]));
        assert_eq!(plan.skills[0].removes, keys(&["agent_a"]));
        assert_eq!(plan.skills[0].skipped.len(), 1);
        assert_eq!(plan.skills[0].skipped[0].reason, SkipReason::SharedDir);
    }

    /// Pulling from the library replaces only the vendored copy, and in place,
    /// so the agent links keep resolving; an edit made in the repo is never
    /// pulled over.
    #[cfg(unix)]
    #[test]
    fn updating_a_vendored_skill_keeps_its_links_and_refuses_repo_edits() {
        let tmp = tempdir().unwrap();
        let (store, record) = agent_selection_fixture(tmp.path(), None);
        convert_project_to_copy(&store, &record).unwrap();
        let record = store.get_project_by_id(&record.id).unwrap().unwrap();
        let project = Path::new(&record.path);
        let library_md = tmp.path().join("library/x/SKILL.md");
        let vendored_md = project.join(".agents/skills/x/SKILL.md");

        fs::write(&library_md, "---\nname: x\n---\nnew\n").unwrap();
        update_vendored_x(&store, &record).unwrap();

        assert_eq!(
            fs::read_to_string(&vendored_md).unwrap(),
            "---\nname: x\n---\nnew\n"
        );
        assert_eq!(
            fs::read_to_string(project.join(".a/skills/x/SKILL.md")).unwrap(),
            "---\nname: x\n---\nnew\n"
        );

        fs::write(&vendored_md, "edited in the repo").unwrap();
        let an_hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        fs::File::options()
            .write(true)
            .open(&library_md)
            .unwrap()
            .set_modified(an_hour_ago)
            .unwrap();
        let err = update_vendored_x(&store, &record).unwrap_err();

        assert_eq!(err.kind, ErrorKind::InvalidInput);
        assert_eq!(
            fs::read_to_string(&vendored_md).unwrap(),
            "edited in the repo"
        );
    }

    /// `x` converted to copy mode: vendored in `.agents/skills` and linked
    /// from agent_a. Returns the converted record and the agent group that
    /// holds the vendored copy.
    #[cfg(unix)]
    fn vendored_fixture(tmp: &Path) -> (SkillStore, ProjectRecord, String) {
        let (store, record) = agent_selection_fixture(tmp, None);
        convert_project_to_copy(&store, &record).unwrap();
        let record = store.get_project_by_id(&record.id).unwrap().unwrap();
        let group = agent_skill_configs(&store)
            .into_iter()
            .find(|config| project_deploy::is_vendored_dir(&config.relative_skills_dir))
            .unwrap()
            .key;
        (store, record, group)
    }

    /// One agent cannot take away the vendored copy the others read, or the
    /// edits in it; deleting the whole skill can, links and all.
    #[cfg(unix)]
    #[test]
    fn only_deleting_the_whole_skill_removes_its_vendored_copy() {
        let tmp = tempdir().unwrap();
        let (store, record, group) = vendored_fixture(tmp.path());
        let project = Path::new(&record.path);

        let err = delete_skill_copy(&store, &record, "x", &group, false).unwrap_err();

        assert_eq!(err.kind, ErrorKind::InvalidInput);
        assert!(project.join(".agents/skills/x/SKILL.md").is_file());
        assert!(project.join(".a/skills/x/SKILL.md").is_file());

        delete_skill_copy(&store, &record, "x", &group, true).unwrap();

        assert!(fs::symlink_metadata(project.join(".agents/skills/x")).is_err());
        assert!(fs::symlink_metadata(project.join(".a/skills/x")).is_err());
    }

    /// Switching back to linking only changes how skills are added: a skill
    /// vendored before still toggles with its links and pulls into its files.
    #[cfg(unix)]
    #[test]
    fn a_project_switched_back_to_linking_keeps_its_vendored_skills_vendored() {
        let tmp = tempdir().unwrap();
        let (store, record, _) = vendored_fixture(tmp.path());
        store.set_project_deploy_mode(&record.id, "link").unwrap();
        let record = store.get_project_by_id(&record.id).unwrap().unwrap();
        let project = Path::new(&record.path);

        toggle_skill_copy(&store, &record, "x", "agent_a", false).unwrap();

        assert!(project.join(".agents/skills-disabled/x/SKILL.md").is_file());
        assert_eq!(
            fs::read_link(project.join(".a/skills-disabled/x")).unwrap(),
            Path::new("../../.agents/skills-disabled/x")
        );
        toggle_skill_copy(&store, &record, "x", "agent_a", true).unwrap();
        assert!(project.join(".a/skills/x/SKILL.md").is_file());

        let library_md = tmp.path().join("library/x/SKILL.md");
        fs::write(&library_md, "---\nname: x\n---\nnew\n").unwrap();
        update_vendored_x(&store, &record).unwrap();

        let vendored = project.join(".agents/skills/x");
        assert!(fs::symlink_metadata(&vendored).unwrap().is_dir());
        assert_eq!(
            fs::read_to_string(vendored.join("SKILL.md")).unwrap(),
            "---\nname: x\n---\nnew\n"
        );
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
