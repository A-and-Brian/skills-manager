//! Tag writes shared by the GUI commands and the CLI.

use crate::core::{error::AppError, skill_store::SkillStore, sync_metadata};

/// Shared implementation for GUI and CLI tag writes. Keeping the DB row and
/// its backup metadata under one repo lock prevents another process from
/// reindexing the half-written state between those two operations.
pub fn set_skill_tags_internal(
    store: &SkillStore,
    skill_id: &str,
    tags: &[String],
) -> Result<(), AppError> {
    let mut normalized = Vec::new();
    for tag in tags {
        let tag = tag.trim();
        if !tag.is_empty() && !normalized.iter().any(|existing| existing == tag) {
            normalized.push(tag.to_string());
        }
    }

    sync_metadata::with_repo_lock("set skill tags", || {
        store.set_tags_for_skill(skill_id, &normalized)?;
        sync_metadata::ensure_skill_metadata_unlocked(store, skill_id)
    })
    .map_err(AppError::db)
}

pub fn rename_tag_internal(
    store: &SkillStore,
    old_name: &str,
    new_name: &str,
) -> Result<Vec<String>, AppError> {
    let old_name = old_name.trim();
    let new_name = new_name.trim();
    if old_name.is_empty() || new_name.is_empty() {
        return Err(AppError::invalid_input("Tag name cannot be empty"));
    }
    if new_name == old_name {
        return Ok(Vec::new());
    }
    sync_metadata::with_repo_lock("rename tag", || {
        let affected = store.rename_tag(old_name, new_name)?;
        for skill_id in &affected {
            sync_metadata::ensure_skill_metadata_unlocked(store, skill_id)?;
        }
        Ok(affected)
    })
    .map_err(AppError::db)
}

pub fn delete_tag_internal(store: &SkillStore, name: &str) -> Result<Vec<String>, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::invalid_input("Tag name cannot be empty"));
    }
    sync_metadata::with_repo_lock("delete tag", || {
        let affected = store.delete_tag(name)?;
        for skill_id in &affected {
            sync_metadata::ensure_skill_metadata_unlocked(store, skill_id)?;
        }
        Ok(affected)
    })
    .map_err(AppError::db)
}
