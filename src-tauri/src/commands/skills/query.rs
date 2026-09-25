//! Read-only skill queries: listings, documents and source diffs.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use tauri::State;
use walkdir::WalkDir;

use super::types::{
    SkillDocumentDto, SkillSourceDiffDto, SkillSourceDiffEntryDto, SourceSkillDocumentDto,
};

use crate::core::{
    error::AppError,
    git_fetcher,
    host::HostCtx,
    managed_skill::{managed_skill_to_dto, ManagedSkillDto},
    skill_install::resolve_skill_dir,
    skill_source::git_source_from_skill,
    skill_store::SkillRecord,
    timing::should_log_first_or_slow,
};

static GET_MANAGED_SKILLS_FIRST_CALL: AtomicBool = AtomicBool::new(true);

#[tauri::command]
pub async fn get_managed_skills(ctx: State<'_, HostCtx>) -> Result<Vec<ManagedSkillDto>, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || get_managed_skills_core(&ctx)).await?
}

pub fn get_managed_skills_core(ctx: &HostCtx) -> Result<Vec<ManagedSkillDto>, AppError> {
    let store = ctx.store.clone();
    let start = Instant::now();
    let skills = store.get_all_skills().map_err(AppError::db)?;
    let all_targets = store.get_all_targets().map_err(AppError::db)?;
    let tags_map = store.get_tags_map().map_err(AppError::db)?;
    let count = skills.len();
    let dtos: Vec<ManagedSkillDto> = skills
        .into_iter()
        .map(|skill| managed_skill_to_dto(&store, skill, &all_targets, &tags_map))
        .collect();
    let elapsed_ms = start.elapsed().as_millis();
    if should_log_first_or_slow(&GET_MANAGED_SKILLS_FIRST_CALL, elapsed_ms, 100) {
        log::info!("get_managed_skills: {count} skills in {elapsed_ms} ms");
    }
    Ok(dtos)
}

#[tauri::command]
pub async fn get_skills_for_preset(
    preset_id: String,
    ctx: State<'_, HostCtx>,
) -> Result<Vec<ManagedSkillDto>, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || get_skills_for_preset_core(&ctx, preset_id))
        .await?
}

pub fn get_skills_for_preset_core(
    ctx: &HostCtx,
    preset_id: String,
) -> Result<Vec<ManagedSkillDto>, AppError> {
    let store = ctx.store.clone();
    let skills = store
        .get_skills_for_scenario(&preset_id)
        .map_err(AppError::db)?;
    let all_targets = store.get_all_targets().map_err(AppError::db)?;
    let tags_map = store.get_tags_map().map_err(AppError::db)?;

    Ok(skills
        .into_iter()
        .map(|skill| managed_skill_to_dto(&store, skill, &all_targets, &tags_map))
        .collect())
}

#[tauri::command]
pub async fn get_skill_document(
    skill_id: String,
    ctx: State<'_, HostCtx>,
) -> Result<SkillDocumentDto, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || get_skill_document_core(&ctx, skill_id)).await?
}

pub fn get_skill_document_core(
    ctx: &HostCtx,
    skill_id: String,
) -> Result<SkillDocumentDto, AppError> {
    let store = ctx.store.clone();
    let skill = store
        .get_skill_by_id(&skill_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Skill not found"))?;

    let (filename, content) = read_skill_document_from_dir(Path::new(&skill.central_path))?;

    Ok(SkillDocumentDto {
        skill_id,
        filename,
        content,
        central_path: skill.central_path,
    })
}

#[tauri::command]
pub async fn get_source_skill_document(
    skill_id: String,
    ctx: State<'_, HostCtx>,
) -> Result<SourceSkillDocumentDto, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || get_source_skill_document_core(&ctx, skill_id))
        .await?
}

pub fn get_source_skill_document_core(
    ctx: &HostCtx,
    skill_id: String,
) -> Result<SourceSkillDocumentDto, AppError> {
    let store = ctx.store.clone();
    let proxy_url = store.proxy_url();
    let skill = store
        .get_skill_by_id(&skill_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Skill not found"))?;

    if matches!(skill.source_type.as_str(), "local" | "import") {
        let source_path = skill.source_ref.as_ref().ok_or_else(|| {
            AppError::not_found("Local skill is missing its original source path")
        })?;
        let source_dir = PathBuf::from(source_path);
        if !source_dir.exists() {
            return Err(AppError::not_found("Original source path no longer exists"));
        }
        let (filename, content) = read_skill_document_from_dir(&source_dir)?;
        return Ok(SourceSkillDocumentDto {
            skill_id,
            filename,
            content,
            source_label: source_label_for_skill(&skill),
            revision: "workspace".to_string(),
        });
    }

    if !matches!(skill.source_type.as_str(), "git" | "skillssh") {
        return Err(AppError::invalid_input(
            "Skill does not support source diff preview",
        ));
    }

    let git_source = git_source_from_skill(&skill)?;
    git_fetcher::validate_git_url(&git_source.clone_url).map_err(AppError::git)?;
    let remote_revision = git_fetcher::resolve_remote_revision(
        &git_source.clone_url,
        git_source.branch.as_deref(),
        proxy_url.as_deref(),
    )
    .map_err(AppError::git)?;

    let temp_dir = git_fetcher::clone_repo_ref_scoped(
        &git_source.clone_url,
        git_source.branch.as_deref(),
        git_source.subpath.as_deref(),
        None,
        proxy_url.as_deref(),
        None,
    )
    .map_err(AppError::classify_git_error)?;

    let result = (|| -> Result<SourceSkillDocumentDto, AppError> {
        git_fetcher::checkout_revision(&temp_dir, &remote_revision).map_err(AppError::git)?;
        let skill_dir = resolve_skill_dir(
            &temp_dir,
            git_source.subpath.as_deref(),
            git_source.locator_skill_id.as_deref(),
        )?;
        let (filename, content) = read_skill_document_from_dir(&skill_dir)?;

        Ok(SourceSkillDocumentDto {
            skill_id,
            filename,
            content,
            source_label: source_label_for_skill(&skill),
            revision: remote_revision,
        })
    })();

    git_fetcher::cleanup_temp(&temp_dir);
    result
}

/// Files larger than this are flagged but not sent to the frontend — the
/// line diff is O(n²), so previewing a huge file would hang the UI.
const MAX_DIFF_FILE_BYTES: usize = 256 * 1024;

/// Classify a file's bytes for diffing: oversized and binary files get a
/// summary row instead of a text body.
fn classify_diff_bytes(bytes: Option<Vec<u8>>) -> (&'static str, Option<String>) {
    match bytes {
        Some(b) if b.len() > MAX_DIFF_FILE_BYTES => ("too_large", None),
        Some(b) if b.contains(&0) => ("binary", None),
        Some(b) => match String::from_utf8(b) {
            Ok(text) => ("text", Some(text)),
            Err(_) => ("binary", None),
        },
        None => ("binary", None),
    }
}

/// Diff the whole content scope of two skill directories. `original_dir` is
/// the central copy (old), `updated_dir` is the source (new). Uses the same
/// file enumeration as the hash so it reports exactly what flips the badge.
fn build_source_diff_entries(
    original_dir: &Path,
    updated_dir: &Path,
) -> Vec<SkillSourceDiffEntryDto> {
    use crate::core::content_hash::{self, ContentEntry};
    use std::collections::BTreeMap;

    let index = |dir: &Path| -> BTreeMap<String, ContentEntry> {
        content_hash::list_content_files(dir)
            .into_iter()
            .map(|e| (e.relative_path.clone(), e))
            .collect()
    };
    let original = index(original_dir);
    let updated = index(updated_dir);

    let mut keys: Vec<&String> = original.keys().chain(updated.keys()).collect();
    keys.sort();
    keys.dedup();

    let mut entries = Vec::new();
    for key in keys {
        match (original.get(key), updated.get(key)) {
            (None, Some(u)) => {
                let (kind, text) = classify_diff_bytes(std::fs::read(&u.path).ok());
                entries.push(SkillSourceDiffEntryDto {
                    relative_path: key.clone(),
                    status: "added".into(),
                    content_kind: kind.into(),
                    original_text: None,
                    updated_text: text,
                    executable_before: false,
                    executable_after: u.is_executable(),
                });
            }
            (Some(o), None) => {
                let (kind, text) = classify_diff_bytes(std::fs::read(&o.path).ok());
                entries.push(SkillSourceDiffEntryDto {
                    relative_path: key.clone(),
                    status: "removed".into(),
                    content_kind: kind.into(),
                    original_text: text,
                    updated_text: None,
                    executable_before: o.is_executable(),
                    executable_after: false,
                });
            }
            (Some(o), Some(u)) => {
                let o_bytes = std::fs::read(&o.path).ok();
                let u_bytes = std::fs::read(&u.path).ok();
                let exec_before = o.is_executable();
                let exec_after = u.is_executable();
                let bytes_equal = o_bytes.is_some() && o_bytes == u_bytes;

                if bytes_equal {
                    if exec_before == exec_after {
                        continue; // unchanged — must match the hash's verdict
                    }
                    entries.push(SkillSourceDiffEntryDto {
                        relative_path: key.clone(),
                        status: "modified".into(),
                        content_kind: "permission_only".into(),
                        original_text: None,
                        updated_text: None,
                        executable_before: exec_before,
                        executable_after: exec_after,
                    });
                    continue;
                }

                let (o_kind, o_text) = classify_diff_bytes(o_bytes);
                let (u_kind, u_text) = classify_diff_bytes(u_bytes);
                let (kind, original_text, updated_text) = if o_kind == "text" && u_kind == "text" {
                    ("text", o_text, u_text)
                } else if o_kind == "too_large" || u_kind == "too_large" {
                    ("too_large", None, None)
                } else {
                    ("binary", None, None)
                };
                entries.push(SkillSourceDiffEntryDto {
                    relative_path: key.clone(),
                    status: "modified".into(),
                    content_kind: kind.into(),
                    original_text,
                    updated_text,
                    executable_before: exec_before,
                    executable_after: exec_after,
                });
            }
            (None, None) => {}
        }
    }

    entries
}

#[tauri::command]
pub async fn get_skill_source_diff(
    skill_id: String,
    ctx: State<'_, HostCtx>,
) -> Result<SkillSourceDiffDto, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || get_skill_source_diff_core(&ctx, skill_id)).await?
}

pub fn get_skill_source_diff_core(
    ctx: &HostCtx,
    skill_id: String,
) -> Result<SkillSourceDiffDto, AppError> {
    let store = ctx.store.clone();
    let proxy_url = store.proxy_url();
    let skill = store
        .get_skill_by_id(&skill_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Skill not found"))?;

    let central_dir = PathBuf::from(&skill.central_path);
    let source_label = source_label_for_skill(&skill);

    if matches!(skill.source_type.as_str(), "local" | "import") {
        let source_path = skill.source_ref.as_ref().ok_or_else(|| {
            AppError::not_found("Local skill is missing its original source path")
        })?;
        let source_dir = PathBuf::from(source_path);
        if !source_dir.exists() {
            return Err(AppError::not_found("Original source path no longer exists"));
        }
        let entries = build_source_diff_entries(&central_dir, &source_dir);
        return Ok(SkillSourceDiffDto {
            skill_id,
            source_label,
            revision: "workspace".to_string(),
            entries,
        });
    }

    if !matches!(skill.source_type.as_str(), "git" | "skillssh") {
        return Err(AppError::invalid_input(
            "Skill does not support source diff preview",
        ));
    }

    let git_source = git_source_from_skill(&skill)?;
    git_fetcher::validate_git_url(&git_source.clone_url).map_err(AppError::git)?;
    let remote_revision = git_fetcher::resolve_remote_revision(
        &git_source.clone_url,
        git_source.branch.as_deref(),
        proxy_url.as_deref(),
    )
    .map_err(AppError::git)?;

    let temp_dir = git_fetcher::clone_repo_ref_scoped(
        &git_source.clone_url,
        git_source.branch.as_deref(),
        git_source.subpath.as_deref(),
        None,
        proxy_url.as_deref(),
        None,
    )
    .map_err(AppError::classify_git_error)?;

    let result = (|| -> Result<SkillSourceDiffDto, AppError> {
        git_fetcher::checkout_revision(&temp_dir, &remote_revision).map_err(AppError::git)?;
        let skill_dir = resolve_skill_dir(
            &temp_dir,
            git_source.subpath.as_deref(),
            git_source.locator_skill_id.as_deref(),
        )?;
        let entries = build_source_diff_entries(&central_dir, &skill_dir);
        Ok(SkillSourceDiffDto {
            skill_id,
            source_label,
            revision: remote_revision,
            entries,
        })
    })();

    git_fetcher::cleanup_temp(&temp_dir);
    result
}

fn read_skill_document_from_dir(dir: &Path) -> Result<(String, String), AppError> {
    let candidates = [
        "SKILL.md",
        "skill.md",
        "CLAUDE.md",
        "claude.md",
        "README.md",
        "readme.md",
    ];

    for name in &candidates {
        let path = dir.join(name);
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            return Ok((name.to_string(), content));
        }
    }

    for e in WalkDir::new(dir).max_depth(4).into_iter().flatten() {
        let fname = e.file_name().to_string_lossy();
        if candidates.contains(&fname.as_ref()) {
            let content = std::fs::read_to_string(e.path())?;
            return Ok((fname.to_string(), content));
        }
    }

    Err(AppError::not_found("No documentation file found"))
}

fn source_label_for_skill(skill: &SkillRecord) -> String {
    match skill.source_type.as_str() {
        "skillssh" => "skills.sh".to_string(),
        "git" => "Git".to_string(),
        "local" => "Local".to_string(),
        "import" => "Imported".to_string(),
        other => other.to_string(),
    }
}
