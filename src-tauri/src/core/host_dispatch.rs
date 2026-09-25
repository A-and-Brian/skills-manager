//! Runs a host-scoped command by name, outside Tauri.
//!
//! `args` is the same camelCase object the UI passes to `invoke`, read the way
//! Tauri reads it: a missing required key or a value of the wrong type is
//! `invalid_input`, a missing or null optional key is `None`, and unknown keys
//! are ignored. A name outside [`COMMANDS`] is `not_found`.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::commands::{agent_workspace, presets, scan, settings, skills, sync, tools};
use crate::core::error::AppError;
use crate::core::host::HostCtx;

/// Every command [`dispatch`] accepts.
pub const COMMANDS: &[&str] = &[
    // Tools
    "get_tool_status",
    "set_tool_enabled",
    "set_all_tools_enabled",
    "get_tool_order_cmd",
    "set_tool_order_cmd",
    "set_custom_tool_path",
    "reset_custom_tool_path",
    "set_custom_tool_project_path",
    "reset_custom_tool_project_path",
    "add_custom_tool",
    "remove_custom_tool",
    // Skills
    "get_managed_skills",
    "get_skills_for_preset",
    "get_skill_document",
    "get_source_skill_document",
    "get_skill_source_diff",
    "delete_managed_skill",
    "delete_managed_skills",
    "install_local",
    "install_git",
    "preview_git_install",
    "confirm_git_install",
    "cancel_git_preview",
    "install_from_skillssh",
    "check_skill_update",
    "check_all_skill_updates",
    "update_skill",
    "batch_update_skills",
    "reimport_local_skill",
    "relink_local_skill_source",
    "detach_local_skill_source",
    "get_all_tags",
    "set_skill_tags",
    "rename_tag",
    "delete_tag",
    "cancel_install",
    "batch_import_folder",
    // Sync
    "sync_skill_to_tool",
    "unsync_skill_from_tool",
    "get_skill_tool_toggles",
    "set_skill_tool_toggle",
    // Scan
    "scan_local_skills",
    "import_existing_skill",
    "import_all_discovered",
    // Agent local workspace
    "get_global_local_skills",
    "get_global_local_skill_document",
    "import_global_local_skill_to_center",
    "update_global_local_skill_from_center",
    "delete_global_local_skill",
    // Presets
    "get_presets",
    "get_active_preset",
    "create_preset",
    "update_preset",
    "delete_preset",
    "switch_preset",
    "apply_preset_to_default",
    "apply_preset_to_coding_agents",
    "add_skill_to_preset",
    "remove_skill_from_preset",
    "reorder_presets",
    "get_preset_skill_order",
    "reorder_preset_skills",
    // Settings
    "get_settings",
    "set_settings",
    "get_central_repo_path",
    "get_central_repo_path_override",
    "get_central_repo_warnings",
    "set_central_repo_path",
];

/// Settings that describe the app you sit at rather than the machine it
/// manages: window, tray, language and backup. They are never written on a host.
const LOCAL_ONLY_SETTINGS: &[&str] = &[
    "theme",
    "language",
    "text_size",
    "close_action",
    "show_tray_icon",
    "merge_engine",
];
const LOCAL_ONLY_SETTING_PREFIXES: &[&str] = &["backup_", "git_backup_", "github_"];

pub fn is_local_only_setting(key: &str) -> bool {
    LOCAL_ONLY_SETTINGS.contains(&key)
        || LOCAL_ONLY_SETTING_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(prefix))
}

struct Args<'a>(&'a Value);

impl Args<'_> {
    fn req<T: DeserializeOwned>(&self, key: &str) -> Result<T, AppError> {
        let value = self
            .0
            .get(key)
            .ok_or_else(|| AppError::invalid_input(format!("Missing argument: {key}")))?;
        T::deserialize(value)
            .map_err(|e| AppError::invalid_input(format!("Invalid argument {key}: {e}")))
    }

    fn opt<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, AppError> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(_) => self.req(key).map(Some),
        }
    }
}

fn reply<T: Serialize>(result: Result<T, AppError>) -> Result<Value, AppError> {
    serde_json::to_value(result?).map_err(AppError::internal)
}

pub fn dispatch(ctx: &HostCtx, command: &str, args: &Value) -> Result<Value, AppError> {
    let a = Args(args);
    match command {
        // Tools
        "get_tool_status" => reply(tools::get_tool_status_core(ctx)),
        "set_tool_enabled" => reply(tools::set_tool_enabled_core(
            ctx,
            a.req("key")?,
            a.req("enabled")?,
        )),
        "set_all_tools_enabled" => reply(tools::set_all_tools_enabled_core(ctx, a.req("enabled")?)),
        "get_tool_order_cmd" => reply(tools::get_tool_order_cmd_core(ctx)),
        "set_tool_order_cmd" => reply(tools::set_tool_order_cmd_core(ctx, a.req("order")?)),
        "set_custom_tool_path" => reply(tools::set_custom_tool_path_core(
            ctx,
            a.req("key")?,
            a.req("path")?,
        )),
        "reset_custom_tool_path" => reply(tools::reset_custom_tool_path_core(ctx, a.req("key")?)),
        "set_custom_tool_project_path" => reply(tools::set_custom_tool_project_path_core(
            ctx,
            a.req("key")?,
            a.opt("projectRelativeSkillsDir")?,
        )),
        "reset_custom_tool_project_path" => reply(tools::reset_custom_tool_project_path_core(
            ctx,
            a.req("key")?,
        )),
        "add_custom_tool" => reply(tools::add_custom_tool_core(
            ctx,
            a.req("key")?,
            a.req("displayName")?,
            a.req("skillsDir")?,
            a.opt("projectRelativeSkillsDir")?,
        )),
        "remove_custom_tool" => reply(tools::remove_custom_tool_core(ctx, a.req("key")?)),
        // Skills
        "get_managed_skills" => reply(skills::get_managed_skills_core(ctx)),
        "get_skills_for_preset" => {
            reply(skills::get_skills_for_preset_core(ctx, a.req("presetId")?))
        }
        "get_skill_document" => reply(skills::get_skill_document_core(ctx, a.req("skillId")?)),
        "get_source_skill_document" => reply(skills::get_source_skill_document_core(
            ctx,
            a.req("skillId")?,
        )),
        "get_skill_source_diff" => {
            reply(skills::get_skill_source_diff_core(ctx, a.req("skillId")?))
        }
        "delete_managed_skill" => reply(skills::delete_managed_skill_core(ctx, a.req("skillId")?)),
        "delete_managed_skills" => {
            reply(skills::delete_managed_skills_core(ctx, a.req("skillIds")?))
        }
        "install_local" => reply(skills::install_local_core(
            ctx,
            a.req("sourcePath")?,
            a.opt("name")?,
        )),
        "install_git" => reply(skills::install_git_core(
            ctx,
            a.req("repoUrl")?,
            a.opt("name")?,
        )),
        "preview_git_install" => reply(skills::preview_git_install_core(ctx, a.req("repoUrl")?)),
        "confirm_git_install" => reply(skills::confirm_git_install_core(
            ctx,
            a.req("repoUrl")?,
            a.req("tempDir")?,
            a.req("items")?,
        )),
        "cancel_git_preview" => reply(skills::cancel_git_preview_core(a.req("tempDir")?)),
        "install_from_skillssh" => reply(skills::install_from_skillssh_core(
            ctx,
            a.req("source")?,
            a.req("skillId")?,
        )),
        "check_skill_update" => reply(skills::check_skill_update_core(
            ctx,
            a.req("skillId")?,
            a.opt("force")?,
        )),
        "check_all_skill_updates" => {
            reply(skills::check_all_skill_updates_core(ctx, a.opt("force")?))
        }
        "update_skill" => reply(skills::update_skill_core(
            ctx,
            a.req("skillId")?,
            a.opt("approvedRemovals")?,
        )),
        "batch_update_skills" => reply(skills::batch_update_skills_core(ctx, a.req("skillIds")?)),
        "reimport_local_skill" => reply(skills::reimport_local_skill_core(
            ctx,
            a.req("skillId")?,
            a.opt("approvedRemovals")?,
        )),
        "relink_local_skill_source" => reply(skills::relink_local_skill_source_core(
            ctx,
            a.req("skillId")?,
            a.req("sourcePath")?,
            a.opt("approvedRemovals")?,
        )),
        "detach_local_skill_source" => reply(skills::detach_local_skill_source_core(
            ctx,
            a.req("skillId")?,
        )),
        "get_all_tags" => reply(skills::get_all_tags_core(ctx)),
        "set_skill_tags" => reply(skills::set_skill_tags_core(
            ctx,
            a.req("skillId")?,
            a.req("tags")?,
        )),
        "rename_tag" => reply(skills::rename_tag_core(
            ctx,
            a.req("oldName")?,
            a.req("newName")?,
        )),
        "delete_tag" => reply(skills::delete_tag_core(ctx, a.req("name")?)),
        "cancel_install" => reply(skills::cancel_install_core(ctx, a.req("key")?)),
        "batch_import_folder" => reply(skills::batch_import_folder_core(ctx, a.req("folderPath")?)),
        // Sync
        "sync_skill_to_tool" => reply(sync::sync_skill_to_tool_core(
            ctx,
            a.req("skillId")?,
            a.req("tool")?,
        )),
        "unsync_skill_from_tool" => reply(sync::unsync_skill_from_tool_core(
            ctx,
            a.req("skillId")?,
            a.req("tool")?,
        )),
        "get_skill_tool_toggles" => reply(sync::get_skill_tool_toggles_core(
            ctx,
            a.req("skillId")?,
            a.req("presetId")?,
        )),
        "set_skill_tool_toggle" => reply(sync::set_skill_tool_toggle_core(
            ctx,
            a.req("skillId")?,
            a.req("presetId")?,
            a.req("tool")?,
            a.req("enabled")?,
        )),
        // Scan
        "scan_local_skills" => reply(scan::scan_local_skills_core(ctx)),
        "import_existing_skill" => reply(scan::import_existing_skill_core(
            ctx,
            a.req("sourcePath")?,
            a.opt("name")?,
        )),
        "import_all_discovered" => reply(scan::import_all_discovered_core(ctx)),
        // Agent local workspace
        "get_global_local_skills" => reply(agent_workspace::get_global_local_skills_core(
            ctx,
            a.req("agent")?,
        )),
        "get_global_local_skill_document" => {
            reply(agent_workspace::get_global_local_skill_document_core(
                ctx,
                a.req("agent")?,
                a.req("skillRelativePath")?,
            ))
        }
        "import_global_local_skill_to_center" => {
            reply(agent_workspace::import_global_local_skill_to_center_core(
                ctx,
                a.req("agent")?,
                a.req("skillRelativePath")?,
            ))
        }
        "update_global_local_skill_from_center" => {
            reply(agent_workspace::update_global_local_skill_from_center_core(
                ctx,
                a.req("agent")?,
                a.req("skillRelativePath")?,
            ))
        }
        "delete_global_local_skill" => reply(agent_workspace::delete_global_local_skill_core(
            ctx,
            a.req("agent")?,
            a.req("skillRelativePath")?,
        )),
        // Presets
        "get_presets" => reply(presets::get_presets_core(ctx)),
        "get_active_preset" => reply(presets::get_active_preset_core(ctx)),
        "create_preset" => reply(presets::create_preset_core(
            ctx,
            a.req("name")?,
            a.opt("description")?,
            a.opt("icon")?,
        )),
        "update_preset" => reply(presets::update_preset_core(
            ctx,
            a.req("id")?,
            a.req("name")?,
            a.opt("description")?,
            a.opt("icon")?,
        )),
        "delete_preset" => reply(presets::delete_preset_core(ctx, a.req("id")?)),
        "switch_preset" | "apply_preset_to_default" => {
            reply(presets::apply_preset_to_default_core(ctx, a.req("id")?))
        }
        "apply_preset_to_coding_agents" => reply(presets::apply_preset_to_coding_agents_core(
            ctx,
            a.req("presetId")?,
            a.req("mode")?,
        )),
        "add_skill_to_preset" => reply(presets::add_skill_to_preset_core(
            ctx,
            a.req("skillId")?,
            a.req("presetId")?,
        )),
        "remove_skill_from_preset" => reply(presets::remove_skill_from_preset_core(
            ctx,
            a.req("skillId")?,
            a.req("presetId")?,
        )),
        "reorder_presets" => reply(presets::reorder_presets_core(ctx, a.req("ids")?)),
        "get_preset_skill_order" => reply(presets::get_preset_skill_order_core(
            ctx,
            a.req("presetId")?,
        )),
        "reorder_preset_skills" => reply(presets::reorder_preset_skills_core(
            ctx,
            a.req("presetId")?,
            a.req("skillIds")?,
        )),
        // Settings
        "get_settings" => reply(settings::get_settings_core(ctx, a.req("key")?)),
        "set_settings" => {
            let key: String = a.req("key")?;
            if is_local_only_setting(&key) {
                return Err(AppError::invalid_input(format!(
                    "Setting {key} belongs to this computer and is not changed on a host"
                )));
            }
            reply(settings::set_settings_core(ctx, key, a.req("value")?))
        }
        "get_central_repo_path" => reply(Ok(settings::get_central_repo_path())),
        "get_central_repo_path_override" => reply(Ok(settings::get_central_repo_path_override())),
        "get_central_repo_warnings" => reply(Ok(settings::get_central_repo_warnings())),
        "set_central_repo_path" => reply(settings::set_central_repo_path_core(a.opt("path")?)),
        _ => Err(AppError::not_found(format!("Unknown command: {command}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::error::ErrorKind;
    use crate::core::host::{HostEvents, NoopEvents, RecordingEvents};
    use crate::core::skill_store::SkillStore;
    use serde_json::json;
    use std::sync::{Arc, MutexGuard};
    use tempfile::TempDir;

    /// A host on a temp store and a temp central repo. The repo override is
    /// process-wide, so the guard is held for the test's lifetime.
    struct TestHost {
        ctx: HostCtx,
        tmp: TempDir,
        _lock: MutexGuard<'static, ()>,
    }

    impl Drop for TestHost {
        fn drop(&mut self) {
            crate::core::central_repo::set_test_base_dir_override(None);
        }
    }

    fn test_host() -> TestHost {
        test_host_with(Arc::new(NoopEvents))
    }

    fn test_host_with(events: Arc<dyn HostEvents>) -> TestHost {
        let lock = crate::core::central_repo::test_base_dir_lock();
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("repo");
        crate::core::central_repo::set_test_base_dir_override(Some(base.clone()));
        std::fs::create_dir_all(crate::core::central_repo::skills_dir()).unwrap();
        let store = SkillStore::new(&base.join("test.db")).unwrap();
        TestHost {
            ctx: HostCtx::for_tests(store, events),
            tmp,
            _lock: lock,
        }
    }

    fn is_unknown_command(err: &AppError) -> bool {
        err.kind == ErrorKind::NotFound && err.message.starts_with("Unknown command")
    }

    /// Every listed command reaches its own arm. Each runs on a fresh temp
    /// host so one command's writes can't feed the next (a scan followed by
    /// import-all would copy this machine's agent skills). The mistyped
    /// `path` stops path-taking commands before they touch the disk —
    /// `set_central_repo_path` would otherwise rewrite this user's config.
    #[test]
    fn every_listed_command_is_dispatched() {
        for command in COMMANDS {
            let host = test_host();
            if let Err(err) = dispatch(&host.ctx, command, &json!({ "path": 0 })) {
                assert!(
                    !is_unknown_command(&err),
                    "{command} is listed but not routed"
                );
            }
        }
    }

    #[test]
    fn unknown_command_is_not_found() {
        let host = test_host();
        let err = dispatch(&host.ctx, "git_backup_push", &json!({})).unwrap_err();
        assert!(is_unknown_command(&err));
    }

    #[test]
    fn missing_or_mistyped_argument_is_invalid_input() {
        let host = test_host();
        let missing =
            dispatch(&host.ctx, "set_tool_enabled", &json!({ "key": "claude" })).unwrap_err();
        assert_eq!(missing.kind, ErrorKind::InvalidInput);
        assert_eq!(missing.message, "Missing argument: enabled");

        let mistyped = dispatch(
            &host.ctx,
            "set_tool_enabled",
            &json!({ "key": "claude", "enabled": "yes" }),
        )
        .unwrap_err();
        assert_eq!(mistyped.kind, ErrorKind::InvalidInput);
        assert!(mistyped.message.starts_with("Invalid argument enabled"));
    }

    #[test]
    fn get_tool_status_lists_the_builtin_agents() {
        let host = test_host();
        let result = dispatch(&host.ctx, "get_tool_status", &json!({})).unwrap();
        let keys: Vec<&str> = result
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["key"].as_str().unwrap())
            .collect();
        assert!(keys.contains(&"claude_code"), "got {keys:?}");
    }

    #[test]
    fn created_preset_is_listed() {
        let host = test_host();
        let created = dispatch(
            &host.ctx,
            "create_preset",
            &json!({ "name": "Work", "description": null }),
        )
        .unwrap();
        assert_eq!(created["name"], "Work");

        let presets = dispatch(&host.ctx, "get_presets", &json!({})).unwrap();
        let listed = presets.as_array().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["id"], created["id"]);
    }

    #[test]
    fn host_settings_round_trip_and_local_ones_are_refused() {
        let host = test_host();
        dispatch(
            &host.ctx,
            "set_settings",
            &json!({ "key": "sync_mode", "value": "copy" }),
        )
        .unwrap();
        let value = dispatch(&host.ctx, "get_settings", &json!({ "key": "sync_mode" })).unwrap();
        assert_eq!(value, "copy");

        for key in [
            "theme",
            "show_tray_icon",
            "git_backup_remote_url",
            "github_auth_method",
        ] {
            let err = dispatch(
                &host.ctx,
                "set_settings",
                &json!({ "key": key, "value": "x" }),
            )
            .unwrap_err();
            assert_eq!(err.kind, ErrorKind::InvalidInput, "{key}");
            assert_eq!(host.ctx.store.get_setting(key).unwrap(), None, "{key}");
        }
    }

    #[test]
    fn batch_import_reports_progress_per_skill() {
        let events = Arc::new(RecordingEvents::default());
        let host = test_host_with(events.clone());
        let folder = host.tmp.path().join("import");
        for name in ["alpha", "beta"] {
            let dir = folder.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: test\n---\n"),
            )
            .unwrap();
        }

        let result = dispatch(
            &host.ctx,
            "batch_import_folder",
            &json!({ "folderPath": folder.to_string_lossy() }),
        )
        .unwrap();
        assert_eq!(result["imported"], 2);

        let recorded = events.0.lock().unwrap();
        let progress: Vec<(u64, u64)> = recorded
            .iter()
            .filter(|(event, _)| event == "batch-import-progress")
            .map(|(_, p)| (p["current"].as_u64().unwrap(), p["total"].as_u64().unwrap()))
            .collect();
        assert_eq!(progress, vec![(1, 2), (2, 2)]);
    }
}
