//! `skills adopt`: bring skills already in agent folders into the central library.

use std::path::{Path, PathBuf};

use anyhow::bail;
use app_lib::core::{
    central_repo, git_fetcher, installer, repo_lock::RepoLock, skill_install, skill_metadata,
    skill_store::SkillStore,
};

use crate::output::map_app_err;
use crate::reports::{AdoptCandidate, AdoptReport, InstallReport};
use crate::skills::install::expand_path;

pub(crate) fn run_adopt(
    store: &SkillStore,
    paths: &[PathBuf],
    git_url: Option<&str>,
    git_subpath: Option<&str>,
    dry_run: bool,
) -> anyhow::Result<AdoptReport> {
    if paths.is_empty() {
        bail!("provide at least one path to scan");
    }
    if git_url.is_some() && paths.len() != 1 {
        bail!("--git-url requires exactly one path");
    }
    if git_subpath.is_some() && git_url.is_none() {
        bail!("--git-subpath requires --git-url");
    }

    // Resolve the source subpath for git-based adopts up front so we fail fast
    // before any filesystem work. parse_git_source pulls a subpath out of GitHub
    // /tree/branch/path URLs; --git-subpath is the explicit override (pass ""
    // to mean "skill lives at the repo root").
    #[allow(clippy::type_complexity)]
    let resolved_git: Option<(String, Option<String>, Option<String>, Option<String>)> =
        if let Some(url) = git_url {
            git_fetcher::validate_git_url(url)?;
            let parsed = git_fetcher::parse_git_source(url);
            let subpath = match git_subpath {
                Some(s) => {
                    if s.is_empty() {
                        None
                    } else {
                        Some(s.to_string())
                    }
                }
                None => parsed.subpath.clone(),
            };
            if subpath.is_none() && git_subpath.is_none() {
                bail!(
                    "--git-url has no subpath and --git-subpath was not provided. \
                     Pass --git-subpath \"\" if the skill lives at the repo root, \
                     --git-subpath <path> for a subdirectory, or use a URL like \
                     https://github.com/owner/repo/tree/branch/path/to/skill"
                );
            }
            Some((
                parsed.clone_url,
                subpath,
                parsed.branch,
                Some(url.to_string()),
            ))
        } else {
            None
        };

    // Build exclusion set: existing central paths, sync target paths, canonicals
    let mut excluded: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for skill in store.get_all_skills()? {
        let p = PathBuf::from(&skill.central_path);
        excluded.insert(p.clone());
        if let Ok(c) = p.canonicalize() {
            excluded.insert(c);
        }
    }
    for target in store.get_all_targets()? {
        let p = PathBuf::from(&target.target_path);
        excluded.insert(p.clone());
        if let Ok(c) = p.canonicalize() {
            excluded.insert(c);
        }
    }
    let central_root = central_repo::skills_dir();
    let central_root_canonical = central_root.canonicalize().unwrap_or(central_root.clone());

    let mut candidates: Vec<AdoptCandidate> = Vec::new();
    let mut skipped: Vec<AdoptCandidate> = Vec::new();

    for path in paths {
        let path = expand_path(&path.to_string_lossy())?;
        if !path.is_dir() {
            skipped.push(AdoptCandidate {
                path: path.to_string_lossy().to_string(),
                name: String::new(),
                reason: "not a directory".to_string(),
            });
            continue;
        }

        // If the user pointed directly at a single skill dir, treat it as one
        // candidate rather than scanning its children (which would be the
        // skill's own files/references and miss the SKILL.md at the root).
        if skill_metadata::is_valid_skill_dir(&path) {
            classify_adopt_candidate(
                &path,
                false, // path itself can't be a symlink-into-central in this branch
                &excluded,
                &central_root_canonical,
                &mut candidates,
                &mut skipped,
            );
            continue;
        }

        for entry in std::fs::read_dir(&path)? {
            let entry = entry?;
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let is_symlink = entry.file_type()?.is_symlink();
            classify_adopt_candidate(
                &dir,
                is_symlink,
                &excluded,
                &central_root_canonical,
                &mut candidates,
                &mut skipped,
            );
        }
    }

    if dry_run {
        return Ok(AdoptReport {
            ok: true,
            dry_run: true,
            adopted: Vec::new(),
            candidates,
            skipped,
        });
    }

    if git_url.is_some() && candidates.len() != 1 {
        bail!(
            "--git-url requires exactly one adoptable skill, found {}",
            candidates.len()
        );
    }

    let mut adopted = Vec::new();
    for c in &candidates {
        let dir = PathBuf::from(&c.path);
        let _lock = RepoLock::acquire_foreground("cli adopt")?;
        let result = installer::install_from_local(&dir, None)?;
        let metadata = if let Some((clone_url, subpath, branch, original_url)) = &resolved_git {
            skill_install::InstallSourceMetadata {
                source_type: "git".to_string(),
                source_ref: original_url.clone(),
                source_ref_resolved: Some(clone_url.clone()),
                source_subpath: subpath.clone(),
                source_branch: branch.clone(),
                source_revision: None,
                remote_revision: None,
                update_status: "unknown".to_string(),
            }
        } else {
            skill_install::InstallSourceMetadata {
                source_type: "local".to_string(),
                source_ref: Some(dir.to_string_lossy().to_string()),
                source_ref_resolved: None,
                source_subpath: None,
                source_branch: None,
                source_revision: None,
                remote_revision: None,
                update_status: "local_only".to_string(),
            }
        };
        let central_path = result.central_path.to_string_lossy().to_string();
        let install_name = result.name.clone();
        let source_type = metadata.source_type.clone();
        let skill_id =
            skill_install::store_installed_skill_unlocked(store, &result, &metadata, None)
                .map_err(map_app_err)?;
        adopted.push(InstallReport {
            ok: true,
            skill_id,
            name: install_name,
            central_path,
            source_type,
            synced: false,
            preset_id: None,
        });
    }

    Ok(AdoptReport {
        ok: true,
        dry_run: false,
        adopted,
        candidates: Vec::new(),
        skipped,
    })
}

fn classify_adopt_candidate(
    dir: &Path,
    is_symlink: bool,
    excluded: &std::collections::HashSet<PathBuf>,
    central_root_canonical: &Path,
    candidates: &mut Vec<AdoptCandidate>,
    skipped: &mut Vec<AdoptCandidate>,
) {
    let canonical = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    if excluded.contains(dir) || excluded.contains(&canonical) {
        skipped.push(AdoptCandidate {
            path: dir.to_string_lossy().to_string(),
            name,
            reason: "already managed (in DB or sync target)".to_string(),
        });
        return;
    }

    if is_symlink && canonical.starts_with(central_root_canonical) {
        skipped.push(AdoptCandidate {
            path: dir.to_string_lossy().to_string(),
            name,
            reason: "symlink into central repo (already managed)".to_string(),
        });
        return;
    }

    if !skill_metadata::is_valid_skill_dir(dir) {
        skipped.push(AdoptCandidate {
            path: dir.to_string_lossy().to_string(),
            name,
            reason: "no SKILL.md / skill.md".to_string(),
        });
        return;
    }

    candidates.push(AdoptCandidate {
        path: dir.to_string_lossy().to_string(),
        name,
        reason: "ready".to_string(),
    });
}
