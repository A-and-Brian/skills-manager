use std::path::{Path, PathBuf};

use anyhow::bail;
use app_lib::core::{
    app_state, central_repo, git_fetcher, installer, repo_lock::RepoLock, serve, skill_install,
    skill_metadata, skill_store::SkillStore, skill_tags, skill_update, skill_update_check,
};
use clap::{Args, Parser, Subcommand};

use crate::args::{GitArgs, PresetArgs, RepoArgs, SkillsArgs, TagArgs, TagCommand, ToolsArgs};
use crate::output::{error_envelope, map_app_err, print_json};
use crate::presets::run_presets;
use crate::repo::{run_git, run_repo};
use crate::reports::{
    AdoptCandidate, AdoptReport, CheckReport, GlobalTagReport, InstallReport, TagReport,
    UpdateReport,
};
use crate::skills::install::expand_path;
use crate::skills::list::resolve_skill;
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
}
