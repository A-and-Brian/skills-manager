//! `skills`: the subcommand dispatcher. Each group of subcommands lives in its own module.

use app_lib::core::{skill_source, skill_store::SkillStore};

use crate::args::{SkillsArgs, SkillsCommand};
use crate::output::{map_app_err, print_json};
use crate::{
    classify_ref, export_skill, list_skills_filtered, resolve_skill, run_adopt, run_check,
    run_deprecated_set_enabled, run_install, run_remove, run_search, run_skill_deployment,
    run_sync, run_tag, run_update, show_skill, skill_status, SyncTarget,
};

pub(crate) fn run_skills(args: SkillsArgs, store: &SkillStore, json: bool) -> anyhow::Result<()> {
    match args.command {
        SkillsCommand::List {
            query,
            tags,
            preset,
            deployed_to,
            untagged,
            no_preset,
            source,
        } => print_json(
            &list_skills_filtered(
                store,
                query.as_deref(),
                &tags,
                preset.as_deref(),
                deployed_to.as_deref(),
                untagged,
                no_preset,
                source.as_deref(),
            )?,
            json,
        ),
        SkillsCommand::Show { reference } => print_json(&show_skill(store, &reference)?, json),
        SkillsCommand::Export {
            reference,
            dest,
            force,
        } => {
            let result = export_skill(store, &reference, &dest, force)?;
            print_json(
                &serde_json::json!({"ok": true, "destination": result}),
                json,
            );
        }
        SkillsCommand::Install {
            reference,
            local,
            git,
            skillssh,
            name,
            sync,
            sync_preset,
        } => {
            let kind = classify_ref(&reference, local, git, skillssh)?;
            let sync_target = if let Some(ref s) = sync_preset {
                SyncTarget::Specific(s.clone())
            } else if sync {
                SyncTarget::Active
            } else {
                SyncTarget::None
            };
            let report = run_install(store, &reference, name.as_deref(), kind, sync_target)?;
            print_json(&report, json);
        }
        SkillsCommand::Update { reference, all } => {
            let reports = run_update(store, reference.as_deref(), all)?;
            print_json(&reports, json);
        }
        SkillsCommand::Check {
            reference,
            all,
            force,
        } => {
            let reports = run_check(store, reference.as_deref(), all, force)?;
            print_json(&reports, json);
        }
        SkillsCommand::Remove {
            references,
            yes,
            dry_run,
        } => {
            let report = run_remove(store, &references, yes, dry_run)?;
            print_json(&report, json);
        }
        SkillsCommand::Enable { references } => {
            let reports = run_deprecated_set_enabled(store, &references, true)?;
            print_json(&reports, json);
        }
        SkillsCommand::Disable { references } => {
            let reports = run_deprecated_set_enabled(store, &references, false)?;
            print_json(&reports, json);
        }
        SkillsCommand::Deploy {
            references,
            agents,
            dry_run,
        } => {
            let report = run_skill_deployment(store, &references, &agents, true, dry_run)?;
            print_json(&report, json);
        }
        SkillsCommand::Undeploy {
            references,
            agents,
            dry_run,
        } => {
            let report = run_skill_deployment(store, &references, &agents, false, dry_run)?;
            print_json(&report, json);
        }
        SkillsCommand::Status { reference } => {
            print_json(&skill_status(store, &reference)?, json);
        }
        SkillsCommand::Sync {
            preset,
            tool,
            dry_run,
        } => {
            let report = run_sync(store, preset.as_deref(), tool.as_deref(), dry_run)?;
            print_json(&report, json);
        }
        SkillsCommand::Search { query, limit } => {
            let hits = run_search(store, &query, limit)?;
            print_json(&hits, json);
        }
        SkillsCommand::SetSource {
            reference,
            git_url,
            subpath,
            branch,
            force,
            dry_run,
        } => {
            let skill = resolve_skill(store, &reference)?;
            let report = skill_source::set_git_source_internal(
                store,
                &skill.id,
                &git_url,
                subpath.as_deref(),
                branch.as_deref(),
                store.proxy_url().as_deref(),
                force,
                dry_run,
            )
            .map_err(map_app_err)?;
            print_json(&report, json);
        }
        SkillsCommand::Adopt {
            paths,
            git_url,
            git_subpath,
            dry_run,
        } => {
            let report = run_adopt(
                store,
                &paths,
                git_url.as_deref(),
                git_subpath.as_deref(),
                dry_run,
            )?;
            print_json(&report, json);
        }
        SkillsCommand::Tag(args) => run_tag(args, store, json)?,
    }
    Ok(())
}
