//! Enabling, disabling and deleting one agent's copy of a project skill.

use std::path::Path;

use tauri::State;

use super::agents_model::{agent_skill_configs, resolve_agent_skills_roots, skill_has_any_copy};
use super::fs_safety::{
    ensure_dir_within_root, ensure_safe_skill_relative_path, remove_workspace_skill_target,
    set_project_skill_enabled_state,
};
use crate::core::project_deploy;
use crate::core::project_scanner::VENDORED_SKILLS_DIR;
use crate::core::skill_store::{ProjectRecord, SkillStore};
use crate::core::{error::AppError, host::HostCtx};

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

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::super::agents_model::agent_skill_configs;
    #[cfg(unix)]
    use super::super::deploy_mode::convert_project_to_copy;
    #[cfg(unix)]
    use super::super::test_fixtures::{agent_selection_fixture, update_vendored_x};
    #[cfg(unix)]
    use super::{delete_skill_copy, toggle_skill_copy};
    #[cfg(unix)]
    use crate::core::error::ErrorKind;
    #[cfg(unix)]
    use crate::core::project_deploy;
    #[cfg(unix)]
    use crate::core::skill_store::{ProjectRecord, SkillStore};
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::path::Path;
    #[cfg(unix)]
    use tempfile::tempdir;

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
