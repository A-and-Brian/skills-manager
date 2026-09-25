//! `skills install` (local, git or skills.sh), `remove`, and the skills.sh `search`.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{anyhow, bail};
use app_lib::core::{
    git_fetcher, installer, repo_lock::RepoLock, skill_delete, skill_install,
    skill_store::SkillStore, skillssh_api,
};

use crate::output::map_app_err;
use crate::presets::resolve_scenario;
use crate::reports::{InstallReport, RemoveReport, SearchHit};
use crate::skills::list::resolve_skill;

pub(crate) enum InstallKind {
    Local,
    Git,
    Skillssh,
}

pub(crate) enum SyncTarget {
    None,
    Active,
    Specific(String),
}

// ── install ───────────────────────────────────────────────────────────────

pub(crate) fn classify_ref(
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

pub(crate) fn run_install(
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

pub(crate) fn expand_path(s: &str) -> anyhow::Result<PathBuf> {
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

// ── remove ────────────────────────────────────────────────────────────────

pub(crate) fn run_remove(
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

// ── search ────────────────────────────────────────────────────────────────

pub(crate) fn run_search(
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
