//! Switching a project between linking skills and vendoring copies of them.

use std::path::Path;

use tauri::State;

use super::agents_model::{agent_skill_configs, get_deploy_mode_project, read_workspace_skills};
use crate::core::project_deploy::{self, ConversionOutcome, SkillConversion};
use crate::core::project_scanner::VENDORED_SKILLS_DIR;
use crate::core::skill_store::{ProjectRecord, SkillStore};
use crate::core::{error::AppError, host::HostCtx};

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
pub(super) fn convert_project_to_copy(
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
mod tests {
    #[cfg(unix)]
    use super::super::agents_model::{agent_skill_configs, plan_project_agent_change};
    #[cfg(unix)]
    use super::super::test_fixtures::{agent_selection_fixture, keys};
    #[cfg(unix)]
    use super::convert_project_to_copy;
    #[cfg(unix)]
    use crate::core::project_deploy::SkipReason;
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::path::Path;
    #[cfg(unix)]
    use tempfile::tempdir;

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
}
