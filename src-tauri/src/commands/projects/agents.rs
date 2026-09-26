//! Choosing the agents a project, or one of its skills, deploys to.

use std::path::Path;

use tauri::State;

use super::agents_model::{
    agent_skill_configs, effective_project_agent_keys, get_agent_selectable_project,
    plan_project_agent_change, read_workspace_skills, reconcile_skill_agents, validated_agent_keys,
};
use super::fs_safety::ensure_safe_skill_relative_path;
use crate::core::project_deploy::{self, AgentChangePlan, SkillOutcome};
use crate::core::skill_store::{ProjectRecord, SkillStore};
use crate::core::{error::AppError, host::HostCtx};

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

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::super::test_fixtures::{agent_selection_fixture, keys};
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::path::Path;
    #[cfg(unix)]
    use tempfile::tempdir;

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
}
