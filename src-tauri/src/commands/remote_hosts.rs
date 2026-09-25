//! Remote hosts: the table CRUD, a reachability probe, and the live
//! `serve --stdio` session that host-scoped commands are routed through.

use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tauri::State;

use crate::core::error::AppError;
use crate::core::remote_host;
use crate::core::remote_session::{HostSessionInfo, RemoteSession, RemoteSessions};
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
    Ok((
        name.to_string(),
        ssh_target.to_string(),
        cli_path.map(str::to_string),
    ))
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

/// Connect, read the host's hello and close again. A failure is the reason
/// the host cannot be used, worded for the UI (ssh refused, no CLI, not
/// POSIX, …); a different version is reported, not refused.
#[tauri::command]
pub async fn remote_host_probe(
    host_id: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<RemoteProbe, AppError> {
    let store = store.inner().clone();
    tokio::task::spawn_blocking(move || {
        let host = load_host(&store, &host_id)?;
        let version = RemoteSession::handshake(&remote_host::cli_command, &host)?;
        Ok(probe_result(version))
    })
    .await?
}

/// Both sides must run the same build, as connecting requires.
fn probe_result(version: String) -> RemoteProbe {
    let app_version = env!("CARGO_PKG_VERSION");
    RemoteProbe {
        compatible: version == app_version,
        version,
        app_version: app_version.to_string(),
    }
}

// ── Live session (`serve --stdio`) ──

/// Connect to the host (or reuse the live session) and say who answered.
#[tauri::command]
pub async fn remote_host_connect(
    host_id: String,
    sessions: State<'_, RemoteSessions>,
) -> Result<HostSessionInfo, AppError> {
    Ok(sessions.session_for(&host_id).await?.info().clone())
}

#[tauri::command]
pub async fn remote_host_disconnect(sessions: State<'_, RemoteSessions>) -> Result<(), AppError> {
    sessions.disconnect().await;
    Ok(())
}

/// Run one host-scoped command on the host, connecting first if its session
/// is not live. `args` is the object the command takes locally.
#[tauri::command]
pub async fn remote_invoke(
    host_id: String,
    command: String,
    args: Value,
    sessions: State<'_, RemoteSessions>,
) -> Result<Value, AppError> {
    sessions
        .session_for(&host_id)
        .await?
        .call(&command, args)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_same_version_is_compatible() {
        let same = probe_result(env!("CARGO_PKG_VERSION").to_string());
        assert!(same.compatible);
        assert_eq!(same.version, same.app_version);

        let close = probe_result(concat!(env!("CARGO_PKG_VERSION"), "-dev").to_string());
        assert!(!close.compatible);
        assert_eq!(close.version, concat!(env!("CARGO_PKG_VERSION"), "-dev"));
        assert_eq!(close.app_version, env!("CARGO_PKG_VERSION"));
    }
}
