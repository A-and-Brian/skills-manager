//! `skills update` and its dry-run partner `skills check`.

use anyhow::bail;
use app_lib::core::{skill_store::SkillStore, skill_update, skill_update_check};

use crate::reports::{CheckReport, UpdateReport};
use crate::skills::list::resolve_skill;

pub(crate) fn run_update(
    store: &SkillStore,
    reference: Option<&str>,
    all: bool,
) -> anyhow::Result<Vec<UpdateReport>> {
    let targets = select_skill_ids(store, reference, all)?;
    let proxy_url = store.proxy_url();
    let mut reports = Vec::new();

    for skill in targets {
        let report = match skill.source_type.as_str() {
            "git" | "skillssh" => {
                match skill_update::update_git_skill_internal(
                    store,
                    &skill.id,
                    proxy_url.as_deref(),
                    None,
                    None,
                ) {
                    Ok(r) => UpdateReport {
                        skill_id: skill.id.clone(),
                        name: skill.name.clone(),
                        source_type: skill.source_type.clone(),
                        refreshed: r.content_changed,
                        error: None,
                        held_back_removals: r
                            .pending_removals
                            .iter()
                            .map(|p| format!("{}: {}", p.location, p.path))
                            .collect(),
                    },
                    Err(e) => UpdateReport {
                        skill_id: skill.id.clone(),
                        name: skill.name.clone(),
                        source_type: skill.source_type.clone(),
                        refreshed: false,
                        error: Some(e.message.clone()),
                        held_back_removals: Vec::new(),
                    },
                }
            }
            "local" | "import" => {
                match skill_update::reimport_local_skill_internal(store, &skill.id, None) {
                    Ok(r) => UpdateReport {
                        skill_id: skill.id.clone(),
                        name: skill.name.clone(),
                        source_type: skill.source_type.clone(),
                        refreshed: r.pending_removals.is_empty(),
                        error: None,
                        held_back_removals: r
                            .pending_removals
                            .iter()
                            .map(|p| format!("{}: {}", p.location, p.path))
                            .collect(),
                    },
                    Err(e) => UpdateReport {
                        skill_id: skill.id.clone(),
                        name: skill.name.clone(),
                        source_type: skill.source_type.clone(),
                        refreshed: false,
                        error: Some(e.message.clone()),
                        held_back_removals: Vec::new(),
                    },
                }
            }
            other => UpdateReport {
                skill_id: skill.id.clone(),
                name: skill.name.clone(),
                source_type: skill.source_type.clone(),
                refreshed: false,
                error: Some(format!("source type '{other}' cannot be refreshed")),
                held_back_removals: Vec::new(),
            },
        };
        reports.push(report);
    }

    Ok(reports)
}

pub(crate) fn run_check(
    store: &SkillStore,
    reference: Option<&str>,
    all: bool,
    force: bool,
) -> anyhow::Result<Vec<CheckReport>> {
    let targets = select_skill_ids(store, reference, all)?;
    let proxy_url = store.proxy_url();
    let mut reports = Vec::new();

    for skill in targets {
        if !matches!(skill.source_type.as_str(), "git" | "skillssh") {
            reports.push(CheckReport {
                skill_id: skill.id.clone(),
                name: skill.name.clone(),
                source_type: skill.source_type.clone(),
                update_status: skill.update_status.clone(),
                last_check_error: skill.last_check_error.clone(),
                skipped: true,
            });
            continue;
        }
        let report = match skill_update_check::check_skill_update_internal(
            store,
            &skill.id,
            force,
            proxy_url.as_deref(),
        ) {
            Ok(dto) => CheckReport {
                skill_id: dto.id,
                name: dto.name,
                source_type: dto.source_type,
                update_status: dto.update_status,
                last_check_error: dto.last_check_error,
                skipped: false,
            },
            Err(e) => CheckReport {
                skill_id: skill.id.clone(),
                name: skill.name.clone(),
                source_type: skill.source_type.clone(),
                update_status: "error".to_string(),
                last_check_error: Some(e.message.clone()),
                skipped: false,
            },
        };
        reports.push(report);
    }

    Ok(reports)
}

fn select_skill_ids(
    store: &SkillStore,
    reference: Option<&str>,
    all: bool,
) -> anyhow::Result<Vec<app_lib::core::skill_store::SkillRecord>> {
    if let Some(r) = reference {
        if all {
            bail!("pass either a ref or --all, not both");
        }
        Ok(vec![resolve_skill(store, r)?])
    } else if all {
        Ok(store.get_all_skills()?)
    } else {
        bail!("pass a skill ref or --all")
    }
}
