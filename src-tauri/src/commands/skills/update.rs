//! Updating and reimporting skills from their sources.

use tauri::State;

use super::types::{BatchUpdateSkillsResult, CancelRegistrationGuard};

use crate::core::{
    audit_log::AuditDraft,
    error::AppError,
    host::HostCtx,
    skill_store::SkillStore,
    skill_update::{
        reimport_local_skill_internal, update_git_skill_internal, ReimportSkillResult,
        UpdateSkillResult,
    },
};

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
