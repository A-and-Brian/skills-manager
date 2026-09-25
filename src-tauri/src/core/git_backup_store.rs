//! The git backup's settings, device identity and index reconciliation, which
//! need the skill store that `git_backup` itself does not have.

use std::path::Path;
use walkdir::WalkDir;

use crate::core::{
    central_repo, git2_engine, git_backup, skill_metadata, skill_store::SkillStore, sync_metadata,
};

/// Push the persisted engine choice (`git_backup_engine` = "git2" | "system")
/// and proxy setting into the core layer, which has no store access. Called
/// at the entry of every command that can touch the network.
pub(crate) fn sync_engine_pref(store: &SkillStore) {
    let git2_enabled = store
        .get_setting("git_backup_engine")
        .ok()
        .flatten()
        .map(|v| v.trim() == "git2")
        .unwrap_or(false);
    git2_engine::set_preference(git2_enabled, store.proxy_url());
}

/// Resolve the device name (§4.3 设备命名): the persisted setting, or a
/// hostname-derived default that is persisted on first use so it stays stable
/// across sessions.
pub(crate) fn effective_device_name(store: &SkillStore) -> String {
    let saved = store
        .get_setting("backup_device_name")
        .ok()
        .flatten()
        .map(|v| git_backup::sanitize_device_name(&v))
        .filter(|v| !v.is_empty());
    if let Some(name) = saved {
        return name;
    }
    let name = git_backup::default_device_name();
    if let Err(e) = store.set_setting("backup_device_name", &name) {
        log::warn!("device name: failed to persist default: {e:#}");
    }
    name
}

/// Best-effort: bring the repo's commit identity in line with the device name
/// before an operation that can create commits. Identity trouble must never
/// block a backup — commits then just carry the previous (or global) author.
pub(crate) fn apply_device_identity(store: &SkillStore, skills_dir: &Path) {
    let name = effective_device_name(store);
    if let Err(e) = git_backup::configure_device_identity(skills_dir, &name) {
        log::warn!("device name: failed to configure git identity: {e:#}");
    }
}

pub(crate) fn reconcile_skills_index_unlocked(store: &SkillStore) -> anyhow::Result<()> {
    sync_metadata::cleanup_temporary_files()?;
    if sync_metadata::has_complete_skill_snapshot() {
        sync_metadata::reindex_from_metadata_unlocked(store)?;
        return Ok(());
    }

    let skills_dir = central_repo::skills_dir();
    std::fs::create_dir_all(&skills_dir)?;

    // Remove stale DB records whose central directories no longer exist.
    let existing = store.get_all_skills()?;
    for skill in existing {
        if !std::path::Path::new(&skill.central_path).exists() {
            store.delete_skill(&skill.id)?;
        }
    }

    // Add missing DB records for directories present in central repo.
    for entry in WalkDir::new(&skills_dir)
        .min_depth(1)
        .max_depth(6)
        .into_iter()
        .filter_entry(|e| e.file_name().to_string_lossy() != ".git")
        .flatten()
    {
        let path = entry.path().to_path_buf();
        if !entry.file_type().is_dir() || !skill_metadata::is_valid_skill_dir(&path) {
            continue;
        }

        let central_path = path.to_string_lossy().to_string();
        if store.get_skill_by_central_path(&central_path)?.is_some() {
            continue;
        }

        let meta = crate::core::skill_metadata::parse_skill_md(&path);
        let inferred_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown-skill".to_string());
        let name = meta
            .name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(inferred_name);
        let now = chrono::Utc::now().timestamp_millis();

        let record = crate::core::skill_store::SkillRecord {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            description: meta.description,
            source_type: "import".to_string(),
            source_ref: Some(central_path.clone()),
            source_ref_resolved: None,
            source_subpath: None,
            source_branch: None,
            source_revision: None,
            remote_revision: None,
            central_path,
            content_hash: crate::core::content_hash::hash_directory(&path).ok(),
            enabled: true,
            created_at: now,
            updated_at: now,
            status: "ok".to_string(),
            update_status: "local_only".to_string(),
            last_checked_at: Some(now),
            last_check_error: None,
        };

        store.insert_skill(&record)?;
    }

    sync_metadata::write_all_from_db_unlocked(store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_support::{git, test_env};

    #[test]
    fn device_name_default_persists_and_rename_updates_repo_config() {
        let env = test_env();
        // First resolve derives a hostname default and persists it, so the
        // name stays stable even if the hostname later changes.
        let name = effective_device_name(&env.store);
        assert!(!name.is_empty());
        assert_eq!(
            env.store
                .get_setting("backup_device_name")
                .unwrap()
                .as_deref(),
            Some(name.as_str())
        );

        // With a repo present, a rename rewrites the repo-local identity used
        // for all future commits (§4.3).
        git(&env.skills_dir, &["init", "-b", "main"]);
        env.store
            .set_setting("backup_device_name", "Work Laptop")
            .unwrap();
        apply_device_identity(&env.store, &env.skills_dir);
        let user_name = std::process::Command::new("git")
            .arg("-C")
            .arg(&env.skills_dir)
            .args(["config", "--local", "--get", "user.name"])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&user_name.stdout).trim(),
            "Work Laptop"
        );
    }
}
