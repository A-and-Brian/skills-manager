//! The machine a host-scoped command runs against.
//!
//! Host-scoped command bodies take a `HostCtx` instead of Tauri state, so the
//! desktop app and a headless caller run the same code. Tray refreshes and
//! other app-instance side effects stay in the Tauri wrappers.

use std::sync::Arc;

use serde_json::Value;
use tauri::Emitter;

use crate::core::install_cancel::InstallCancelRegistry;
use crate::core::skill_store::SkillStore;

/// Where a host-scoped command reports progress (`install-progress`,
/// `batch-import-progress`, `app-files-changed`).
pub trait HostEvents: Send + Sync {
    fn emit(&self, event: &str, payload: Value);
}

#[derive(Clone)]
pub struct HostCtx {
    pub store: Arc<SkillStore>,
    pub cancel: Arc<InstallCancelRegistry>,
    pub events: Arc<dyn HostEvents>,
}

/// Events for the desktop app: forwarded to the webview.
pub struct TauriEvents(pub tauri::AppHandle);

impl HostEvents for TauriEvents {
    fn emit(&self, event: &str, payload: Value) {
        if let Err(err) = self.0.emit(event, payload) {
            log::debug!("Failed to emit {event}: {err}");
        }
    }
}

/// Events nobody listens to.
pub struct NoopEvents;

impl HostEvents for NoopEvents {
    fn emit(&self, _event: &str, _payload: Value) {}
}

#[cfg(test)]
impl HostCtx {
    pub(crate) fn for_tests(store: SkillStore, events: Arc<dyn HostEvents>) -> Self {
        Self {
            store: Arc::new(store),
            cancel: Arc::new(InstallCancelRegistry::new()),
            events,
        }
    }
}
