//! How a project's agents are modelled: which agent groups it has, which it
//! deploys to, and planning a change to them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::types::ProjectAgentTargetDto;
use crate::core::project_deploy::{self, AgentChangePlan, SkillOutcome};
use crate::core::project_skill_match::{find_best_center_match, slugify_skill_dir_name};
use crate::core::skill_store::{ProjectRecord, SkillRecord, SkillStore};
use crate::core::{error::AppError, project_scanner, tool_adapters};

pub(super) fn agent_skill_configs(store: &SkillStore) -> Vec<project_scanner::AgentSkillConfig> {
    let mut grouped: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for adapter in tool_adapters::all_tool_adapters(store) {
        let project_dir = adapter.project_relative_skills_dir().to_string();
        if project_dir.is_empty() {
            continue;
        }
        if let Some((_, agents)) = grouped.iter_mut().find(|(dir, _)| *dir == project_dir) {
            agents.push((adapter.key, adapter.display_name));
        } else {
            grouped.push((project_dir, vec![(adapter.key, adapter.display_name)]));
        }
    }

    grouped
        .into_iter()
        .filter_map(|(relative_skills_dir, agents)| {
            let (key, first_display_name) = agents.first()?.clone();
            let display_name = if agents.len() == 1 {
                first_display_name
            } else {
                agents
                    .into_iter()
                    .map(|(_, display_name)| display_name)
                    .collect::<Vec<_>>()
                    .join(" / ")
            };
            Some(project_scanner::AgentSkillConfig {
                key,
                display_name,
                relative_skills_dir,
            })
        })
        .collect()
}

fn linked_workspace_agent_key(rec: &ProjectRecord) -> String {
    rec.linked_agent_key
        .clone()
        .unwrap_or_else(|| slugify_skill_dir_name(&rec.name))
}

fn linked_workspace_agent_name(rec: &ProjectRecord) -> String {
    rec.linked_agent_name
        .clone()
        .unwrap_or_else(|| rec.name.clone())
}

pub(super) fn read_workspace_skills(
    rec: &ProjectRecord,
    configs: &[project_scanner::AgentSkillConfig],
) -> Vec<project_scanner::ProjectSkillInfo> {
    if rec.workspace_type == "linked" {
        return project_scanner::read_linked_workspace_skills(
            Path::new(&rec.path),
            rec.disabled_path.as_deref().map(Path::new),
            &linked_workspace_agent_key(rec),
            &linked_workspace_agent_name(rec),
            true,
        );
    }
    project_scanner::read_project_skills(Path::new(&rec.path), configs)
}

/// Resolve the enabled and disabled skills root directories for a given agent in a workspace.
pub(super) fn resolve_agent_skills_roots(
    store: &SkillStore,
    rec: &ProjectRecord,
    agent: &str,
) -> Option<(PathBuf, Option<PathBuf>)> {
    if rec.workspace_type == "linked" {
        if linked_workspace_agent_key(rec) != agent {
            return None;
        }
        return Some((
            PathBuf::from(&rec.path),
            rec.disabled_path.as_ref().map(PathBuf::from),
        ));
    }

    let adapter = tool_adapters::all_tool_adapters(store)
        .into_iter()
        .find(|adapter| adapter.key == agent)?;
    let project_dir = adapter.project_relative_skills_dir();
    let skills_root = Path::new(&rec.path).join(project_dir);
    let disabled_root = Path::new(&rec.path).join(format!("{}-disabled", project_dir));
    Some((skills_root, Some(disabled_root)))
}

pub(super) fn project_agent_targets_for_record(
    store: &SkillStore,
    rec: &ProjectRecord,
) -> Vec<ProjectAgentTargetDto> {
    if rec.workspace_type == "linked" {
        return vec![ProjectAgentTargetDto {
            key: linked_workspace_agent_key(rec),
            display_name: linked_workspace_agent_name(rec),
            enabled: true,
            installed: true,
            is_custom: false,
            selected: true,
            relative_skills_dir: rec.path.clone(),
        }];
    }

    let disabled_tools: std::collections::HashSet<String> = store
        .get_setting("disabled_tools")
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
        .unwrap_or_default()
        .into_iter()
        .collect();

    agent_skill_configs(store)
        .into_iter()
        .map(|config| {
            let adapter = tool_adapters::find_adapter_with_store(store, &config.key);
            let enabled = !disabled_tools.contains(&config.key);
            let installed = adapter.as_ref().map(|a| a.is_installed()).unwrap_or(false);
            // A project that never chose uses every agent it can deploy to.
            let selected = match &rec.agent_keys {
                Some(keys) => keys.contains(&config.key),
                None => installed && enabled,
            };
            ProjectAgentTargetDto {
                enabled,
                installed,
                is_custom: adapter.as_ref().map(|a| a.is_custom).unwrap_or(false),
                selected,
                key: config.key,
                display_name: config.display_name,
                relative_skills_dir: config.relative_skills_dir,
            }
        })
        .collect()
}

/// Agents that are installed and enabled, the only ones a deployment can reach.
fn available_agent_keys(store: &SkillStore, rec: &ProjectRecord) -> HashSet<String> {
    project_agent_targets_for_record(store, rec)
        .into_iter()
        .filter(|target| target.installed && target.enabled)
        .map(|target| target.key)
        .collect()
}

/// Agents a project deploys to when none are named: its selection, limited
/// to agents that are installed and enabled.
pub(super) fn effective_project_agent_keys(store: &SkillStore, rec: &ProjectRecord) -> Vec<String> {
    project_agent_targets_for_record(store, rec)
        .into_iter()
        .filter(|target| target.selected && target.installed && target.enabled)
        .map(|target| target.key)
        .collect()
}

/// The agents an export writes to: the requested ones, or the project's own
/// selection when none are requested, limited to agents it can reach.
pub(super) fn export_agent_keys(
    store: &SkillStore,
    project: &ProjectRecord,
    requested: Option<Vec<String>>,
) -> Result<Vec<String>, AppError> {
    let requested = requested
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| effective_project_agent_keys(store, project));
    if project.workspace_type == "linked" {
        return Ok(requested);
    }
    let available = available_agent_keys(store, project);
    let filtered = requested
        .into_iter()
        .filter(|key| available.contains(key))
        .collect::<Vec<_>>();
    // A copy-mode project vendors the skill even with no agent to link.
    if filtered.is_empty() && !is_copy_project(project) {
        return Err(AppError::invalid_input(
            "No enabled installed agents selected for this project",
        ));
    }
    Ok(filtered)
}

pub(super) fn is_copy_project(rec: &ProjectRecord) -> bool {
    rec.deploy_mode == "copy"
}

/// Load a project for a deploy-mode command. Linked workspaces are one
/// agent's own folder and have nothing to vendor into.
pub(super) fn get_deploy_mode_project(
    store: &SkillStore,
    project_id: &str,
) -> Result<ProjectRecord, AppError> {
    let record = store
        .get_project_by_id(project_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Workspace not found"))?;
    if record.workspace_type == "linked" {
        return Err(AppError::invalid_input(
            "Linked workspaces have no deploy mode",
        ));
    }
    Ok(record)
}

/// The scanned vendored copy that `skill` stands for: itself, or the one it
/// reads as a link into `.agents/skills`. `None` for any other copy, which is
/// handled on its own. Decided from the disk, not the deploy mode: vendored
/// copies stay vendored after a project switches back to linking.
pub(super) fn vendored_variant<'a>(
    rec: &ProjectRecord,
    skills: &'a [project_scanner::ProjectSkillInfo],
    skill: &project_scanner::ProjectSkillInfo,
) -> Option<&'a project_scanner::ProjectSkillInfo> {
    if rec.workspace_type == "linked" {
        return None;
    }
    let relative_path = skill.alias_of.as_deref().unwrap_or(&skill.relative_path);
    let vendored = project_deploy::vendored_copy(Path::new(&rec.path), relative_path)?;
    if skill.alias_of.is_none() && Path::new(&skill.path) != vendored {
        return None;
    }
    skills
        .iter()
        .find(|scanned| Path::new(&scanned.path) == vendored)
}

/// Load a project for an agent-selection command. Linked workspaces have a
/// single agent, so there is nothing to select.
pub(super) fn get_agent_selectable_project(
    store: &SkillStore,
    project_id: &str,
) -> Result<ProjectRecord, AppError> {
    let record = store
        .get_project_by_id(project_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Workspace not found"))?;
    if record.workspace_type == "linked" {
        return Err(AppError::invalid_input(
            "Linked workspaces have a single agent and no agent selection",
        ));
    }
    Ok(record)
}

/// Check requested agents are project agent groups, dropping duplicates.
pub(super) fn validated_agent_keys(
    configs: &[project_scanner::AgentSkillConfig],
    agent_keys: Vec<String>,
) -> Result<Vec<String>, AppError> {
    let mut keys: Vec<String> = Vec::new();
    for key in agent_keys {
        if !configs.iter().any(|config| config.key == key) {
            return Err(AppError::invalid_input(format!("Unknown agent: {key}")));
        }
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    Ok(keys)
}

fn library_source(
    skill: &project_scanner::ProjectSkillInfo,
    all_managed: &[SkillRecord],
) -> Option<PathBuf> {
    find_best_center_match(skill, all_managed).map(|managed| PathBuf::from(&managed.central_path))
}

pub(super) fn plan_project_agent_change(
    store: &SkillStore,
    rec: &ProjectRecord,
    configs: &[project_scanner::AgentSkillConfig],
    desired: &[String],
) -> Result<AgentChangePlan, AppError> {
    let skills = read_workspace_skills(rec, configs);
    let all_managed = store.get_all_skills().map_err(AppError::db)?;
    let overrides = store
        .get_project_skill_agent_overrides(&rec.id)
        .map_err(AppError::db)?;
    let available = available_agent_keys(store, rec);
    let source_of = |skill: &project_scanner::ProjectSkillInfo| library_source(skill, &all_managed);
    Ok(if is_copy_project(rec) {
        project_deploy::plan_copy_agent_change(
            Path::new(&rec.path),
            configs,
            &skills,
            desired,
            &available,
            &overrides,
            source_of,
        )
    } else {
        project_deploy::plan_agent_change(
            Path::new(&rec.path),
            configs,
            &skills,
            desired,
            &available,
            &overrides,
            source_of,
        )
    })
}

/// Put one skill on exactly `desired`, the single-skill counterpart of a
/// bulk agent change.
pub(super) fn reconcile_skill_agents(
    store: &SkillStore,
    rec: &ProjectRecord,
    relative_path: &str,
    desired: &[String],
    remove_real_dirs: bool,
) -> Result<SkillOutcome, AppError> {
    let configs = agent_skill_configs(store);
    let skills = read_workspace_skills(rec, &configs);
    let variants: Vec<&project_scanner::ProjectSkillInfo> = skills
        .iter()
        .filter(|skill| skill.relative_path.eq_ignore_ascii_case(relative_path))
        .collect();
    if variants.is_empty() {
        return Err(AppError::not_found("Skill not found in workspace"));
    }
    let all_managed = store.get_all_skills().map_err(AppError::db)?;
    let source = variants
        .iter()
        .find_map(|variant| library_source(variant, &all_managed));
    let available = available_agent_keys(store, rec);
    // A skill vendored before a switch back to linking stays vendored.
    let vendored = is_copy_project(rec)
        || project_deploy::vendored_copy(Path::new(&rec.path), relative_path).is_some();
    let change = if vendored {
        project_deploy::plan_copy_skill_change(
            &configs,
            &variants,
            desired,
            &available,
            source,
            remove_real_dirs,
        )
    } else {
        project_deploy::plan_skill_change(&variants, desired, &available, source, remove_real_dirs)
    };
    let configured_mode = store.get_setting("sync_mode").map_err(AppError::db)?;
    let mut outcomes = project_deploy::apply_agent_change(
        Path::new(&rec.path),
        &configs,
        &[change],
        configured_mode.as_deref(),
        remove_real_dirs,
    );
    Ok(outcomes.remove(0))
}

/// Whether any agent still holds a copy of the skill, enabled or disabled.
pub(super) fn skill_has_any_copy(
    configs: &[project_scanner::AgentSkillConfig],
    project_root: &Path,
    relative_path: &str,
) -> bool {
    configs.iter().any(|config| {
        [
            project_root.join(&config.relative_skills_dir),
            project_root.join(format!("{}-disabled", config.relative_skills_dir)),
        ]
        .iter()
        .any(|root| std::fs::symlink_metadata(root.join(relative_path)).is_ok())
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_fixtures::{agent_selection_fixture, keys};
    use super::export_agent_keys;
    #[cfg(unix)]
    use super::{agent_skill_configs, reconcile_skill_agents, skill_has_any_copy};
    use crate::core::error::ErrorKind;
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn export_without_named_agents_uses_the_project_selection() {
        let tmp = tempdir().unwrap();
        let (store, mut record) = agent_selection_fixture(tmp.path(), Some(keys(&["agent_b"])));

        assert_eq!(
            export_agent_keys(&store, &record, None).unwrap(),
            keys(&["agent_b"])
        );
        assert_eq!(
            export_agent_keys(&store, &record, Some(keys(&["agent_a"]))).unwrap(),
            keys(&["agent_a"])
        );

        // A project that never chose keeps every available agent.
        record.agent_keys = None;
        let legacy = export_agent_keys(&store, &record, None).unwrap();
        assert!(legacy.contains(&"agent_a".to_string()));
        assert!(legacy.contains(&"agent_b".to_string()));

        record.agent_keys = Some(Vec::new());
        let err = export_agent_keys(&store, &record, None).unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidInput);
    }

    #[cfg(unix)]
    #[test]
    fn a_skill_follows_its_own_agents_then_returns_to_the_project_set() {
        let tmp = tempdir().unwrap();
        let (store, record) = agent_selection_fixture(tmp.path(), Some(keys(&["agent_a"])));
        let project = Path::new(&record.path);

        let outcome =
            reconcile_skill_agents(&store, &record, "x", &keys(&["agent_a", "agent_b"]), true)
                .unwrap();
        assert_eq!(outcome.added, keys(&["agent_b"]));
        assert!(project.join(".b/skills/x/SKILL.md").is_file());

        // What clearing the override does: back to the project's agents.
        let outcome =
            reconcile_skill_agents(&store, &record, "x", &keys(&["agent_a"]), false).unwrap();
        assert_eq!(outcome.removed, keys(&["agent_b"]));
        assert!(fs::symlink_metadata(project.join(".b/skills/x")).is_err());
        assert!(project.join(".a/skills/x/SKILL.md").is_file());

        let configs = agent_skill_configs(&store);
        assert!(skill_has_any_copy(&configs, project, "x"));
        fs::remove_file(project.join(".a/skills/x")).unwrap();
        assert!(!skill_has_any_copy(&configs, project, "x"));
        let err =
            reconcile_skill_agents(&store, &record, "x", &keys(&["agent_a"]), false).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
    }

    /// A copy-mode project with no agent to link still vendors.
    #[test]
    fn a_copy_project_exports_with_no_agents_selected() {
        let tmp = tempdir().unwrap();
        let (store, mut record) = agent_selection_fixture(tmp.path(), Some(Vec::new()));
        record.deploy_mode = "copy".to_string();

        assert!(export_agent_keys(&store, &record, None).unwrap().is_empty());
    }
}
