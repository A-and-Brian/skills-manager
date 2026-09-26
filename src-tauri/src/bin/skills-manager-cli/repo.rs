//! `repo` (the base folder and its status) and `git` (backup of the central library).

use app_lib::core::{
    app_state, central_repo, git_backup, merge, repo_lock::RepoLock, skill_store::SkillStore,
    sync_metadata,
};

use crate::args::{GitArgs, GitCommand, RepoArgs, RepoCommand};
use crate::output::print_json;
use crate::reports::RepoStatus;

pub(crate) fn run_repo(args: RepoArgs, store: &SkillStore, json: bool) -> anyhow::Result<()> {
    match args.command {
        RepoCommand::Status => print_json(&repo_status(store), json),
        RepoCommand::SetPath { path } => {
            central_repo::set_base_dir_override(Some(path))?;
            let store = app_state::initialize_cli_store()?;
            print_json(&repo_status(&store), json);
        }
        RepoCommand::ResetPath => {
            central_repo::set_base_dir_override(None)?;
            let store = app_state::initialize_cli_store()?;
            print_json(&repo_status(&store), json);
        }
    }
    Ok(())
}

fn repo_status(store: &SkillStore) -> RepoStatus {
    RepoStatus {
        base_dir: central_repo::base_dir().to_string_lossy().to_string(),
        skills_dir: central_repo::skills_dir().to_string_lossy().to_string(),
        db_path: central_repo::db_path().to_string_lossy().to_string(),
        metadata_dir: sync_metadata::metadata_dir().to_string_lossy().to_string(),
        skill_count: store.get_all_skills().unwrap_or_default().len(),
        preset_count: store.get_all_scenarios().unwrap_or_default().len(),
        active_preset_id: store.get_active_scenario_id().unwrap_or(None),
    }
}

// ── git ───────────────────────────────────────────────────────────────────

pub(crate) fn run_git(
    args: GitArgs,
    store: &SkillStore,
    has_skills_root: bool,
    json: bool,
) -> anyhow::Result<()> {
    match args.command {
        GitCommand::Status => {
            print_json(&git_backup::get_status(&central_repo::skills_dir())?, json)
        }
        GitCommand::Init => {
            // No settings store on this path; the hostname default matches
            // what the GUI derives, and the GUI reconciles the repo identity
            // on its next backup anyway.
            git_backup::init_repo(
                &central_repo::skills_dir(),
                &git_backup::default_device_name(),
            )?;
            print_json(&git_backup::get_status(&central_repo::skills_dir())?, json);
        }
        GitCommand::Clone { url } => {
            let target = central_repo::skills_dir();
            if has_skills_root {
                git_backup::clone_into_strict(&target, &url)?;
            } else {
                git_backup::clone_into(&target, &url)?;
            }
            print_json(&git_backup::get_status(&target)?, json);
        }
        GitCommand::SetRemote { url } => {
            git_backup::set_remote(&central_repo::skills_dir(), &url)?;
            print_json(&git_backup::get_status(&central_repo::skills_dir())?, json);
        }
        GitCommand::Pull => {
            // Same engine gate as the GUI sync (object merge by default,
            // merge_engine=system opts out). A raw line merge from this CLI
            // would read as an old-client violation on other devices (§6).
            let dir = central_repo::skills_dir();
            {
                let _lock = RepoLock::acquire_foreground("git pull")?;
                let device = store
                    .get_setting("backup_device_name")
                    .ok()
                    .flatten()
                    .map(|v| git_backup::sanitize_device_name(&v))
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(git_backup::default_device_name);
                let _ = git_backup::configure_device_identity(&dir, &device);
                merge::gated_pull_unlocked(store, &dir)?;
            }
            // Reconcile the DB from the merged metadata (takes its own lock).
            sync_metadata::reindex_from_metadata(store)?;
            print_json(&git_backup::get_status(&dir)?, json);
        }
        GitCommand::Push => {
            git_backup::push(&central_repo::skills_dir())?;
            print_json(&git_backup::get_status(&central_repo::skills_dir())?, json);
        }
        GitCommand::Commit { message } => {
            git_backup::commit_all(&central_repo::skills_dir(), &message)?;
            let tag = git_backup::create_snapshot_tag(&central_repo::skills_dir())?;
            print_json(&serde_json::json!({"ok": true, "tag": tag}), json);
        }
        GitCommand::Versions { limit } => print_json(
            &git_backup::list_snapshot_versions(&central_repo::skills_dir(), limit)?,
            json,
        ),
        GitCommand::Restore { tag } => {
            git_backup::restore_snapshot_version(&central_repo::skills_dir(), &tag)?;
            print_json(&git_backup::get_status(&central_repo::skills_dir())?, json);
        }
        GitCommand::PruneSyncRefs => {
            let removed = git_backup::prune_hidden_refs_on_remote(&central_repo::skills_dir())?;
            print_json(&serde_json::json!({ "removed": removed }), json);
        }
    }
    Ok(())
}
