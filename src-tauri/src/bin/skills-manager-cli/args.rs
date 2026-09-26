//! The clap subcommands and their arguments, one enum per command group.

use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub(crate) struct RepoArgs {
    #[command(subcommand)]
    pub(crate) command: RepoCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum RepoCommand {
    Status,
    SetPath { path: String },
    ResetPath,
}

#[derive(Args, Debug)]
pub(crate) struct ToolsArgs {
    #[command(subcommand)]
    pub(crate) command: ToolsCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum ToolsCommand {
    List,
    Enable {
        #[arg(required = true)]
        agents: Vec<String>,
    },
    Disable {
        #[arg(required = true)]
        agents: Vec<String>,
    },
}

#[derive(Args, Debug)]
pub(crate) struct SkillsArgs {
    #[command(subcommand)]
    pub(crate) command: SkillsCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum SkillsCommand {
    List {
        #[arg(long)]
        query: Option<String>,
        #[arg(long = "tag", conflicts_with = "untagged")]
        tags: Vec<String>,
        #[arg(long)]
        preset: Option<String>,
        #[arg(long, value_name = "AGENT")]
        deployed_to: Option<String>,
        #[arg(long)]
        untagged: bool,
        #[arg(long)]
        no_preset: bool,
        #[arg(long)]
        source: Option<String>,
    },
    Show {
        reference: String,
    },
    Export {
        reference: String,
        #[arg(long)]
        dest: PathBuf,
        /// Overwrite the destination if it already exists. Without this, an
        /// existing destination is left untouched and the command fails.
        #[arg(long)]
        force: bool,
    },
    Install {
        /// Ref: local path, git URL, or owner/repo[@skill] / owner/repo/skill
        reference: String,
        #[arg(long, conflicts_with_all = ["git", "skillssh"])]
        local: bool,
        #[arg(long, conflicts_with_all = ["local", "skillssh"])]
        git: bool,
        #[arg(long, conflicts_with_all = ["local", "git"])]
        skillssh: bool,
        #[arg(long)]
        name: Option<String>,
        /// Add to current active preset and sync agents
        #[arg(long, conflicts_with = "sync_preset")]
        sync: bool,
        /// Add to given preset (by id or name) and sync agents
        #[arg(long, alias = "sync-scenario", value_name = "REF")]
        sync_preset: Option<String>,
    },
    Update {
        /// Skill ref (id / name / dir basename / central path). Omit for --all.
        reference: Option<String>,
        #[arg(long)]
        all: bool,
    },
    Check {
        reference: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        force: bool,
    },
    Remove {
        references: Vec<String>,
        #[arg(long, short)]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Deprecated compatibility command: use skills deploy.
    Enable {
        references: Vec<String>,
    },
    /// Deprecated compatibility command: use skills undeploy.
    Disable {
        references: Vec<String>,
    },
    /// Deploy library skills to one or more agents' global skill directories.
    Deploy {
        #[arg(required = true)]
        references: Vec<String>,
        #[arg(long = "agent", alias = "to", value_name = "AGENT", required = true)]
        agents: Vec<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove managed deployments from one or more agents.
    Undeploy {
        #[arg(required = true)]
        references: Vec<String>,
        #[arg(long = "agent", alias = "from", value_name = "AGENT", required = true)]
        agents: Vec<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Show preset membership and actual per-agent deployment state.
    Status {
        reference: String,
    },
    Sync {
        /// Preset id or name (default = current active preset)
        #[arg(long, alias = "scenario")]
        preset: Option<String>,
        /// Tool key (default = all enabled tools)
        #[arg(long)]
        tool: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    Search {
        query: String,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Re-point an installed skill at a git source in place, keeping its id,
    /// tags, preset membership and deployments.
    SetSource {
        /// Skill ref (id / name / dir basename / central path)
        reference: String,
        /// Git URL or owner/repo, optionally a GitHub tree URL encoding branch and subpath
        #[arg(long = "git-url")]
        git_url: String,
        /// Subpath inside the repo. Pass "" if the skill is at the repo root.
        /// Overrides a subpath encoded in the URL.
        #[arg(long)]
        subpath: Option<String>,
        /// Branch to track. Overrides a branch encoded in the URL.
        #[arg(long)]
        branch: Option<String>,
        /// Overwrite the central copy when the new source's content differs.
        /// Without this, a content difference is refused.
        #[arg(long)]
        force: bool,
        /// Resolve and compare without writing anything.
        #[arg(long)]
        dry_run: bool,
    },
    Adopt {
        /// Agent skill dirs to scan (e.g. ~/.claude/skills), or a single skill dir
        paths: Vec<PathBuf>,
        /// If set, adopt as git source (only with single adoptable skill)
        #[arg(long)]
        git_url: Option<String>,
        /// Subpath inside the git repo where the adopted skill lives. Required
        /// with --git-url when the URL itself does not encode a subpath. Pass
        /// "" if the skill is at the repo root.
        #[arg(long)]
        git_subpath: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    Tag(TagArgs),
}

#[derive(Args, Debug)]
pub(crate) struct TagArgs {
    #[command(subcommand)]
    pub(crate) command: TagCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum TagCommand {
    Add {
        reference: String,
        tags: Vec<String>,
    },
    Remove {
        reference: String,
        tags: Vec<String>,
    },
    Set {
        reference: String,
        tags: Vec<String>,
    },
    Rename {
        old_name: String,
        new_name: String,
    },
    Delete {
        name: String,
        #[arg(long, short)]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
    List {
        reference: Option<String>,
    },
}

#[derive(Args, Debug)]
pub(crate) struct PresetArgs {
    #[command(subcommand)]
    pub(crate) command: PresetCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum PresetCommand {
    List,
    Current,
    Show {
        reference: String,
    },
    Create {
        name: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        icon: Option<String>,
    },
    Update {
        reference: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        icon: Option<String>,
    },
    Delete {
        reference: String,
        #[arg(long, short)]
        yes: bool,
        #[arg(long)]
        dry_run: bool,
    },
    Preview {
        reference: String,
    },
    /// Legacy exclusive switch: replaces the current active preset.
    Apply {
        reference: String,
    },
    /// Legacy exclusive close operation. Prefer undeploy for additive presets.
    Deactivate {
        reference: String,
    },
    /// Additively deploy this preset without removing other deployed presets.
    #[command(alias = "activate", alias = "enable", alias = "start", alias = "open")]
    Deploy {
        reference: String,
        #[arg(long = "agent", value_name = "AGENT")]
        agents: Vec<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove this preset's deployed pairs without changing its membership.
    #[command(alias = "disable", alias = "stop", alias = "close", alias = "off")]
    Undeploy {
        reference: String,
        #[arg(long = "agent", value_name = "AGENT")]
        agents: Vec<String>,
        #[arg(long)]
        dry_run: bool,
    },
    Status {
        reference: String,
        #[arg(long = "agent", value_name = "AGENT")]
        agents: Vec<String>,
    },
    AddSkill {
        preset: String,
        #[arg(required = true)]
        skills: Vec<String>,
    },
    RemoveSkill {
        preset: String,
        #[arg(required = true)]
        skills: Vec<String>,
    },
}

#[derive(Args, Debug)]
pub(crate) struct GitArgs {
    #[command(subcommand)]
    pub(crate) command: GitCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum GitCommand {
    Status,
    Init,
    Clone {
        url: String,
    },
    SetRemote {
        url: String,
    },
    Pull,
    Push,
    Commit {
        #[arg(short, long)]
        message: String,
    },
    Versions {
        #[arg(long)]
        limit: Option<usize>,
    },
    Restore {
        tag: String,
    },
    /// Remove refs/skills-manager/* that a `git push --mirror`/--all style
    /// operation uploaded to the backup remote. Local sync refs are kept.
    PruneSyncRefs,
}
