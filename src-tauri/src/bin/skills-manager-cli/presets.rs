//! `presets` (stored as scenarios): membership, additive deploy and status, legacy apply.

use anyhow::{anyhow, bail};
use app_lib::commands::presets as preset_cmd;
use app_lib::core::{
    audit_log::AuditDraft, scenario_service, skill_store::SkillStore, tool_adapters, tool_service,
};

use crate::args::{PresetArgs, PresetCommand};
use crate::output::{map_app_err, print_json};
use crate::reports::{
    PresetAgentStatus, PresetDeactivateReport, PresetDeleteReport, PresetDeploymentReport,
    PresetInfo, PresetMembershipReport, PresetStatusReport,
};
use crate::skills::deploy::verify_deployment_state;
use crate::skills::list::resolve_skill_references;

pub(crate) fn run_presets(args: PresetArgs, store: &SkillStore, json: bool) -> anyhow::Result<()> {
    match args.command {
        PresetCommand::List => print_json(&list_presets(store)?, json),
        PresetCommand::Current => print_json(&current_preset(store)?, json),
        PresetCommand::Show { reference } => {
            let preset = resolve_scenario(store, &reference)?;
            print_json(&preset_info_for(store, preset)?, json);
        }
        PresetCommand::Create {
            name,
            description,
            icon,
        } => {
            let preset = preset_cmd::create_preset_internal(
                store,
                &name,
                description.as_deref(),
                icon.as_deref(),
            )
            .map_err(map_app_err)?;
            print_json(&preset_info_for(store, preset)?, json);
        }
        PresetCommand::Update {
            reference,
            name,
            description,
            icon,
        } => {
            if name.is_none() && description.is_none() && icon.is_none() {
                bail!("pass at least one of --name, --description, or --icon");
            }
            let preset = resolve_scenario(store, &reference)?;
            let next_name = name.unwrap_or_else(|| preset.name.clone());
            let next_description = match description {
                Some(value) if value.trim().is_empty() => None,
                Some(value) => Some(value),
                None => preset.description.clone(),
            };
            let next_icon = match icon {
                Some(value) if value.trim().is_empty() => None,
                Some(value) => Some(value),
                None => preset.icon.clone(),
            };
            preset_cmd::update_preset_internal(
                store,
                &preset.id,
                &next_name,
                next_description.as_deref(),
                next_icon.as_deref(),
            )
            .map_err(map_app_err)?;
            let updated = resolve_scenario(store, &preset.id)?;
            print_json(&preset_info_for(store, updated)?, json);
        }
        PresetCommand::Delete {
            reference,
            yes,
            dry_run,
        } => {
            let preset = resolve_scenario(store, &reference)?;
            if !dry_run && !yes {
                bail!("refusing to delete preset without --yes");
            }
            if !dry_run {
                preset_cmd::delete_preset_internal(store, &preset.id).map_err(map_app_err)?;
            }
            print_json(
                &PresetDeleteReport {
                    ok: true,
                    preset_id: preset.id,
                    preset_name: preset.name,
                    dry_run,
                    deleted: !dry_run,
                },
                json,
            );
        }
        PresetCommand::Preview { reference } => {
            let preset = resolve_scenario(store, &reference)?;
            let preview =
                scenario_service::preview_scenario_sync(store, &preset.id).map_err(map_app_err)?;
            print_json(&preview, json);
        }
        PresetCommand::Apply { reference } => {
            let preset = resolve_scenario(store, &reference)?;
            let refusals = scenario_service::apply_scenario_to_default(store, &preset.id)
                .map_err(map_app_err)?;
            scenario_service::refusals_to_error(refusals).map_err(map_app_err)?;
            print_json(&current_preset(store)?, json);
        }
        PresetCommand::Deactivate { reference } => {
            let preset = resolve_scenario(store, &reference)?;
            let active = store.get_active_scenario_id()?;
            let is_active = active.as_deref() == Some(preset.id.as_str());
            let count_before = count_synced_targets_for_preset(store, &preset.id)?;

            if is_active {
                let next_active = replacement_preset_after_deactivate(store, &preset.id)?;
                if let Some(next) = next_active.as_ref() {
                    for refusal in scenario_service::apply_scenario_to_default(store, &next.id)
                        .map_err(map_app_err)?
                    {
                        eprintln!("warning: {refusal}");
                    }
                } else {
                    scenario_service::unsync_scenario_skills(store, &preset.id)
                        .map_err(map_app_err)?;
                    store.clear_active_scenario()?;
                }
            } else {
                // Closing a non-active preset still tears down sync targets for
                // any skills it shares with the active preset. Unsync this
                // preset first, then re-sync the active preset so the shared
                // targets are restored.
                scenario_service::unsync_scenario_skills(store, &preset.id).map_err(map_app_err)?;
                if let Some(active_id) = active.as_deref() {
                    // The delete already happened; a refusal here must not fail
                    // the command, only be reported.
                    for refusal in scenario_service::sync_scenario_skills(store, active_id)
                        .map_err(map_app_err)?
                    {
                        eprintln!("warning: {refusal}");
                    }
                }
            }

            let count_after = count_synced_targets_for_preset(store, &preset.id)?;
            let removed_target_count = count_before.saturating_sub(count_after);

            let active_after = current_preset(store)?;
            print_json(
                &PresetDeactivateReport {
                    ok: true,
                    preset_id: preset.id,
                    preset_name: preset.name,
                    removed_target_count,
                    active_preset_id: active_after.as_ref().map(|preset| preset.id.clone()),
                    active_preset_name: active_after.map(|preset| preset.name),
                },
                json,
            );
        }
        PresetCommand::Deploy {
            reference,
            agents,
            dry_run,
        } => {
            let report = run_preset_deployment(store, &reference, &agents, true, dry_run)?;
            print_json(&report, json);
        }
        PresetCommand::Undeploy {
            reference,
            agents,
            dry_run,
        } => {
            let report = run_preset_deployment(store, &reference, &agents, false, dry_run)?;
            print_json(&report, json);
        }
        PresetCommand::Status { reference, agents } => {
            print_json(&preset_status(store, &reference, &agents)?, json);
        }
        PresetCommand::AddSkill { preset, skills } => {
            let s = resolve_scenario(store, &preset)?;
            let resolved = resolve_skill_references(store, &skills)?;
            let ids: Vec<String> = resolved.iter().map(|skill| skill.id.clone()).collect();
            preset_cmd::set_preset_skills_internal(store, &s.id, &ids, true)
                .map_err(map_app_err)?;
            print_json(
                &PresetMembershipReport {
                    preset_id: s.id,
                    preset_name: s.name,
                    added: resolved.into_iter().map(|skill| skill.name).collect(),
                    removed: Vec::new(),
                    missing: Vec::new(),
                },
                json,
            );
        }
        PresetCommand::RemoveSkill { preset, skills } => {
            let s = resolve_scenario(store, &preset)?;
            let resolved = resolve_skill_references(store, &skills)?;
            let ids: Vec<String> = resolved.iter().map(|skill| skill.id.clone()).collect();
            preset_cmd::set_preset_skills_internal(store, &s.id, &ids, false)
                .map_err(map_app_err)?;
            print_json(
                &PresetMembershipReport {
                    preset_id: s.id,
                    preset_name: s.name,
                    added: Vec::new(),
                    removed: resolved.into_iter().map(|skill| skill.name).collect(),
                    missing: Vec::new(),
                },
                json,
            );
        }
    }
    Ok(())
}

fn preset_info_for(
    store: &SkillStore,
    preset: app_lib::core::skill_store::ScenarioRecord,
) -> anyhow::Result<PresetInfo> {
    let active = store.get_active_scenario_id()?;
    Ok(PresetInfo {
        skill_count: store.get_skill_ids_for_scenario(&preset.id)?.len(),
        active: active.as_deref() == Some(preset.id.as_str()),
        id: preset.id,
        name: preset.name,
        description: preset.description,
        icon: preset.icon,
        sort_order: preset.sort_order,
    })
}

pub(crate) fn select_preset_agents(
    store: &SkillStore,
    requested: &[String],
    require_available: bool,
) -> anyhow::Result<Vec<tool_service::ToolInfo>> {
    let infos = tool_service::list_tool_info(store);
    if requested.is_empty() {
        return Ok(infos
            .into_iter()
            .filter(|agent| {
                agent.installed
                    && agent.enabled
                    && matches!(agent.category, tool_adapters::ToolCategory::Coding)
            })
            .collect());
    }

    let mut selected = Vec::new();
    for key in requested {
        let agent = infos
            .iter()
            .find(|agent| agent.key == *key)
            .ok_or_else(|| anyhow!("unknown agent: {key}"))?;
        if require_available && !agent.installed {
            bail!("agent is not installed: {}", agent.display_name);
        }
        if require_available && !agent.enabled {
            bail!("agent is disabled: {}", agent.display_name);
        }
        if !selected
            .iter()
            .any(|existing: &tool_service::ToolInfo| existing.key == agent.key)
        {
            selected.push(agent.clone());
        }
    }
    Ok(selected)
}

pub(crate) fn select_agent_keys_for_removal(
    store: &SkillStore,
    requested: &[String],
    skill_ids: &[String],
    existing_targets: &[app_lib::core::skill_store::SkillTargetRecord],
) -> anyhow::Result<Vec<String>> {
    let deployed_keys: std::collections::HashSet<String> = existing_targets
        .iter()
        .filter(|target| skill_ids.contains(&target.skill_id))
        .map(|target| target.tool.clone())
        .collect();
    if requested.is_empty() {
        let mut keys: Vec<String> = deployed_keys.into_iter().collect();
        keys.sort();
        return Ok(keys);
    }

    let known_keys: std::collections::HashSet<String> = tool_service::list_tool_info(store)
        .into_iter()
        .map(|agent| agent.key)
        .collect();
    let mut selected = Vec::new();
    for key in requested {
        if !known_keys.contains(key) && !deployed_keys.contains(key) {
            bail!("unknown agent: {key}");
        }
        if !selected.contains(key) {
            selected.push(key.clone());
        }
    }
    Ok(selected)
}

pub(crate) fn preset_status(
    store: &SkillStore,
    reference: &str,
    requested_agents: &[String],
) -> anyhow::Result<PresetStatusReport> {
    let preset = resolve_scenario(store, reference)?;
    let preset_info = preset_info_for(store, preset.clone())?;
    let skill_ids = store.get_skill_ids_for_scenario(&preset.id)?;
    let all_targets = store.get_all_targets()?;
    let targets: std::collections::HashSet<(String, String)> = all_targets
        .iter()
        .filter(|target| target.status == "ok")
        .map(|target| (target.skill_id.clone(), target.tool.clone()))
        .collect();
    let infos = tool_service::list_tool_info(store);
    let agent_keys = if requested_agents.is_empty() {
        let mut keys: Vec<String> = infos
            .iter()
            .filter(|agent| {
                agent.installed
                    && agent.enabled
                    && matches!(agent.category, tool_adapters::ToolCategory::Coding)
            })
            .map(|agent| agent.key.clone())
            .collect();
        for target in all_targets
            .iter()
            .filter(|target| target.status == "ok" && skill_ids.contains(&target.skill_id))
        {
            if !keys.contains(&target.tool) {
                keys.push(target.tool.clone());
            }
        }
        keys
    } else {
        select_agent_keys_for_removal(store, requested_agents, &skill_ids, &all_targets)?
    };
    let agents = agent_keys
        .into_iter()
        .map(|agent_key| {
            let deployed = skill_ids
                .iter()
                .filter(|skill_id| targets.contains(&((*skill_id).clone(), agent_key.clone())))
                .count();
            let total = skill_ids.len();
            let status = if total == 0 {
                "empty"
            } else if deployed == 0 {
                "inactive"
            } else if deployed == total {
                "active"
            } else {
                "partial"
            };
            let display_name = infos
                .iter()
                .find(|agent| agent.key == agent_key)
                .map(|agent| agent.display_name.clone())
                .unwrap_or_else(|| agent_key.clone());
            PresetAgentStatus {
                key: agent_key,
                display_name,
                deployed,
                total,
                status: status.to_string(),
            }
        })
        .collect();
    Ok(PresetStatusReport {
        preset: preset_info,
        agents,
    })
}

pub(crate) fn run_preset_deployment(
    store: &SkillStore,
    reference: &str,
    requested_agents: &[String],
    deploy: bool,
    dry_run: bool,
) -> anyhow::Result<PresetDeploymentReport> {
    let preset = resolve_scenario(store, reference)?;
    let skill_ids = store.get_skill_ids_for_scenario(&preset.id)?;
    let existing_targets = store.get_all_targets()?;
    let agent_keys = if deploy {
        select_preset_agents(store, requested_agents, true)?
            .into_iter()
            .map(|agent| agent.key)
            .collect()
    } else {
        select_agent_keys_for_removal(store, requested_agents, &skill_ids, &existing_targets)?
    };
    if deploy && agent_keys.is_empty() {
        bail!("no enabled, installed coding agents found");
    }
    let pair_count = skill_ids.len() * agent_keys.len();
    let existing: std::collections::HashSet<(String, String)> = existing_targets
        .iter()
        .filter(|target| !deploy || target.status == "ok")
        .map(|target| (target.skill_id.clone(), target.tool.clone()))
        .collect();
    let changed: std::collections::HashSet<(String, String)> = skill_ids
        .iter()
        .flat_map(|skill_id| {
            agent_keys
                .iter()
                .map(move |agent| (skill_id.clone(), agent.clone()))
        })
        .filter(|pair| {
            let present = existing.contains(pair);
            if deploy {
                !present
            } else {
                present
            }
        })
        .collect();
    let changed_pairs = changed.len();

    let mut preserved: Vec<String> = Vec::new();
    if !dry_run {
        scenario_service::apply_skills_to_tools(
            store,
            &skill_ids,
            &agent_keys,
            if deploy {
                scenario_service::BatchApplyMode::Add
            } else {
                scenario_service::BatchApplyMode::Remove
            },
        )
        .map_err(map_app_err)?;
        let verification =
            verify_deployment_state(store, &skill_ids, &agent_keys, deploy, &existing_targets)?;
        for skill_id in &skill_ids {
            let skill = store
                .get_skill_by_id(skill_id)?
                .ok_or_else(|| anyhow!("skill missing"))?;
            for agent in &agent_keys {
                if verification
                    .succeeded
                    .contains(&(skill.id.clone(), agent.clone()))
                    && changed.contains(&(skill.id.clone(), agent.clone()))
                {
                    store.log_audit(
                        AuditDraft::new(if deploy {
                            "deploy_preset"
                        } else {
                            "undeploy_preset"
                        })
                        .skill(skill.id.clone(), skill.name.clone())
                        .tool(agent.clone())
                        .detail(format!("preset={} ({})", preset.name, preset.id))
                        .ok(),
                    );
                }
            }
        }
        preserved = verification.preserved.clone();
        if !verification.failures.is_empty() {
            bail!(
                "deployment incomplete: {} pair(s) verified, {} verification issue(s): {}",
                verification.succeeded.len(),
                verification.failures.len(),
                verification.failures.join("; ")
            );
        }
    }

    Ok(PresetDeploymentReport {
        ok: true,
        action: if deploy { "deploy" } else { "undeploy" }.to_string(),
        preset_id: preset.id,
        preset_name: preset.name,
        agents: agent_keys,
        dry_run,
        skill_count: skill_ids.len(),
        pair_count,
        changed_pairs,
        preserved,
    })
}

fn list_presets(store: &SkillStore) -> anyhow::Result<Vec<PresetInfo>> {
    let active = store.get_active_scenario_id()?;
    let scenarios = store.get_all_scenarios()?;
    Ok(scenarios
        .into_iter()
        .map(|scenario| PresetInfo {
            skill_count: store
                .get_skill_ids_for_scenario(&scenario.id)
                .unwrap_or_default()
                .len(),
            active: active.as_deref() == Some(scenario.id.as_str()),
            id: scenario.id,
            name: scenario.name,
            description: scenario.description,
            icon: scenario.icon,
            sort_order: scenario.sort_order,
        })
        .collect())
}

fn current_preset(store: &SkillStore) -> anyhow::Result<Option<PresetInfo>> {
    let scenarios = list_presets(store)?;
    Ok(scenarios.into_iter().find(|s| s.active))
}

fn count_synced_targets_for_preset(store: &SkillStore, preset_id: &str) -> anyhow::Result<usize> {
    let skill_ids = store.get_skill_ids_for_scenario(preset_id)?;
    let mut count = 0;
    for skill_id in skill_ids {
        count += store.get_targets_for_skill(&skill_id)?.len();
    }
    Ok(count)
}

fn replacement_preset_after_deactivate(
    store: &SkillStore,
    deactivated_id: &str,
) -> anyhow::Result<Option<app_lib::core::skill_store::ScenarioRecord>> {
    let scenarios = store.get_all_scenarios()?;
    Ok(scenarios
        .into_iter()
        .find(|scenario| scenario.id != deactivated_id))
}

pub(crate) fn resolve_scenario(
    store: &SkillStore,
    reference: &str,
) -> anyhow::Result<app_lib::core::skill_store::ScenarioRecord> {
    let scenarios = store.get_all_scenarios()?;
    if reference == "current" {
        let active = store
            .get_active_scenario_id()?
            .ok_or_else(|| anyhow!("no active preset"))?;
        return scenarios
            .into_iter()
            .find(|scenario| scenario.id == active)
            .ok_or_else(|| anyhow!("active preset not found"));
    }
    let matches: Vec<_> = scenarios
        .into_iter()
        .filter(|s| s.id == reference || s.name == reference)
        .collect();
    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        0 => Err(anyhow!("preset not found: {reference}")),
        _ => Err(anyhow!("preset reference is ambiguous: {reference}")),
    }
}
