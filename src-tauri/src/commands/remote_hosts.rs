//! Remote hosts: the table CRUD plus thin wrappers over the remote CLI. No
//! remote logic lives in the frontend; each command maps one-to-one onto a
//! CLI invocation and returns its JSON payload as-is.

use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tauri::State;

use crate::core::error::AppError;
use crate::core::remote_host;
use crate::core::skill_store::{RemoteHostRecord, SkillStore};

#[derive(Serialize)]
pub struct RemoteProbe {
    pub version: String,
    pub compatible: bool,
    pub app_version: String,
}

fn load_host(store: &SkillStore, host_id: &str) -> Result<RemoteHostRecord, AppError> {
    store
        .get_remote_host(host_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Remote host not found"))
}

/// Trim the form fields once, here, so every stored row is clean.
fn validated_fields(
    name: &str,
    ssh_target: &str,
    cli_path: Option<&str>,
) -> Result<(String, String, Option<String>), AppError> {
    let name = name.trim();
    let ssh_target = ssh_target.trim();
    if name.is_empty() || ssh_target.is_empty() {
        return Err(AppError::invalid_input("Name and SSH target are required"));
    }
    let cli_path = cli_path.map(str::trim).filter(|p| !p.is_empty());
    Ok((name.to_string(), ssh_target.to_string(), cli_path.map(str::to_string)))
}

/// Load the host and run one remote call off the async runtime.
async fn with_host<T, F>(
    store: State<'_, Arc<SkillStore>>,
    host_id: String,
    call: F,
) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&RemoteHostRecord) -> Result<T, AppError> + Send + 'static,
{
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move || {
        let host = load_host(&store, &host_id)?;
        call(&host)
    })
    .await?
}

#[tauri::command]
pub async fn remote_hosts_list(
    store: State<'_, Arc<SkillStore>>,
) -> Result<Vec<RemoteHostRecord>, AppError> {
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move || store.get_all_remote_hosts().map_err(AppError::db)).await?
}

#[tauri::command]
pub async fn remote_host_add(
    name: String,
    ssh_target: String,
    cli_path: Option<String>,
    store: State<'_, Arc<SkillStore>>,
) -> Result<RemoteHostRecord, AppError> {
    let (name, ssh_target, cli_path) = validated_fields(&name, &ssh_target, cli_path.as_deref())?;
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move || {
        let host = RemoteHostRecord {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            ssh_target,
            cli_path,
            created_at: chrono::Utc::now().timestamp_millis(),
        };
        store.insert_remote_host(&host).map_err(AppError::db)?;
        Ok(host)
    })
    .await?
}

#[tauri::command]
pub async fn remote_host_update(
    host_id: String,
    name: String,
    ssh_target: String,
    cli_path: Option<String>,
    store: State<'_, Arc<SkillStore>>,
) -> Result<RemoteHostRecord, AppError> {
    let (name, ssh_target, cli_path) = validated_fields(&name, &ssh_target, cli_path.as_deref())?;
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move || {
        load_host(&store, &host_id)?;
        store
            .update_remote_host(&host_id, &name, &ssh_target, cli_path.as_deref())
            .map_err(AppError::db)?;
        load_host(&store, &host_id)
    })
    .await?
}

#[tauri::command]
pub async fn remote_host_remove(
    host_id: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<(), AppError> {
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move || store.delete_remote_host(&host_id).map_err(AppError::db))
        .await?
}

/// Reachability plus the remote CLI version. A failure is the reason the host
/// cannot be used, worded for the UI (ssh refused, no CLI, not POSIX, …).
#[tauri::command]
pub async fn remote_host_probe(
    host_id: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<RemoteProbe, AppError> {
    with_host(store, host_id, |host| {
        let version = remote_host::version(host)?;
        Ok(RemoteProbe {
            compatible: remote_host::is_compatible(&version),
            version: version.to_string(),
            app_version: remote_host::app_version().to_string(),
        })
    })
    .await
}

#[tauri::command]
pub async fn remote_host_tools(
    host_id: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<Value, AppError> {
    with_host(store, host_id, |host| remote_host::run(host, &["tools", "list"])).await
}

#[tauri::command]
pub async fn remote_host_skills(
    host_id: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<Value, AppError> {
    with_host(store, host_id, |host| remote_host::run(host, &["skills", "list"])).await
}

#[tauri::command]
pub async fn remote_host_deploy(
    host_id: String,
    skill_ref: String,
    agent: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<Value, AppError> {
    with_host(store, host_id, move |host| {
        remote_host::run_write(host, &["skills", "deploy", &skill_ref, "--agent", &agent])
    })
    .await
}

#[tauri::command]
pub async fn remote_host_undeploy(
    host_id: String,
    skill_ref: String,
    agent: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<Value, AppError> {
    with_host(store, host_id, move |host| {
        remote_host::run_write(host, &["skills", "undeploy", &skill_ref, "--agent", &agent])
    })
    .await
}

/// Install from a Git URL or a skills.sh shorthand; the remote fetches the
/// source itself. `source_type` picks the CLI flag so a shorthand that looks
/// like a path is never guessed at.
#[tauri::command]
pub async fn remote_host_install(
    host_id: String,
    source: String,
    source_type: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<Value, AppError> {
    let flag = match source_type.as_str() {
        "git" => "--git",
        "skillssh" => "--skillssh",
        _ => {
            return Err(AppError::invalid_input(
                "Only skills with a Git or skills.sh source can be installed on a remote host",
            ))
        }
    };
    with_host(store, host_id, move |host| {
        remote_host::run_write(host, &["skills", "install", &source, flag])
    })
    .await
}

#[tauri::command]
pub async fn remote_host_update_skill(
    host_id: String,
    skill_ref: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<Value, AppError> {
    with_host(store, host_id, move |host| {
        remote_host::run_write(host, &["skills", "update", &skill_ref])
    })
    .await
}
