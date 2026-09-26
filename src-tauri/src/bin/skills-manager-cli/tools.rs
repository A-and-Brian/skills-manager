//! `agents` (alias `tools`): list agents and switch them on or off globally.

use anyhow::{anyhow, bail};
use app_lib::commands::tools as tool_cmd;
use app_lib::core::{audit_log::AuditDraft, skill_store::SkillStore, tool_service};

use crate::args::{ToolsArgs, ToolsCommand};
use crate::output::{map_app_err, print_json};
use crate::reports::AgentMutationReport;

pub(crate) fn run_tools(args: ToolsArgs, store: &SkillStore, json: bool) -> anyhow::Result<()> {
    match args.command {
        ToolsCommand::List => print_json(&tool_service::list_tool_info(store), json),
        ToolsCommand::Enable { agents } => {
            print_json(&run_set_agents_enabled(store, &agents, true)?, json)
        }
        ToolsCommand::Disable { agents } => {
            print_json(&run_set_agents_enabled(store, &agents, false)?, json)
        }
    }
    Ok(())
}

fn run_set_agents_enabled(
    store: &SkillStore,
    agents: &[String],
    enabled: bool,
) -> anyhow::Result<Vec<AgentMutationReport>> {
    if agents.is_empty() {
        bail!("no agent key provided");
    }
    let infos = tool_service::list_tool_info(store);
    let mut resolved = Vec::new();
    for key in agents {
        let info = infos
            .iter()
            .find(|info| info.key == *key)
            .ok_or_else(|| anyhow!("unknown agent: {key}"))?;
        if !resolved
            .iter()
            .any(|existing: &String| existing == &info.key)
        {
            resolved.push(info.key.clone());
        }
    }

    let mut reports = Vec::new();
    for key in resolved {
        let before = infos.iter().find(|info| info.key == key).unwrap().enabled;
        tool_cmd::set_tool_enabled_internal(store, &key, enabled).map_err(map_app_err)?;
        store.log_audit(
            AuditDraft::new(if enabled {
                "enable_agent"
            } else {
                "disable_agent"
            })
            .tool(key.clone())
            .ok(),
        );
        reports.push(AgentMutationReport {
            agent: key,
            enabled,
            changed: before != enabled,
        });
    }
    Ok(reports)
}
