//! DTOs returned by the skills commands, and the install cancel guard.

use serde::Serialize;
use std::sync::Arc;

use crate::core::install_cancel::InstallCancelRegistry;

#[derive(Debug, Serialize)]
pub struct BatchUpdateSkillsResult {
    pub refreshed: usize,
    pub unchanged: usize,
    pub failed: Vec<String>,
    /// Skills left alone because updating would have removed files the new
    /// version does not have. Named so the user can go and look, rather than
    /// wondering why the badge did not clear.
    pub held_back: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SkillDocumentDto {
    pub skill_id: String,
    pub filename: String,
    pub content: String,
    pub central_path: String,
}

#[derive(Debug, Serialize)]
pub struct SourceSkillDocumentDto {
    pub skill_id: String,
    pub filename: String,
    pub content: String,
    pub source_label: String,
    pub revision: String,
}

/// Whole-directory diff between the central copy (`original`) and the source
/// (`updated`), covering the same file scope that drives the update badge so
/// the diff can never come back empty while the badge says "update available".
#[derive(Debug, Serialize)]
pub struct SkillSourceDiffDto {
    pub skill_id: String,
    pub source_label: String,
    pub revision: String,
    pub entries: Vec<SkillSourceDiffEntryDto>,
}

#[derive(Debug, Serialize)]
pub struct SkillSourceDiffEntryDto {
    pub relative_path: String,
    /// "added" | "removed" | "modified"
    pub status: String,
    /// "text" | "binary" | "too_large" | "permission_only"
    pub content_kind: String,
    /// Present only when `content_kind == "text"`.
    pub original_text: Option<String>,
    pub updated_text: Option<String>,
    pub executable_before: bool,
    pub executable_after: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct GitSkillPreview {
    /// Path relative to the resolved scan root, using `/` separators. Stable key.
    pub rel_path: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct GitPreviewResult {
    pub temp_dir: String,
    pub skills: Vec<GitSkillPreview>,
}

#[derive(Debug, serde::Deserialize)]
pub struct SkillInstallItem {
    pub rel_path: String,
    pub name: String,
}

pub(super) struct CancelRegistrationGuard {
    registry: Arc<InstallCancelRegistry>,
    key: String,
}

impl CancelRegistrationGuard {
    pub(super) fn new(registry: Arc<InstallCancelRegistry>, key: String) -> Self {
        Self { registry, key }
    }
}

impl Drop for CancelRegistrationGuard {
    fn drop(&mut self) {
        self.registry.remove(&self.key);
    }
}

#[derive(Debug, Serialize)]
pub struct BatchImportResult {
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}
