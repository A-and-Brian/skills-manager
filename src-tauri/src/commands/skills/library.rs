//! Deleting skills and editing tags.

use tauri::State;

use crate::core::{
    error::AppError,
    host::HostCtx,
    skill_delete::{delete_managed_skills_by_ids, BatchDeleteSkillsResult},
    skill_tags::{delete_tag_internal, rename_tag_internal, set_skill_tags_internal},
};

#[tauri::command]
pub async fn delete_managed_skill(
    skill_id: String,
    ctx: State<'_, HostCtx>,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || delete_managed_skill_core(&ctx, skill_id)).await?
}

pub fn delete_managed_skill_core(ctx: &HostCtx, skill_id: String) -> Result<(), AppError> {
    let store = ctx.store.clone();
    let result = delete_managed_skills_by_ids(&store, std::slice::from_ref(&skill_id))?;
    if result.deleted == 0 {
        return Err(AppError::not_found("Skill not found"));
    }
    Ok(())
}

#[tauri::command]
pub async fn delete_managed_skills(
    skill_ids: Vec<String>,
    ctx: State<'_, HostCtx>,
) -> Result<BatchDeleteSkillsResult, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || delete_managed_skills_core(&ctx, skill_ids))
        .await?
}

pub fn delete_managed_skills_core(
    ctx: &HostCtx,
    skill_ids: Vec<String>,
) -> Result<BatchDeleteSkillsResult, AppError> {
    let store = ctx.store.clone();
    delete_managed_skills_by_ids(&store, &skill_ids)
}

#[tauri::command]
pub async fn get_all_tags(ctx: State<'_, HostCtx>) -> Result<Vec<String>, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || get_all_tags_core(&ctx)).await?
}

pub fn get_all_tags_core(ctx: &HostCtx) -> Result<Vec<String>, AppError> {
    let store = ctx.store.clone();
    store.get_all_tags().map_err(AppError::db)
}

#[tauri::command]
pub async fn set_skill_tags(
    skill_id: String,
    tags: Vec<String>,
    ctx: State<'_, HostCtx>,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || set_skill_tags_core(&ctx, skill_id, tags)).await?
}

pub fn set_skill_tags_core(
    ctx: &HostCtx,
    skill_id: String,
    tags: Vec<String>,
) -> Result<(), AppError> {
    let store = ctx.store.clone();
    set_skill_tags_internal(&store, &skill_id, &tags)
}

/// Globally rename a tag across all skills (used by the tag filter bar). If the
/// new name already exists, the tags are merged.
#[tauri::command]
pub async fn rename_tag(
    old_name: String,
    new_name: String,
    ctx: State<'_, HostCtx>,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || rename_tag_core(&ctx, old_name, new_name)).await?
}

pub fn rename_tag_core(ctx: &HostCtx, old_name: String, new_name: String) -> Result<(), AppError> {
    let store = ctx.store.clone();
    rename_tag_internal(&store, &old_name, &new_name).map(|_| ())
}

/// Globally delete a tag from all skills (used by the tag filter bar).
#[tauri::command]
pub async fn delete_tag(name: String, ctx: State<'_, HostCtx>) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || delete_tag_core(&ctx, name)).await?
}

pub fn delete_tag_core(ctx: &HostCtx, name: String) -> Result<(), AppError> {
    let store = ctx.store.clone();
    delete_tag_internal(&store, &name).map(|_| ())
}
