//! `skills deploy`/`undeploy` with verification, deprecated `enable`/`disable`, legacy `sync`.

use std::path::Path;

use anyhow::{anyhow, bail};
use app_lib::core::{
    audit_log::AuditDraft, scenario_service, skill_store::SkillStore, sync_engine, sync_metadata,
};

use crate::output::map_app_err;
use crate::presets::{resolve_scenario, select_agent_keys_for_removal, select_preset_agents};
use crate::reports::{
    DeploymentVerification, DeprecatedEnableReport, SkillDeploymentReport, SyncReport,
};
use crate::skills::list::{resolve_skill, resolve_skill_references};

pub(crate) fn run_skill_deployment(
    store: &SkillStore,
    references: &[String],
    requested_agents: &[String],
    deploy: bool,
    dry_run: bool,
) -> anyhow::Result<SkillDeploymentReport> {
    let skills = resolve_skill_references(store, references)?;
    if requested_agents.is_empty() {
        bail!("no agent key provided");
    }
    let existing_targets = store.get_all_targets()?;
    let skill_ids: Vec<String> = skills.iter().map(|skill| skill.id.clone()).collect();
    let agent_keys = if deploy {
        select_preset_agents(store, requested_agents, true)?
            .into_iter()
            .map(|agent| agent.key)
            .collect()
    } else {
        select_agent_keys_for_removal(store, requested_agents, &skill_ids, &existing_targets)?
    };
    let pair_count = skills.len() * agent_keys.len();
    let existing: std::collections::HashSet<(String, String)> = existing_targets
        .iter()
        .filter(|target| !deploy || target.status == "ok")
        .map(|target| (target.skill_id.clone(), target.tool.clone()))
        .collect();
    let changed: std::collections::HashSet<(String, String)> = skills
        .iter()
        .flat_map(|skill| {
            agent_keys
                .iter()
                .map(move |agent| (skill.id.clone(), agent.clone()))
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
        for skill in &skills {
            for agent in &agent_keys {
                if verification
                    .succeeded
                    .contains(&(skill.id.clone(), agent.clone()))
                    && changed.contains(&(skill.id.clone(), agent.clone()))
                {
                    store.log_audit(
                        AuditDraft::new(if deploy { "deploy" } else { "undeploy" })
                            .skill(skill.id.clone(), skill.name.clone())
                            .tool(agent.clone())
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

    Ok(SkillDeploymentReport {
        ok: true,
        action: if deploy { "deploy" } else { "undeploy" }.to_string(),
        agents: agent_keys,
        dry_run,
        skill_count: skills.len(),
        pair_count,
        changed_pairs,
        skills: skills.into_iter().map(|skill| skill.name).collect(),
        preserved,
    })
}

pub(crate) fn verify_deployment_state(
    store: &SkillStore,
    skill_ids: &[String],
    agent_keys: &[String],
    deployed: bool,
    previous_targets: &[app_lib::core::skill_store::SkillTargetRecord],
) -> anyhow::Result<DeploymentVerification> {
    let current_targets = store.get_all_targets()?;
    let mut failures = Vec::new();
    let mut preserved = Vec::new();
    let mut succeeded = std::collections::HashSet::new();

    for skill_id in skill_ids {
        for agent_key in agent_keys {
            let current = current_targets
                .iter()
                .find(|target| target.skill_id == *skill_id && target.tool == *agent_key);
            if deployed {
                match current.filter(|target| target.status == "ok") {
                    Some(target) => {
                        if let Err(error) = std::fs::symlink_metadata(&target.target_path) {
                            failures.push(format!(
                                "{skill_id}@{agent_key}: target is missing ({error})"
                            ));
                        } else {
                            succeeded.insert((skill_id.clone(), agent_key.clone()));
                        }
                    }
                    None => {
                        failures.push(format!("{skill_id}@{agent_key}: target was not created"))
                    }
                }
                continue;
            }

            if current.is_some() {
                failures.push(format!(
                    "{skill_id}@{agent_key}: target record still exists"
                ));
                continue;
            }

            let mut pair_succeeded = true;
            for previous in previous_targets
                .iter()
                .filter(|target| target.skill_id == *skill_id && target.tool == *agent_key)
            {
                let still_referenced = current_targets
                    .iter()
                    .any(|target| target.target_path == previous.target_path);
                if !still_referenced {
                    match std::fs::symlink_metadata(&previous.target_path) {
                        Ok(_) => {
                            // A path that survived undeploy is a failure only
                            // if it is still our deployment. If something else
                            // took it over, keeping it was the correct call and
                            // reporting it as a failure would train users to
                            // ignore the warning (#363).
                            let preserved_deliberately = !sync_engine::matches_recorded_deployment(
                                Path::new(&previous.target_path),
                                &previous.mode,
                            )
                            .unwrap_or(true);
                            if preserved_deliberately {
                                preserved.push(previous.target_path.clone());
                            } else {
                                pair_succeeded = false;
                                failures.push(format!(
                                    "{skill_id}@{agent_key}: target path still exists"
                                ));
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            pair_succeeded = false;
                            failures.push(format!(
                                "{skill_id}@{agent_key}: cannot verify removal ({error})"
                            ));
                        }
                    }
                }
            }
            if pair_succeeded {
                succeeded.insert((skill_id.clone(), agent_key.clone()));
            }
        }
    }

    Ok(DeploymentVerification {
        succeeded,
        failures,
        preserved,
    })
}

// ── enable / disable ──────────────────────────────────────────────────────

pub(crate) fn run_deprecated_set_enabled(
    store: &SkillStore,
    references: &[String],
    requested_enabled: bool,
) -> anyhow::Result<Vec<DeprecatedEnableReport>> {
    if references.is_empty() {
        bail!("no skill ref provided");
    }
    let mut reports = Vec::new();
    for r in references {
        let skill = resolve_skill(store, r)?;
        // `skills enable` repairs legacy enabled=false rows; `skills disable`
        // is a true no-op. Flipping enabled to true on disable would be the
        // opposite of what the user asked for.
        let changed = if requested_enabled && !skill.enabled {
            store.update_skill_enabled(&skill.id, true)?;
            true
        } else {
            false
        };
        let enabled_after = if requested_enabled {
            true
        } else {
            skill.enabled
        };
        let message = if requested_enabled {
            "Deprecated compatibility command: use `skills deploy --agent <key>` to make a skill available to an agent."
        } else {
            "Deprecated compatibility command: use `skills undeploy --agent <key>` to remove a skill from an agent."
        };
        reports.push(DeprecatedEnableReport {
            skill_id: skill.id,
            name: skill.name,
            enabled: enabled_after,
            changed,
            deprecated: true,
            message: message.to_string(),
        });
    }
    if reports.iter().any(|report| report.changed) {
        sync_metadata::write_all_from_db(store)?;
    }
    Ok(reports)
}

// ── sync ──────────────────────────────────────────────────────────────────

pub(crate) fn run_sync(
    store: &SkillStore,
    preset_ref: Option<&str>,
    tool_key: Option<&str>,
    dry_run: bool,
) -> anyhow::Result<SyncReport> {
    let preset = match preset_ref {
        Some(s) => resolve_scenario(store, s)?,
        None => {
            let active = store
                .get_active_scenario_id()?
                .ok_or_else(|| anyhow!("no active preset; pass --preset"))?;
            store
                .get_all_scenarios()?
                .into_iter()
                .find(|s| s.id == active)
                .ok_or_else(|| anyhow!("active preset not found"))?
        }
    };

    let preview =
        scenario_service::preview_scenario_sync(store, &preset.id).map_err(map_app_err)?;

    let filtered: Vec<_> = if let Some(t) = tool_key {
        preview.into_iter().filter(|p| p.tool == t).collect()
    } else {
        preview
    };

    if dry_run {
        return Ok(SyncReport {
            ok: true,
            preset_id: preset.id,
            preset_name: preset.name,
            tool: tool_key.map(|s| s.to_string()),
            dry_run: true,
            targets: filtered,
        });
    }

    // Make preset active if it isn't, then sync.
    let active = store.get_active_scenario_id()?;
    if active.as_deref() != Some(preset.id.as_str()) {
        store.set_active_scenario(&preset.id)?;
    }

    if let Some(t) = tool_key {
        // Build targets locally and filter to the requested tool so we don't
        // fan out to every enabled adapter (which is what
        // sync_active_scenario_to_tool ends up doing via
        // sync_skill_to_active_scenario).
        let all_targets = scenario_service::collect_scenario_sync_targets(store, &preset.id)
            .map_err(map_app_err)?;
        let desired: Vec<_> = all_targets.into_iter().filter(|tg| tg.tool == t).collect();
        let refusals =
            scenario_service::sync_desired_targets(store, &desired).map_err(map_app_err)?;
        scenario_service::refusals_to_error(refusals).map_err(map_app_err)?;
    } else {
        let refusals =
            scenario_service::apply_scenario_to_default(store, &preset.id).map_err(map_app_err)?;
        scenario_service::refusals_to_error(refusals).map_err(map_app_err)?;
    }

    Ok(SyncReport {
        ok: true,
        preset_id: preset.id,
        preset_name: preset.name,
        tool: tool_key.map(|s| s.to_string()),
        dry_run: false,
        targets: filtered,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::{preset_status, run_preset_deployment};
    use crate::skills::list::skill_status;
    use app_lib::core::skill_store::{ScenarioRecord, SkillRecord};
    use app_lib::core::tool_adapters::{CustomToolDef, ToolCategory};
    use app_lib::core::tool_service;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn skill_and_preset_deployment_round_trip() {
        let tmp = tempdir().unwrap();
        let store = SkillStore::new(&tmp.path().join("skills.db")).unwrap();
        let source = tmp.path().join("central/demo");
        let target_root = tmp.path().join("agent-skills");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target_root).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: demo\ndescription: test skill\n---\n",
        )
        .unwrap();
        fs::write(source.join("payload.txt"), "managed").unwrap();

        let test_agent = CustomToolDef {
            key: "test_agent".to_string(),
            display_name: "Test Agent".to_string(),
            skills_dir: target_root.to_string_lossy().to_string(),
            project_relative_skills_dir: None,
            category: ToolCategory::Coding,
        };
        tool_service::set_custom_tools(&store, std::slice::from_ref(&test_agent)).unwrap();
        store.set_setting("sync_mode", "copy").unwrap();
        store
            .insert_skill(&SkillRecord {
                id: "skill-demo".to_string(),
                name: "demo".to_string(),
                description: Some("test skill".to_string()),
                source_type: "local".to_string(),
                source_ref: Some(source.to_string_lossy().to_string()),
                source_ref_resolved: None,
                source_subpath: None,
                source_branch: None,
                source_revision: None,
                remote_revision: None,
                central_path: source.to_string_lossy().to_string(),
                content_hash: None,
                enabled: true,
                created_at: 1,
                updated_at: 1,
                status: "ok".to_string(),
                update_status: "local_only".to_string(),
                last_checked_at: None,
                last_check_error: None,
            })
            .unwrap();

        let dry_run = run_skill_deployment(
            &store,
            &["demo".to_string()],
            &["test_agent".to_string()],
            true,
            true,
        )
        .unwrap();
        assert_eq!(dry_run.changed_pairs, 1);
        assert!(!target_root.join("demo").exists());
        assert!(store.get_all_targets().unwrap().is_empty());

        let deployed = run_skill_deployment(
            &store,
            &["demo".to_string()],
            &["test_agent".to_string()],
            true,
            false,
        )
        .unwrap();
        assert_eq!(deployed.changed_pairs, 1);
        assert_eq!(
            fs::read_to_string(target_root.join("demo/payload.txt")).unwrap(),
            "managed"
        );
        let status = skill_status(&store, "demo").unwrap();
        assert!(status
            .agents
            .iter()
            .any(|agent| agent.key == "test_agent" && agent.deployed));

        let dry_remove = run_skill_deployment(
            &store,
            &["demo".to_string()],
            &["test_agent".to_string()],
            false,
            true,
        )
        .unwrap();
        assert_eq!(dry_remove.changed_pairs, 1);
        assert!(target_root.join("demo").exists());

        run_skill_deployment(
            &store,
            &["demo".to_string()],
            &["test_agent".to_string()],
            false,
            false,
        )
        .unwrap();
        assert!(!target_root.join("demo").exists());
        assert!(store.get_all_targets().unwrap().is_empty());
        let audit_count = store.list_audit(None).unwrap().len();
        let noop_remove = run_skill_deployment(
            &store,
            &["demo".to_string()],
            &["test_agent".to_string()],
            false,
            false,
        )
        .unwrap();
        assert_eq!(noop_remove.changed_pairs, 0);
        assert_eq!(store.list_audit(None).unwrap().len(), audit_count);

        store
            .insert_scenario(&ScenarioRecord {
                id: "preset-web".to_string(),
                name: "Web Dev".to_string(),
                description: None,
                icon: None,
                sort_order: 0,
                created_at: 1,
                updated_at: 1,
            })
            .unwrap();
        store
            .add_skill_to_scenario("preset-web", "skill-demo")
            .unwrap();

        let deployed =
            run_preset_deployment(&store, "Web Dev", &["test_agent".to_string()], true, false)
                .unwrap();
        assert_eq!(deployed.changed_pairs, 1);
        let status = preset_status(&store, "Web Dev", &["test_agent".to_string()]).unwrap();
        assert_eq!(status.agents[0].status, "active");

        store
            .set_setting(
                "disabled_tools",
                &serde_json::to_string(&vec!["test_agent"]).unwrap(),
            )
            .unwrap();
        let status = preset_status(&store, "Web Dev", &[]).unwrap();
        assert!(status
            .agents
            .iter()
            .any(|agent| agent.key == "test_agent" && agent.status == "active"));

        tool_service::set_custom_tools(&store, &[]).unwrap();
        let status = skill_status(&store, "demo").unwrap();
        assert!(status
            .agents
            .iter()
            .any(|agent| { agent.key == "test_agent" && agent.deployed && !agent.installed }));

        run_preset_deployment(&store, "Web Dev", &[], false, false).unwrap();
        tool_service::set_custom_tools(&store, &[test_agent]).unwrap();
        let status = preset_status(&store, "Web Dev", &["test_agent".to_string()]).unwrap();
        assert_eq!(status.agents[0].status, "inactive");
        assert!(!target_root.join("demo").exists());

        store.set_setting("disabled_tools", "[]").unwrap();
        let missing_source = tmp.path().join("central/broken");
        store
            .insert_skill(&SkillRecord {
                id: "skill-broken".to_string(),
                name: "broken".to_string(),
                description: Some("missing source".to_string()),
                source_type: "local".to_string(),
                source_ref: Some(missing_source.to_string_lossy().to_string()),
                source_ref_resolved: None,
                source_subpath: None,
                source_branch: None,
                source_revision: None,
                remote_revision: None,
                central_path: missing_source.to_string_lossy().to_string(),
                content_hash: None,
                enabled: true,
                created_at: 1,
                updated_at: 1,
                status: "ok".to_string(),
                update_status: "local_only".to_string(),
                last_checked_at: None,
                last_check_error: None,
            })
            .unwrap();
        let audit_count = store.list_audit(None).unwrap().len();
        let error = run_skill_deployment(
            &store,
            &["demo".to_string(), "broken".to_string()],
            &["test_agent".to_string()],
            true,
            false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("deployment incomplete"));
        assert!(target_root.join("demo").exists());
        assert!(!target_root.join("broken").exists());
        let audit = store.list_audit(None).unwrap();
        assert_eq!(audit.len(), audit_count + 1);
        assert_eq!(audit[0].action, "deploy");
        assert_eq!(audit[0].skill_id.as_deref(), Some("skill-demo"));
    }
}
