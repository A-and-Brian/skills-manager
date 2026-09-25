use std::path::PathBuf;

use app_lib::core::{app_state, central_repo, serve};
use clap::{Args, Parser, Subcommand};

use crate::args::{GitArgs, PresetArgs, RepoArgs, SkillsArgs, ToolsArgs};
use crate::output::error_envelope;
use crate::presets::run_presets;
use crate::repo::{run_git, run_repo};
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
