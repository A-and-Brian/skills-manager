//! Read-only skill views (`list`, `show`, `status`, `export`) and resolving a skill reference.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context};
use app_lib::core::{skill_store::SkillStore, sync_engine, tool_adapters, tool_service};

use crate::presets::resolve_scenario;
use crate::reports::{SkillAgentStatus, SkillDetail, SkillStatusReport, SkillSummary};

fn list_skills(store: &SkillStore) -> anyhow::Result<Vec<SkillSummary>> {
    let tags_map = store.get_tags_map()?;
    let targets = store.get_all_targets()?;
    let scenarios = store.get_all_scenarios()?;
    let scenario_lookup: std::collections::HashMap<String, String> =
        scenarios.into_iter().map(|s| (s.id, s.name)).collect();

    let mut items = Vec::new();
    for skill in store.get_all_skills()? {
        let preset_ids = store.get_scenarios_for_skill(&skill.id)?;
        let preset_names = preset_ids
            .iter()
            .filter_map(|id| scenario_lookup.get(id).cloned())
            .collect();
        let mut deployed_to: Vec<String> = targets
            .iter()
            .filter(|target| target.skill_id == skill.id && target.status == "ok")
            .map(|target| target.tool.clone())
            .collect();
        deployed_to.sort();
        deployed_to.dedup();
        items.push(SkillSummary {
            id: skill.id.clone(),
            name: skill.name.clone(),
            description: skill.description.clone(),
            path: skill.central_path.clone(),
            enabled: skill.enabled,
            tags: tags_map.get(&skill.id).cloned().unwrap_or_default(),
            source_type: skill.source_type.clone(),
            source_ref: skill.source_ref.clone(),
            preset_ids,
            presets: preset_names,
            deployed_to,
        });
    }
    Ok(items)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn list_skills_filtered(
    store: &SkillStore,
    query: Option<&str>,
    tags: &[String],
    preset_ref: Option<&str>,
    deployed_to: Option<&str>,
    untagged: bool,
    no_preset: bool,
    source: Option<&str>,
) -> anyhow::Result<Vec<SkillSummary>> {
    let preset_id = preset_ref
        .map(|reference| resolve_scenario(store, reference).map(|preset| preset.id))
        .transpose()?;
    if let Some(agent) = deployed_to {
        if tool_adapters::find_adapter_with_store(store, agent).is_none() {
            bail!("unknown agent: {agent}");
        }
    }
    let query = query
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_lowercase);
    let source = source
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_lowercase);
    let wanted_tags: Vec<String> = tags
        .iter()
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty())
        .map(str::to_string)
        .collect();

    Ok(list_skills(store)?
        .into_iter()
        .filter(|skill| {
            query.as_ref().is_none_or(|needle| {
                skill.name.to_lowercase().contains(needle)
                    || skill
                        .description
                        .as_deref()
                        .unwrap_or_default()
                        .to_lowercase()
                        .contains(needle)
            })
        })
        .filter(|skill| wanted_tags.iter().all(|tag| skill.tags.contains(tag)))
        .filter(|skill| !untagged || skill.tags.is_empty())
        .filter(|skill| !no_preset || skill.preset_ids.is_empty())
        .filter(|skill| {
            preset_id
                .as_ref()
                .is_none_or(|id| skill.preset_ids.contains(id))
        })
        .filter(|skill| {
            deployed_to
                .as_ref()
                .is_none_or(|agent| skill.deployed_to.iter().any(|key| key == agent))
        })
        .filter(|skill| {
            source.as_ref().is_none_or(|needle| {
                skill.source_type.to_lowercase().contains(needle)
                    || skill
                        .source_ref
                        .as_deref()
                        .unwrap_or_default()
                        .to_lowercase()
                        .contains(needle)
            })
        })
        .collect())
}

pub(crate) fn show_skill(store: &SkillStore, reference: &str) -> anyhow::Result<SkillDetail> {
    let skill = resolve_skill(store, reference)?;

    let summary = list_skills(store)?
        .into_iter()
        .find(|item| item.id == skill.id)
        .ok_or_else(|| anyhow!("skill summary missing"))?;

    let skill_dir = PathBuf::from(&skill.central_path);
    let skill_file = [skill_dir.join("SKILL.md"), skill_dir.join("skill.md")]
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| anyhow!("no SKILL.md found for {}", skill.name))?;
    let markdown = std::fs::read_to_string(&skill_file)?;

    Ok(SkillDetail {
        summary,
        skill_file: skill_file.to_string_lossy().to_string(),
        files: collect_files(&skill_dir)?,
        markdown,
    })
}

pub(crate) fn skill_status(
    store: &SkillStore,
    reference: &str,
) -> anyhow::Result<SkillStatusReport> {
    let skill = resolve_skill(store, reference)?;
    let summary = list_skills(store)?
        .into_iter()
        .find(|item| item.id == skill.id)
        .ok_or_else(|| anyhow!("skill summary missing"))?;
    let targets = store.get_targets_for_skill(&skill.id)?;
    let mut agents: Vec<SkillAgentStatus> = tool_service::list_tool_info(store)
        .into_iter()
        .map(|agent| {
            let target = targets.iter().find(|target| target.tool == agent.key);
            SkillAgentStatus {
                key: agent.key,
                display_name: agent.display_name,
                installed: agent.installed,
                globally_enabled: agent.enabled,
                deployed: target.is_some_and(|target| target.status == "ok"),
                target_path: target.map(|target| target.target_path.clone()),
            }
        })
        .collect();
    let mut unregistered_targets: Vec<_> = targets
        .iter()
        .filter(|target| !agents.iter().any(|agent| agent.key == target.tool))
        .collect();
    unregistered_targets.sort_by(|left, right| left.tool.cmp(&right.tool));
    for target in unregistered_targets {
        agents.push(SkillAgentStatus {
            key: target.tool.clone(),
            display_name: target.tool.clone(),
            installed: false,
            globally_enabled: false,
            deployed: target.status == "ok",
            target_path: Some(target.target_path.clone()),
        });
    }
    Ok(SkillStatusReport {
        skill: summary,
        agents,
    })
}

pub(crate) fn resolve_skill_references(
    store: &SkillStore,
    references: &[String],
) -> anyhow::Result<Vec<app_lib::core::skill_store::SkillRecord>> {
    if references.is_empty() {
        bail!("no skill ref provided");
    }
    let mut skills = Vec::new();
    for reference in references {
        let skill = resolve_skill(store, reference)?;
        if !skills
            .iter()
            .any(|existing: &app_lib::core::skill_store::SkillRecord| existing.id == skill.id)
        {
            skills.push(skill);
        }
    }
    Ok(skills)
}

pub(crate) fn export_skill(
    store: &SkillStore,
    reference: &str,
    dest: &Path,
    force: bool,
) -> anyhow::Result<String> {
    let skill = resolve_skill(store, reference)?;
    let source = PathBuf::from(&skill.central_path);

    // `dest` is an arbitrary user-supplied path, so an unguarded export is a
    // recursive delete of whatever they typed (#363) — `--dest ~/Documents`
    // used to wipe it and leave a SKILL.md. Nothing at an export destination
    // is ever "ours", so overwriting has to be asked for explicitly.
    if !force {
        let state = sync_engine::classify_target(dest, Some(&source))
            .with_context(|| format!("Cannot inspect export destination {}", dest.display()))?;
        if state != sync_engine::TargetState::Absent {
            bail!(
                "Export destination {} already exists; refusing to overwrite it. \
                 Choose a path that does not exist, or pass --force to replace it.",
                dest.display()
            );
        }
    }

    let policy = if force {
        sync_engine::ReplacePolicy::UserConfirmed
    } else {
        sync_engine::ReplacePolicy::NoClobber
    };
    sync_engine::sync_skill(&source, dest, sync_engine::SyncMode::Copy, policy)?;
    Ok(dest.to_string_lossy().to_string())
}

pub(crate) fn resolve_skill(
    store: &SkillStore,
    reference: &str,
) -> anyhow::Result<app_lib::core::skill_store::SkillRecord> {
    let matches: Vec<_> = store
        .get_all_skills()?
        .into_iter()
        .filter(|skill| {
            skill.id == reference
                || skill.name == reference
                || skill.central_path == reference
                || Path::new(&skill.central_path)
                    .file_name()
                    .and_then(|v| v.to_str())
                    == Some(reference)
        })
        .collect();

    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        0 => Err(anyhow!("skill not found: {reference}")),
        _ => Err(anyhow!("skill reference is ambiguous: {reference}")),
    }
}

fn collect_files(root: &Path) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    collect_files_inner(root, root, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_files_inner(root: &Path, current: &Path, out: &mut Vec<String>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files_inner(root, &path, out)?;
        } else {
            out.push(path.strip_prefix(root)?.to_string_lossy().to_string());
        }
    }
    Ok(())
}
