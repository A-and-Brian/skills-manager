//! Moving skills between a project and the library: importing, exporting
//! and pulling library updates into a project.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tauri::State;

use super::agents_model::{
    agent_skill_configs, export_agent_keys, is_copy_project, read_workspace_skills,
    resolve_agent_skills_roots, vendored_variant,
};
use super::fs_safety::{ensure_dir_within_root, ensure_safe_skill_relative_path};
use crate::core::project_deploy;
use crate::core::project_scanner::VENDORED_SKILLS_DIR;
use crate::core::project_skill_match::{
    classify_sync_status, find_best_center_match, slugify_skill_dir_name,
    source_ref_matches_skill_path,
};
use crate::core::skill_store::SkillRecord;
use crate::core::{error::AppError, host::HostCtx, installer, project_scanner, sync_engine};

#[tauri::command]
pub async fn import_project_skill_to_center(
    ctx: State<'_, HostCtx>,
    project_id: String,
    skill_relative_path: String,
    agent: String,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        import_project_skill_to_center_core(&ctx, project_id, skill_relative_path, agent)
    })
    .await?
}

pub fn import_project_skill_to_center_core(
    ctx: &HostCtx,
    project_id: String,
    skill_relative_path: String,
    agent: String,
) -> Result<(), AppError> {
    let store = ctx.store.clone();
    ensure_safe_skill_relative_path(&skill_relative_path)?;

    let record = store
        .get_project_by_id(&project_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Workspace not found"))?;

    let configs = agent_skill_configs(&store);
    let skills = read_workspace_skills(&record, &configs);
    let skill = skills
        .iter()
        .find(|s| s.relative_path == skill_relative_path && s.agent == agent)
        .ok_or_else(|| AppError::not_found("Skill not found in workspace"))?;
    // A link to a vendored copy imports the vendored copy, and binds to it.
    let skill = vendored_variant(&record, &skills, skill).unwrap_or(skill);

    let source_path = PathBuf::from(&skill.path);
    let all_managed = store.get_all_skills().unwrap_or_default();
    // Use the same matching logic as the UI (find_best_center_match) to
    // stay consistent with sync-status display. After updating, bind
    // source_ref so future imports match by exact path.
    if let Some(existing) = find_best_center_match(skill, &all_managed) {
        let result = installer::install_from_local_to_destination(
            &source_path,
            Some(&existing.name),
            Path::new(&existing.central_path),
        )
        .map_err(AppError::io)?;
        store
            .update_skill_after_install(
                &existing.id,
                &existing.name,
                result.description.as_deref(),
                existing.source_revision.as_deref(),
                existing.remote_revision.as_deref(),
                Some(&result.content_hash),
                "local_only",
            )
            .map_err(AppError::db)?;
        // Only update source_ref when the match was already by source_ref
        // path (not by hash or name). This avoids permanently rebinding
        // unrelated center skills that merely share a name or content.
        let already_matched_by_ref = source_ref_matches_skill_path(
            &skill.path,
            std::fs::canonicalize(&skill.path).ok().as_ref(),
            existing,
        );
        if existing.source_type == "local" && already_matched_by_ref {
            store
                .update_skill_source_ref(&existing.id, &skill.path)
                .map_err(AppError::db)?;
        }
        return Ok(());
    }

    let result =
        installer::install_from_local(&source_path, Some(&skill.name)).map_err(AppError::io)?;

    let now = chrono::Utc::now().timestamp_millis();
    let id = uuid::Uuid::new_v4().to_string();

    let skill_record = SkillRecord {
        id: id.clone(),
        name: result.name.clone(),
        description: result.description.clone(),
        source_type: "local".to_string(),
        source_ref: Some(skill.path.clone()),
        source_ref_resolved: None,
        source_subpath: None,
        source_branch: None,
        source_revision: None,
        remote_revision: None,
        central_path: result.central_path.to_string_lossy().to_string(),
        content_hash: Some(result.content_hash.clone()),
        enabled: true,
        created_at: now,
        updated_at: now,
        status: "ok".to_string(),
        update_status: "local_only".to_string(),
        last_checked_at: Some(now),
        last_check_error: None,
    };

    store.insert_skill(&skill_record).map_err(AppError::db)?;

    Ok(())
}

#[tauri::command]
pub async fn update_project_skill_to_center(
    ctx: State<'_, HostCtx>,
    project_id: String,
    skill_relative_path: String,
    agent: String,
) -> Result<(), AppError> {
    import_project_skill_to_center(ctx, project_id, skill_relative_path, agent).await
}

#[tauri::command]
pub fn slugify_skill_names(names: Vec<String>) -> Vec<String> {
    names.iter().map(|n| slugify_skill_dir_name(n)).collect()
}

#[tauri::command]
pub async fn export_skill_to_project(
    ctx: State<'_, HostCtx>,
    skill_id: String,
    project_id: String,
    agents: Option<Vec<String>>,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        export_skill_to_project_core(&ctx, skill_id, project_id, agents)
    })
    .await?
}

pub fn export_skill_to_project_core(
    ctx: &HostCtx,
    skill_id: String,
    project_id: String,
    agents: Option<Vec<String>>,
) -> Result<(), AppError> {
    let store = ctx.store.clone();
    let project = store
        .get_project_by_id(&project_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Workspace not found"))?;

    let skill = store
        .get_skill_by_id(&skill_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Skill not found"))?;

    let source = PathBuf::from(&skill.central_path);
    let dir_name = sync_engine::target_dir_name(&source, &skill.name);
    ensure_safe_skill_relative_path(&dir_name)?;
    let agent_keys = export_agent_keys(&store, &project, agents)?;

    if is_copy_project(&project) {
        let failed = project_deploy::deploy_copy_mode(
            Path::new(&project.path),
            &agent_skill_configs(&store),
            &source,
            &dir_name,
            &agent_keys,
        )
        .map_err(AppError::io)?;
        if !failed.is_empty() {
            let failures: Vec<String> = failed
                .iter()
                .map(|failure| format!("{}: {}", failure.agent, failure.error))
                .collect();
            return Err(AppError::io(format!(
                "\"{}\" was vendored into {VENDORED_SKILLS_DIR}, but links were not created for {}",
                skill.name,
                failures.join("; ")
            )));
        }
        return Ok(());
    }

    for agent_key in &agent_keys {
        let (skills_root, disabled_root) = resolve_agent_skills_roots(&store, &project, agent_key)
            .ok_or_else(|| AppError::not_found(format!("Unknown agent: {}", agent_key)))?;
        let target_dir = skills_root.join(&dir_name);

        if target_dir.strip_prefix(&skills_root).is_err() {
            return Err(AppError::invalid_input("Invalid skill directory path"));
        }

        if target_dir.exists()
            || disabled_root
                .as_ref()
                .map(|path| path.join(&dir_name).exists())
                .unwrap_or(false)
        {
            return Err(AppError::invalid_input(format!(
                "Skill \"{}\" already exists in this workspace for agent {}",
                skill.name, agent_key
            )));
        }
    }

    let configured_mode = store.get_setting("sync_mode").map_err(AppError::db)?;
    // Two agents can resolve to the same project skills root, in which case
    // the second pass would find the directory the first just wrote and
    // refuse it. The artifact is already correct, so skip instead.
    let mut written: HashSet<PathBuf> = HashSet::new();
    for agent_key in &agent_keys {
        let (skills_root, _) = resolve_agent_skills_roots(&store, &project, agent_key)
            .ok_or_else(|| AppError::not_found(format!("Unknown agent: {}", agent_key)))?;
        if !written.insert(skills_root.join(&dir_name)) {
            continue;
        }
        let mode = sync_engine::sync_mode_for_tool(agent_key, configured_mode.as_deref());
        project_deploy::deploy_skill(&source, &skills_root, &dir_name, mode)
            .map_err(AppError::io)?;
    }

    Ok(())
}

#[tauri::command]
pub async fn update_project_skill_from_center(
    ctx: State<'_, HostCtx>,
    project_id: String,
    skill_relative_path: String,
    agent: String,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        update_project_skill_from_center_core(&ctx, project_id, skill_relative_path, agent)
    })
    .await?
}

pub fn update_project_skill_from_center_core(
    ctx: &HostCtx,
    project_id: String,
    skill_relative_path: String,
    agent: String,
) -> Result<(), AppError> {
    let store = ctx.store.clone();
    ensure_safe_skill_relative_path(&skill_relative_path)?;

    let record = store
        .get_project_by_id(&project_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Workspace not found"))?;

    let configs = agent_skill_configs(&store);
    let skills = read_workspace_skills(&record, &configs);
    let skill = skills
        .iter()
        .find(|s| s.relative_path == skill_relative_path && s.agent == agent)
        .ok_or_else(|| AppError::not_found("Skill not found in workspace"))?;

    let all_managed = store.get_all_skills().unwrap_or_default();
    // Copy mode updates the vendored copy only; its links follow.
    if let Some(vendored) = vendored_variant(&record, &skills, skill) {
        return update_vendored_from_center(vendored, &all_managed);
    }
    let managed = find_best_center_match(skill, &all_managed)
        .ok_or_else(|| AppError::not_found("No matching skill in center"))?;
    ensure_not_project_newer(skill, managed)?;

    let (skills_root, disabled_root) = resolve_agent_skills_roots(&store, &record, &agent)
        .ok_or_else(|| AppError::not_found(format!("Unknown agent: {}", agent)))?;
    let target_path = PathBuf::from(&skill.path);
    if target_path.starts_with(&skills_root) {
        ensure_dir_within_root(&target_path, &skills_root)?;
    } else if disabled_root
        .as_ref()
        .map(|root| target_path.starts_with(root))
        .unwrap_or(false)
    {
        let disabled_root = disabled_root.expect("checked above");
        ensure_dir_within_root(&target_path, &disabled_root)?;
    } else {
        return Err(AppError::invalid_input("Invalid skill directory path"));
    }

    let source = PathBuf::from(&managed.central_path);
    let configured_mode = store.get_setting("sync_mode").map_err(AppError::db)?;
    // A copy-mode project holds files, never links into the library.
    let mode = if is_copy_project(&record) {
        sync_engine::SyncMode::Copy
    } else {
        sync_engine::sync_mode_for_tool(&agent, configured_mode.as_deref())
    };
    // UserConfirmed: this intentionally replaces an existing project copy
    // the user chose to update, and project deployments never create
    // `skill_targets` rows, so no record could vouch for it. The
    // project_newer check above is the guard that makes this safe.
    sync_engine::sync_skill(
        &source,
        &target_path,
        mode,
        sync_engine::ReplacePolicy::UserConfirmed,
    )
    .map_err(AppError::io)?;
    Ok(())
}

/// Mirror the global-workspace protection (agent_workspace.rs): never
/// overwrite a project copy that has unsynced local edits (#225 review).
fn ensure_not_project_newer(
    skill: &project_scanner::ProjectSkillInfo,
    managed: &SkillRecord,
) -> Result<(), AppError> {
    if classify_sync_status(skill, Some(managed)) == "project_newer" {
        return Err(AppError::invalid_input(
            "Project skill is newer than the Skills Center version",
        ));
    }
    Ok(())
}

/// Pull the library version into a vendored copy, replacing it in place so
/// the agent links to it keep resolving.
pub(super) fn update_vendored_from_center(
    vendored: &project_scanner::ProjectSkillInfo,
    all_managed: &[SkillRecord],
) -> Result<(), AppError> {
    let managed = find_best_center_match(vendored, all_managed)
        .ok_or_else(|| AppError::not_found("No matching skill in center"))?;
    ensure_not_project_newer(vendored, managed)?;
    // UserConfirmed for the same reason as a link-mode update: the user asked
    // for this copy to be replaced, and the check above guards their edits.
    sync_engine::sync_skill(
        Path::new(&managed.central_path),
        Path::new(&vendored.path),
        sync_engine::SyncMode::Copy,
        sync_engine::ReplacePolicy::UserConfirmed,
    )
    .map_err(AppError::io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::super::convert_project_to_copy;
    #[cfg(unix)]
    use super::super::test_fixtures::{agent_selection_fixture, update_vendored_x};
    #[cfg(unix)]
    use crate::core::error::ErrorKind;
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::path::Path;
    #[cfg(unix)]
    use tempfile::tempdir;

    /// Pulling from the library replaces only the vendored copy, and in place,
    /// so the agent links keep resolving; an edit made in the repo is never
    /// pulled over.
    #[cfg(unix)]
    #[test]
    fn updating_a_vendored_skill_keeps_its_links_and_refuses_repo_edits() {
        let tmp = tempdir().unwrap();
        let (store, record) = agent_selection_fixture(tmp.path(), None);
        convert_project_to_copy(&store, &record).unwrap();
        let record = store.get_project_by_id(&record.id).unwrap().unwrap();
        let project = Path::new(&record.path);
        let library_md = tmp.path().join("library/x/SKILL.md");
        let vendored_md = project.join(".agents/skills/x/SKILL.md");

        fs::write(&library_md, "---\nname: x\n---\nnew\n").unwrap();
        update_vendored_x(&store, &record).unwrap();

        assert_eq!(
            fs::read_to_string(&vendored_md).unwrap(),
            "---\nname: x\n---\nnew\n"
        );
        assert_eq!(
            fs::read_to_string(project.join(".a/skills/x/SKILL.md")).unwrap(),
            "---\nname: x\n---\nnew\n"
        );

        fs::write(&vendored_md, "edited in the repo").unwrap();
        let an_hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        fs::File::options()
            .write(true)
            .open(&library_md)
            .unwrap()
            .set_modified(an_hour_ago)
            .unwrap();
        let err = update_vendored_x(&store, &record).unwrap_err();

        assert_eq!(err.kind, ErrorKind::InvalidInput);
        assert_eq!(
            fs::read_to_string(&vendored_md).unwrap(),
            "edited in the repo"
        );
    }
}
