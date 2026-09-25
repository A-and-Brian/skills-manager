use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context};
use app_lib::core::{
    app_state, audit_log::AuditDraft, central_repo, git_fetcher, installer, repo_lock::RepoLock,
    scenario_service, serve, skill_delete, skill_install, skill_metadata, skill_store::SkillStore,
    skill_tags, skill_update, skill_update_check, skillssh_api, sync_engine, sync_metadata,
    tool_adapters, tool_service,
};
use clap::{Args, Parser, Subcommand};

use crate::args::{GitArgs, PresetArgs, RepoArgs, SkillsArgs, TagArgs, TagCommand, ToolsArgs};
use crate::output::{error_envelope, map_app_err, print_json};
use crate::presets::{
    resolve_scenario, run_presets, select_agent_keys_for_removal, select_preset_agents,
};
use crate::repo::{run_git, run_repo};
use crate::reports::{
    AdoptCandidate, AdoptReport, CheckReport, DeploymentVerification, DeprecatedEnableReport,
    GlobalTagReport, InstallReport, RemoveReport, SearchHit, SkillAgentStatus,
    SkillDeploymentReport, SkillDetail, SkillStatusReport, SkillSummary, SyncReport, TagReport,
    UpdateReport,
};
use crate::skills::run_skills;
use crate::tools::run_tools;

mod args;
mod output;
mod presets;
mod repo;
mod reports;
mod skills;
mod tools;

#[derive(Parser, Debug)]
#[command(name = "skills-manager-cli")]
#[command(about = "Shared-core CLI for skills-manager", version)]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    skills_root: Option<PathBuf>,
    /// Use this folder as the whole Skills Manager base (library, database,
    /// settings) instead of the configured one. For hermetic tests.
    #[arg(long, global = true, hide = true, conflicts_with = "skills_root")]
    base_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Repo(RepoArgs),
    #[command(name = "agents", visible_alias = "tools")]
    Tools(ToolsArgs),
    Skills(SkillsArgs),
    #[command(alias = "scenarios")]
    Presets(PresetArgs),
    Git(GitArgs),
    /// Answer the app's commands over stdin/stdout, for a Skills Manager app
    /// on another machine connected over ssh.
    Serve(ServeArgs),
}

#[derive(Args, Debug)]
struct ServeArgs {
    /// Speak the protocol on stdin/stdout (the only transport).
    #[arg(long, required = true)]
    stdio: bool,
}

enum InstallKind {
    Local,
    Git,
    Skillssh,
}

enum SyncTarget {
    None,
    Active,
    Specific(String),
}

fn main() {
    let json = std::env::args()
        .skip(1)
        .take_while(|a| a != "--")
        .any(|a| a == "--json" || a.starts_with("--json="));

    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            if !e.use_stderr() {
                e.exit();
            }
            if json {
                let message = e.to_string();
                let envelope = serde_json::json!({
                    "ok": false,
                    "code": "INVALID_ARGUMENT",
                    "message": message,
                    "error": message,
                });
                eprintln!("{}", serde_json::to_string(&envelope).unwrap());
                std::process::exit(2);
            }
            e.exit();
        }
    };

    if let Err(err) = run(cli) {
        if json {
            eprintln!("{}", serde_json::to_string(&error_envelope(&err)).unwrap());
        } else {
            eprintln!("error: {err:#}");
        }
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    if let Some(skills_root) = &cli.skills_root {
        let base = central_repo::external_base_dir(skills_root);
        central_repo::set_runtime_base_dir_override(Some(base));
        central_repo::set_runtime_skills_dir_override(Some(skills_root.clone()));
    }
    if let Some(base_dir) = &cli.base_dir {
        central_repo::set_runtime_base_dir_override(Some(base_dir.clone()));
    }

    let store = app_state::initialize_cli_store()?;

    match cli.command {
        Commands::Repo(args) => run_repo(args, &store, cli.json),
        Commands::Tools(args) => run_tools(args, &store, cli.json),
        Commands::Skills(args) => run_skills(args, &store, cli.json),
        Commands::Presets(args) => run_presets(args, &store, cli.json),
        Commands::Git(args) => run_git(args, &store, cli.skills_root.is_some(), cli.json),
        // stdout carries the protocol; nothing else may print there.
        Commands::Serve(_) => Ok(serve::serve_stdio(store)?),
    }
}

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
fn list_skills_filtered(
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

fn show_skill(store: &SkillStore, reference: &str) -> anyhow::Result<SkillDetail> {
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

fn skill_status(store: &SkillStore, reference: &str) -> anyhow::Result<SkillStatusReport> {
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

fn resolve_skill_references(
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

fn run_skill_deployment(
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

fn verify_deployment_state(
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

fn export_skill(
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

fn resolve_skill(
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

// ── install ───────────────────────────────────────────────────────────────

fn classify_ref(
    reference: &str,
    force_local: bool,
    force_git: bool,
    force_skillssh: bool,
) -> anyhow::Result<InstallKind> {
    if force_local {
        return Ok(InstallKind::Local);
    }
    if force_git {
        return Ok(InstallKind::Git);
    }
    if force_skillssh {
        return Ok(InstallKind::Skillssh);
    }

    if reference.starts_with("./")
        || reference.starts_with("../")
        || reference.starts_with('/')
        || reference.starts_with("~/")
    {
        return Ok(InstallKind::Local);
    }

    if reference.contains("://") || reference.ends_with(".git") || reference.starts_with("git@") {
        return Ok(InstallKind::Git);
    }

    if is_skillssh_shorthand(reference) {
        return Ok(InstallKind::Skillssh);
    }

    bail!(
        "ambiguous ref '{}'; pass --local, --git, or --skillssh to disambiguate",
        reference
    )
}

fn is_skillssh_shorthand(s: &str) -> bool {
    // owner/repo, owner/repo/skill, owner/repo@skill
    fn seg_ok(s: &str) -> bool {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-'))
    }
    let (head, _at_skill) = match s.split_once('@') {
        Some((h, t)) if seg_ok(t) => (h, Some(t)),
        Some(_) => return false,
        None => (s, None),
    };
    let parts: Vec<&str> = head.split('/').collect();
    (parts.len() == 2 || parts.len() == 3) && parts.iter().all(|p| seg_ok(p))
}

fn resolve_sync_target(store: &SkillStore, target: &SyncTarget) -> anyhow::Result<Option<String>> {
    match target {
        SyncTarget::None => Ok(None),
        SyncTarget::Active => Ok(store.get_active_scenario_id()?),
        SyncTarget::Specific(ref_) => {
            let scenario = resolve_scenario(store, ref_)?;
            Ok(Some(scenario.id))
        }
    }
}

fn run_install(
    store: &SkillStore,
    reference: &str,
    name: Option<&str>,
    kind: InstallKind,
    sync: SyncTarget,
) -> anyhow::Result<InstallReport> {
    let preset_id = resolve_sync_target(store, &sync)?;
    let synced = preset_id.is_some();

    let (skill_id, install_name, central_path, source_type) = match kind {
        InstallKind::Local => install_local_action(store, reference, name, preset_id.as_deref())?,
        InstallKind::Git => install_git_action(store, reference, name, preset_id.as_deref())?,
        InstallKind::Skillssh => install_skillssh_action(store, reference, preset_id.as_deref())?,
    };

    Ok(InstallReport {
        ok: true,
        skill_id,
        name: install_name,
        central_path,
        source_type,
        synced,
        preset_id,
    })
}

fn install_local_action(
    store: &SkillStore,
    reference: &str,
    name: Option<&str>,
    active_scenario: Option<&str>,
) -> anyhow::Result<(String, String, String, String)> {
    let path = expand_path(reference)?;
    if !path.exists() {
        bail!("local path does not exist: {}", path.display());
    }

    let _lock = RepoLock::acquire_foreground("cli install local")?;
    let result = installer::install_from_local(&path, name)?;
    let metadata = skill_install::InstallSourceMetadata {
        source_type: "local".to_string(),
        source_ref: Some(path.to_string_lossy().to_string()),
        source_ref_resolved: None,
        source_subpath: None,
        source_branch: None,
        source_revision: None,
        remote_revision: None,
        update_status: "local_only".to_string(),
    };
    let central_path = result.central_path.to_string_lossy().to_string();
    let install_name = result.name.clone();
    let skill_id =
        skill_install::store_installed_skill_unlocked(store, &result, &metadata, active_scenario)
            .map_err(map_app_err)?;
    Ok((skill_id, install_name, central_path, "local".to_string()))
}

fn install_git_action(
    store: &SkillStore,
    repo_url: &str,
    name: Option<&str>,
    active_scenario: Option<&str>,
) -> anyhow::Result<(String, String, String, String)> {
    git_fetcher::validate_git_url(repo_url)?;
    let proxy_url = store.proxy_url();
    let parsed = git_fetcher::parse_git_source_resolved(repo_url, proxy_url.as_deref());
    let cancel = Arc::new(AtomicBool::new(false));
    let temp_dir = git_fetcher::clone_repo_ref_scoped(
        &parsed.clone_url,
        parsed.branch.as_deref(),
        parsed.subpath.as_deref(),
        Some(&cancel),
        proxy_url.as_deref(),
        None,
    )?;
    let result = (|| -> anyhow::Result<(String, String, String)> {
        let _lock = RepoLock::acquire_foreground("cli install git")?;
        let skill_dir =
            skill_install::resolve_skill_dir(&temp_dir, parsed.subpath.as_deref(), None)
                .map_err(map_app_err)?;
        let revision = git_fetcher::get_head_revision(&temp_dir)?;
        let install_result = installer::install_from_git_dir(&skill_dir, name)?;
        let metadata = skill_install::InstallSourceMetadata {
            source_type: "git".to_string(),
            source_ref: Some(parsed.original_url.clone()),
            source_ref_resolved: Some(parsed.clone_url.clone()),
            source_subpath: git_fetcher::relative_subpath(&temp_dir, &skill_dir),
            source_branch: parsed.branch.clone(),
            source_revision: Some(revision.clone()),
            remote_revision: Some(revision),
            update_status: "up_to_date".to_string(),
        };
        let central_path = install_result.central_path.to_string_lossy().to_string();
        let install_name = install_result.name.clone();
        let skill_id = skill_install::store_installed_skill_unlocked(
            store,
            &install_result,
            &metadata,
            active_scenario,
        )
        .map_err(map_app_err)?;
        Ok((skill_id, install_name, central_path))
    })();
    git_fetcher::cleanup_temp(&temp_dir);
    let (skill_id, install_name, central_path) = result?;
    Ok((skill_id, install_name, central_path, "git".to_string()))
}

fn install_skillssh_action(
    store: &SkillStore,
    shorthand: &str,
    active_scenario: Option<&str>,
) -> anyhow::Result<(String, String, String, String)> {
    let (source, skill_id_field) = parse_skillssh_shorthand(shorthand)?;
    let proxy_url = store.proxy_url();
    let repo_url = format!("https://github.com/{}.git", source);
    let cancel = Arc::new(AtomicBool::new(false));
    let temp_dir =
        git_fetcher::clone_repo_ref(&repo_url, None, Some(&cancel), proxy_url.as_deref())?;
    let result = (|| -> anyhow::Result<(String, String, String)> {
        let _lock = RepoLock::acquire_foreground("cli install skillssh")?;
        let skill_dir = skill_install::resolve_skill_dir(&temp_dir, None, Some(&skill_id_field))
            .map_err(map_app_err)?;
        let revision = git_fetcher::get_head_revision(&temp_dir)?;
        let source_ref = format!("{}/{}", source, skill_id_field);
        let (install_name, destination) =
            skill_install::resolve_skillssh_install_target(store, &source_ref, &skill_id_field)
                .map_err(map_app_err)?;
        let install_result =
            installer::install_skill_dir_to_destination(&skill_dir, &install_name, &destination)?;
        let metadata = skill_install::InstallSourceMetadata {
            source_type: "skillssh".to_string(),
            source_ref: Some(source_ref),
            source_ref_resolved: Some(repo_url.clone()),
            source_subpath: git_fetcher::relative_subpath(&temp_dir, &skill_dir),
            source_branch: None,
            source_revision: Some(revision.clone()),
            remote_revision: Some(revision),
            update_status: "up_to_date".to_string(),
        };
        let central_path = install_result.central_path.to_string_lossy().to_string();
        let skill_id = skill_install::store_installed_skill_unlocked(
            store,
            &install_result,
            &metadata,
            active_scenario,
        )
        .map_err(map_app_err)?;
        Ok((skill_id, install_name, central_path))
    })();
    git_fetcher::cleanup_temp(&temp_dir);
    let (skill_id, install_name, central_path) = result?;
    Ok((skill_id, install_name, central_path, "skillssh".to_string()))
}

/// Parse `owner/repo`, `owner/repo@skill`, or `owner/repo/skill` into
/// (source = "owner/repo", skill_id) — matching SkillsMP / install_from_skillssh.
fn parse_skillssh_shorthand(s: &str) -> anyhow::Result<(String, String)> {
    if let Some((head, skill_id)) = s.split_once('@') {
        if head.split('/').count() != 2 {
            bail!("invalid shorthand: '{s}' (expected owner/repo@skill)");
        }
        return Ok((head.to_string(), skill_id.to_string()));
    }
    let parts: Vec<&str> = s.split('/').collect();
    match parts.len() {
        2 => Ok((s.to_string(), parts[1].to_string())),
        3 => Ok((format!("{}/{}", parts[0], parts[1]), parts[2].to_string())),
        _ => bail!("invalid shorthand: '{s}'"),
    }
}

fn expand_path(s: &str) -> anyhow::Result<PathBuf> {
    if let Some(rest) = s.strip_prefix("~/") {
        let home = dirs_home()?;
        return Ok(home.join(rest));
    }
    if s == "~" {
        return dirs_home();
    }
    Ok(PathBuf::from(s))
}

fn dirs_home() -> anyhow::Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("HOME env var not set"))
}

// ── update / check ────────────────────────────────────────────────────────

fn run_update(
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

fn run_check(
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

// ── remove ────────────────────────────────────────────────────────────────

fn run_remove(
    store: &SkillStore,
    references: &[String],
    yes: bool,
    dry_run: bool,
) -> anyhow::Result<RemoveReport> {
    if references.is_empty() {
        bail!("no skill ref provided");
    }
    let mut ids = Vec::new();
    let mut failed = Vec::new();
    for r in references {
        match resolve_skill(store, r) {
            Ok(skill) => ids.push(skill.id),
            Err(e) => failed.push(format!("{r}: {e}")),
        }
    }

    if dry_run {
        return Ok(RemoveReport {
            ok: true,
            deleted: ids.len(),
            failed,
            dry_run: true,
        });
    }
    if !failed.is_empty() {
        bail!("could not resolve every skill: {}", failed.join("; "));
    }
    if !yes {
        bail!("refusing to delete {} skill(s) without --yes", ids.len());
    }

    let result = skill_delete::delete_managed_skills_by_ids(store, &ids).map_err(map_app_err)?;
    for missing in result.failed {
        failed.push(format!("{missing}: not found"));
    }
    Ok(RemoveReport {
        ok: true,
        deleted: result.deleted,
        failed,
        dry_run: false,
    })
}

// ── enable / disable ──────────────────────────────────────────────────────

fn run_deprecated_set_enabled(
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

fn run_sync(
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

// ── search ────────────────────────────────────────────────────────────────

fn run_search(
    store: &SkillStore,
    query: &str,
    limit: Option<usize>,
) -> anyhow::Result<Vec<SearchHit>> {
    let proxy_url = store.proxy_url();
    let bounded = limit.unwrap_or(60).clamp(1, 300);
    let hits = skillssh_api::search_skills(query, bounded, proxy_url.as_deref())?;
    Ok(hits
        .into_iter()
        .map(|s| {
            let install_ref = format!("{}/{}", s.source, s.skill_id);
            let skills_sh_url = format!("https://skills.sh/{}/{}", s.source, s.skill_id);
            SearchHit {
                install_ref,
                name: s.name,
                source: s.source,
                skill_id: s.skill_id,
                installs: s.installs,
                skills_sh_url,
            }
        })
        .collect())
}

// ── adopt ─────────────────────────────────────────────────────────────────

fn run_adopt(
    store: &SkillStore,
    paths: &[PathBuf],
    git_url: Option<&str>,
    git_subpath: Option<&str>,
    dry_run: bool,
) -> anyhow::Result<AdoptReport> {
    if paths.is_empty() {
        bail!("provide at least one path to scan");
    }
    if git_url.is_some() && paths.len() != 1 {
        bail!("--git-url requires exactly one path");
    }
    if git_subpath.is_some() && git_url.is_none() {
        bail!("--git-subpath requires --git-url");
    }

    // Resolve the source subpath for git-based adopts up front so we fail fast
    // before any filesystem work. parse_git_source pulls a subpath out of GitHub
    // /tree/branch/path URLs; --git-subpath is the explicit override (pass ""
    // to mean "skill lives at the repo root").
    #[allow(clippy::type_complexity)]
    let resolved_git: Option<(String, Option<String>, Option<String>, Option<String>)> =
        if let Some(url) = git_url {
            git_fetcher::validate_git_url(url)?;
            let parsed = git_fetcher::parse_git_source(url);
            let subpath = match git_subpath {
                Some(s) => {
                    if s.is_empty() {
                        None
                    } else {
                        Some(s.to_string())
                    }
                }
                None => parsed.subpath.clone(),
            };
            if subpath.is_none() && git_subpath.is_none() {
                bail!(
                    "--git-url has no subpath and --git-subpath was not provided. \
                     Pass --git-subpath \"\" if the skill lives at the repo root, \
                     --git-subpath <path> for a subdirectory, or use a URL like \
                     https://github.com/owner/repo/tree/branch/path/to/skill"
                );
            }
            Some((
                parsed.clone_url,
                subpath,
                parsed.branch,
                Some(url.to_string()),
            ))
        } else {
            None
        };

    // Build exclusion set: existing central paths, sync target paths, canonicals
    let mut excluded: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for skill in store.get_all_skills()? {
        let p = PathBuf::from(&skill.central_path);
        excluded.insert(p.clone());
        if let Ok(c) = p.canonicalize() {
            excluded.insert(c);
        }
    }
    for target in store.get_all_targets()? {
        let p = PathBuf::from(&target.target_path);
        excluded.insert(p.clone());
        if let Ok(c) = p.canonicalize() {
            excluded.insert(c);
        }
    }
    let central_root = central_repo::skills_dir();
    let central_root_canonical = central_root.canonicalize().unwrap_or(central_root.clone());

    let mut candidates: Vec<AdoptCandidate> = Vec::new();
    let mut skipped: Vec<AdoptCandidate> = Vec::new();

    for path in paths {
        let path = expand_path(&path.to_string_lossy())?;
        if !path.is_dir() {
            skipped.push(AdoptCandidate {
                path: path.to_string_lossy().to_string(),
                name: String::new(),
                reason: "not a directory".to_string(),
            });
            continue;
        }

        // If the user pointed directly at a single skill dir, treat it as one
        // candidate rather than scanning its children (which would be the
        // skill's own files/references and miss the SKILL.md at the root).
        if skill_metadata::is_valid_skill_dir(&path) {
            classify_adopt_candidate(
                &path,
                false, // path itself can't be a symlink-into-central in this branch
                &excluded,
                &central_root_canonical,
                &mut candidates,
                &mut skipped,
            );
            continue;
        }

        for entry in std::fs::read_dir(&path)? {
            let entry = entry?;
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let is_symlink = entry.file_type()?.is_symlink();
            classify_adopt_candidate(
                &dir,
                is_symlink,
                &excluded,
                &central_root_canonical,
                &mut candidates,
                &mut skipped,
            );
        }
    }

    if dry_run {
        return Ok(AdoptReport {
            ok: true,
            dry_run: true,
            adopted: Vec::new(),
            candidates,
            skipped,
        });
    }

    if git_url.is_some() && candidates.len() != 1 {
        bail!(
            "--git-url requires exactly one adoptable skill, found {}",
            candidates.len()
        );
    }

    let mut adopted = Vec::new();
    for c in &candidates {
        let dir = PathBuf::from(&c.path);
        let _lock = RepoLock::acquire_foreground("cli adopt")?;
        let result = installer::install_from_local(&dir, None)?;
        let metadata = if let Some((clone_url, subpath, branch, original_url)) = &resolved_git {
            skill_install::InstallSourceMetadata {
                source_type: "git".to_string(),
                source_ref: original_url.clone(),
                source_ref_resolved: Some(clone_url.clone()),
                source_subpath: subpath.clone(),
                source_branch: branch.clone(),
                source_revision: None,
                remote_revision: None,
                update_status: "unknown".to_string(),
            }
        } else {
            skill_install::InstallSourceMetadata {
                source_type: "local".to_string(),
                source_ref: Some(dir.to_string_lossy().to_string()),
                source_ref_resolved: None,
                source_subpath: None,
                source_branch: None,
                source_revision: None,
                remote_revision: None,
                update_status: "local_only".to_string(),
            }
        };
        let central_path = result.central_path.to_string_lossy().to_string();
        let install_name = result.name.clone();
        let source_type = metadata.source_type.clone();
        let skill_id =
            skill_install::store_installed_skill_unlocked(store, &result, &metadata, None)
                .map_err(map_app_err)?;
        adopted.push(InstallReport {
            ok: true,
            skill_id,
            name: install_name,
            central_path,
            source_type,
            synced: false,
            preset_id: None,
        });
    }

    Ok(AdoptReport {
        ok: true,
        dry_run: false,
        adopted,
        candidates: Vec::new(),
        skipped,
    })
}

fn classify_adopt_candidate(
    dir: &Path,
    is_symlink: bool,
    excluded: &std::collections::HashSet<PathBuf>,
    central_root_canonical: &Path,
    candidates: &mut Vec<AdoptCandidate>,
    skipped: &mut Vec<AdoptCandidate>,
) {
    let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    if excluded.contains(dir) || excluded.contains(&canonical) {
        skipped.push(AdoptCandidate {
            path: dir.to_string_lossy().to_string(),
            name,
            reason: "already managed (in DB or sync target)".to_string(),
        });
        return;
    }

    if is_symlink && canonical.starts_with(central_root_canonical) {
        skipped.push(AdoptCandidate {
            path: dir.to_string_lossy().to_string(),
            name,
            reason: "symlink into central repo (already managed)".to_string(),
        });
        return;
    }

    if !skill_metadata::is_valid_skill_dir(dir) {
        skipped.push(AdoptCandidate {
            path: dir.to_string_lossy().to_string(),
            name,
            reason: "no SKILL.md / skill.md".to_string(),
        });
        return;
    }

    candidates.push(AdoptCandidate {
        path: dir.to_string_lossy().to_string(),
        name,
        reason: "ready".to_string(),
    });
}

// ── tag ───────────────────────────────────────────────────────────────────

fn run_tag(args: TagArgs, store: &SkillStore, json: bool) -> anyhow::Result<()> {
    match args.command {
        TagCommand::Add { reference, tags } => {
            let skill = resolve_skill(store, &reference)?;
            let mut current = store
                .get_tags_map()?
                .get(&skill.id)
                .cloned()
                .unwrap_or_default();
            for t in tags {
                let tag = t.trim();
                if !tag.is_empty() && !current.iter().any(|c| c == tag) {
                    current.push(tag.to_string());
                }
            }
            skill_tags::set_skill_tags_internal(store, &skill.id, &current).map_err(map_app_err)?;
            print_json(
                &TagReport {
                    skill_id: skill.id,
                    name: skill.name,
                    tags: current,
                },
                json,
            );
        }
        TagCommand::Remove { reference, tags } => {
            let skill = resolve_skill(store, &reference)?;
            let mut current = store
                .get_tags_map()?
                .get(&skill.id)
                .cloned()
                .unwrap_or_default();
            current.retain(|c| !tags.iter().any(|t| t.trim() == c));
            skill_tags::set_skill_tags_internal(store, &skill.id, &current).map_err(map_app_err)?;
            print_json(
                &TagReport {
                    skill_id: skill.id,
                    name: skill.name,
                    tags: current,
                },
                json,
            );
        }
        TagCommand::Set { reference, tags } => {
            let skill = resolve_skill(store, &reference)?;
            skill_tags::set_skill_tags_internal(store, &skill.id, &tags).map_err(map_app_err)?;
            let current = store
                .get_tags_map()?
                .get(&skill.id)
                .cloned()
                .unwrap_or_default();
            print_json(
                &TagReport {
                    skill_id: skill.id,
                    name: skill.name,
                    tags: current,
                },
                json,
            );
        }
        TagCommand::Rename { old_name, new_name } => {
            let old_name = old_name.trim().to_string();
            let new_name = new_name.trim().to_string();
            let affected = skill_tags::rename_tag_internal(store, &old_name, &new_name)
                .map_err(map_app_err)?;
            print_json(
                &GlobalTagReport {
                    ok: true,
                    tag: old_name,
                    renamed_to: Some(new_name),
                    affected_skills: affected.len(),
                    dry_run: false,
                    deleted: false,
                },
                json,
            );
        }
        TagCommand::Delete { name, yes, dry_run } => {
            let name = name.trim().to_string();
            let affected_skills = store
                .get_tags_map()?
                .values()
                .filter(|tags| tags.iter().any(|tag| tag == &name))
                .count();
            if !dry_run && !yes {
                bail!("refusing to delete tag without --yes");
            }
            if !dry_run {
                skill_tags::delete_tag_internal(store, &name).map_err(map_app_err)?;
            }
            print_json(
                &GlobalTagReport {
                    ok: true,
                    tag: name,
                    renamed_to: None,
                    affected_skills,
                    dry_run,
                    deleted: !dry_run,
                },
                json,
            );
        }
        TagCommand::List { reference } => {
            if let Some(r) = reference {
                let skill = resolve_skill(store, &r)?;
                let tags = store
                    .get_tags_map()?
                    .get(&skill.id)
                    .cloned()
                    .unwrap_or_default();
                print_json(
                    &TagReport {
                        skill_id: skill.id,
                        name: skill.name,
                        tags,
                    },
                    json,
                );
            } else {
                print_json(&store.get_all_tags()?, json);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::{PresetCommand, SkillsCommand, ToolsCommand};
    use crate::presets::{preset_status, run_preset_deployment};
    use app_lib::core::skill_store::{ScenarioRecord, SkillRecord};
    use app_lib::core::tool_adapters::{CustomToolDef, ToolCategory};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parses_agent_friendly_commands_and_aliases() {
        let cli = Cli::try_parse_from([
            "skills-manager-cli",
            "--json",
            "skills",
            "deploy",
            "browser",
            "--to",
            "codex",
            "--agent",
            "claude_code",
            "--dry-run",
        ])
        .unwrap();
        assert!(cli.json);
        assert!(matches!(
            cli.command,
            Commands::Skills(SkillsArgs {
                command: SkillsCommand::Deploy {
                    agents,
                    dry_run: true,
                    ..
                }
            }) if agents == vec!["codex", "claude_code"]
        ));

        let cli = Cli::try_parse_from([
            "skills-manager-cli",
            "skills",
            "list",
            "--query",
            "react",
            "--tag",
            "frontend",
            "--preset",
            "Web Dev",
            "--deployed-to",
            "claude_code",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Skills(SkillsArgs {
                command: SkillsCommand::List {
                    query: Some(query),
                    tags,
                    preset: Some(preset),
                    deployed_to: Some(agent),
                    ..
                }
            }) if query == "react"
                && tags == vec!["frontend"]
                && preset == "Web Dev"
                && agent == "claude_code"
        ));

        let cli = Cli::try_parse_from([
            "skills-manager-cli",
            "agents",
            "enable",
            "codex",
            "claude_code",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Tools(ToolsArgs {
                command: ToolsCommand::Enable { agents }
            }) if agents == vec!["codex", "claude_code"]
        ));

        let cli = Cli::try_parse_from([
            "skills-manager-cli",
            "presets",
            "open",
            "Web Dev",
            "--agent",
            "codex",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Presets(PresetArgs {
                command: PresetCommand::Deploy {
                    reference,
                    agents,
                    ..
                }
            }) if reference == "Web Dev" && agents == vec!["codex"]
        ));
    }

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
