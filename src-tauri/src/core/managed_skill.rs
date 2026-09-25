//! The managed-skill view the UI and CLI read.

use serde::Serialize;
use std::path::Path;

use crate::core::{
    error::AppError,
    skill_metadata,
    skill_store::{SkillRecord, SkillStore, SkillTargetRecord},
};

#[derive(Debug, Serialize)]
pub struct ManagedSkillDto {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// SKILL.md frontmatter author, when the skill names one.
    pub author: Option<String>,
    pub source_type: String,
    pub source_ref: Option<String>,
    pub source_ref_resolved: Option<String>,
    pub source_subpath: Option<String>,
    pub source_branch: Option<String>,
    pub source_revision: Option<String>,
    pub remote_revision: Option<String>,
    pub update_status: String,
    pub last_checked_at: Option<i64>,
    pub last_check_error: Option<String>,
    pub central_path: String,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub status: String,
    pub targets: Vec<TargetDto>,
    pub preset_ids: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TargetDto {
    pub id: String,
    pub skill_id: String,
    pub tool: String,
    pub target_path: String,
    pub mode: String,
    pub status: String,
    pub synced_at: Option<i64>,
}

pub(crate) fn managed_skill_to_dto(
    store: &SkillStore,
    skill: SkillRecord,
    all_targets: &[SkillTargetRecord],
    tags_map: &std::collections::HashMap<String, Vec<String>>,
) -> ManagedSkillDto {
    let targets = all_targets
        .iter()
        .filter(|target| target.skill_id == skill.id)
        .map(|target| TargetDto {
            id: target.id.clone(),
            skill_id: target.skill_id.clone(),
            tool: target.tool.clone(),
            target_path: target.target_path.clone(),
            mode: target.mode.clone(),
            status: target.status.clone(),
            synced_at: target.synced_at,
        })
        .collect();

    let preset_ids = store.get_scenarios_for_skill(&skill.id).unwrap_or_default();
    let tags = tags_map.get(&skill.id).cloned().unwrap_or_default();

    // Prefer description from SKILL.md so the list view reflects edits made
    // directly on disk (file watcher emits a change event; this read serves
    // the fresh value). Keep `name` on the DB value to avoid drift with
    // sync target directory names.
    let meta = skill_metadata::parse_skill_md(Path::new(&skill.central_path));
    let description = meta
        .description
        .filter(|s| !s.trim().is_empty())
        .or(skill.description);

    ManagedSkillDto {
        id: skill.id,
        name: skill.name,
        description,
        author: meta.author,
        source_type: skill.source_type,
        source_ref: skill.source_ref,
        source_ref_resolved: skill.source_ref_resolved,
        source_subpath: skill.source_subpath,
        source_branch: skill.source_branch,
        source_revision: skill.source_revision,
        remote_revision: skill.remote_revision,
        update_status: skill.update_status,
        last_checked_at: skill.last_checked_at,
        last_check_error: skill.last_check_error,
        central_path: skill.central_path,
        enabled: skill.enabled,
        created_at: skill.created_at,
        updated_at: skill.updated_at,
        status: skill.status,
        targets,
        preset_ids,
        tags,
    }
}

pub fn managed_skill_by_id(
    store: &SkillStore,
    skill_id: &str,
) -> Result<ManagedSkillDto, AppError> {
    let skill = store
        .get_skill_by_id(skill_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Skill not found"))?;
    let all_targets = store.get_all_targets().map_err(AppError::db)?;
    let tags_map = store.get_tags_map().map_err(AppError::db)?;
    Ok(managed_skill_to_dto(store, skill, &all_targets, &tags_map))
}
