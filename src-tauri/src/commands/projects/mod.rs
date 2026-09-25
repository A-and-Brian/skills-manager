use std::path::Path;

use tauri::State;

use crate::core::project_deploy::{
    self, AgentChangePlan, ConversionOutcome, SkillConversion, SkillOutcome,
};
use crate::core::project_scanner::VENDORED_SKILLS_DIR;
use crate::core::skill_store::{ProjectRecord, SkillStore};
use crate::core::{error::AppError, host::HostCtx};

mod types;
pub use types::*;

mod agents_model;
use agents_model::*;

mod fs_safety;
use fs_safety::*;
pub(crate) use fs_safety::{ensure_dir_within_root, ensure_safe_skill_relative_path};

mod crud;
pub use crud::*;

mod query;
pub use query::*;

mod center_sync;
pub use center_sync::*;

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
    use crate::core::error::ErrorKind;
    #[cfg(unix)]
    use crate::core::project_deploy::{self, SkipReason};
    use crate::core::skill_store::{ProjectRecord, SkillStore};
    use std::fs;
    use std::path::Path;
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
}
