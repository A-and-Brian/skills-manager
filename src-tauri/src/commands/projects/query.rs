//! Reading a project: its agent targets, its skills and a skill's document.

use std::collections::HashSet;
use std::path::PathBuf;

use tauri::State;

use super::agents_model::{
    agent_skill_configs, project_agent_targets_for_record, read_workspace_skills,
    resolve_agent_skills_roots,
};
use super::fs_safety::{ensure_dir_within_root, ensure_safe_skill_relative_path};
use super::types::{ProjectAgentTargetDto, ProjectSkillDocumentDto};
use crate::core::project_skill_match::{classify_sync_status, find_best_center_match};
use crate::core::{error::AppError, host::HostCtx, project_scanner};

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
