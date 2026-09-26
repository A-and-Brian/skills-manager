//! Matching a project's skill copy to its library skill, and reading how the
//! two have drifted apart.

use std::path::{Path, PathBuf};

use super::project_scanner;
use super::skill_store::SkillRecord;

pub(crate) fn slugify_skill_dir_name(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in name.chars().flat_map(|c| c.to_lowercase()) {
        let valid = ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.';
        if valid {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches(|c| c == '-' || c == '_' || c == '.');
    if trimmed.is_empty() {
        "skill".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn source_ref_matches_skill_path(
    skill_path: &str,
    skill_canonical: Option<&PathBuf>,
    managed: &SkillRecord,
) -> bool {
    let Some(source_ref) = managed.source_ref.as_deref() else {
        return false;
    };
    if source_ref == skill_path {
        return true;
    }
    let Some(skill_canonical) = skill_canonical else {
        return false;
    };
    let Ok(source_canonical) = std::fs::canonicalize(source_ref) else {
        return false;
    };
    source_canonical == *skill_canonical
}

pub(crate) fn find_best_center_match<'a>(
    skill: &project_scanner::ProjectSkillInfo,
    all_managed: &'a [SkillRecord],
) -> Option<&'a SkillRecord> {
    let skill_hash = skill.content_hash.as_deref();
    let canonical_skill_path = std::fs::canonicalize(&skill.path).ok();

    // source_ref is the strongest direct link there is.
    if let Some(managed) = all_managed.iter().find(|managed| {
        source_ref_matches_skill_path(&skill.path, canonical_skill_path.as_ref(), managed)
    }) {
        return Some(managed);
    }

    // A content hash that names exactly one library skill outranks any
    // directory or name evidence: it says the two directories hold the same
    // bytes, while a directory name only says they were once called the same
    // thing. An export written under a slugified name can land on a directory
    // that now reads as a *different* skill (a library holding both
    // "Code Review" and "code-review" exports the first as `code-review`),
    // and this match also decides where an import writes back — binding the
    // wrong row there overwrites the other skill.
    //
    // Only a unique hash qualifies. Several skills sharing one hash is the
    // arbitrary-pick this fix exists to remove, and those fall through to the
    // directory and name layers below.
    if let Some(hash) = skill_hash {
        let mut by_hash = all_managed
            .iter()
            .filter(|managed| managed.content_hash.as_deref() == Some(hash));
        if let Some(first) = by_hash.next() {
            if by_hash.next().is_none() {
                return Some(first);
            }
        }
    }

    // The central directory name is steadier than the frontmatter name:
    // several skills shipped from one repo can share the latter.
    let by_central_dir: Vec<&SkillRecord> = all_managed
        .iter()
        .filter(|managed| {
            Path::new(&managed.central_path)
                .file_name()
                .map(|name| name.to_string_lossy().eq_ignore_ascii_case(&skill.dir_name))
                .unwrap_or(false)
        })
        .collect();
    if let Some(managed) = unique_center_match(&by_central_dir, skill_hash) {
        return Some(managed);
    }

    // Covers the ordinary skill whose name and directory agree.
    let by_name: Vec<&SkillRecord> = all_managed
        .iter()
        .filter(|managed| {
            slugify_skill_dir_name(&managed.name).eq_ignore_ascii_case(&skill.dir_name)
        })
        .collect();
    if let Some(managed) = unique_center_match(&by_name, skill_hash) {
        return Some(managed);
    }

    None
}

/// Pick the one skill among candidates sharing an identity signal,
/// disambiguating by content hash when the signal alone leaves several.
fn unique_center_match<'a>(
    candidates: &[&'a SkillRecord],
    skill_hash: Option<&str>,
) -> Option<&'a SkillRecord> {
    match candidates.len() {
        0 => None,
        1 => Some(candidates[0]),
        _ => {
            let hash = skill_hash?;
            let mut filtered = candidates
                .iter()
                .copied()
                .filter(|managed| managed.content_hash.as_deref() == Some(hash));
            let first = filtered.next()?;
            filtered.next().is_none().then_some(first)
        }
    }
}

pub(crate) fn classify_sync_status(
    skill: &project_scanner::ProjectSkillInfo,
    managed: Option<&SkillRecord>,
) -> String {
    let Some(managed) = managed else {
        return "project_only".to_string();
    };

    // Fast path: compare project hash against DB-stored center hash
    if skill.content_hash.is_some()
        && managed.content_hash.as_deref() == skill.content_hash.as_deref()
    {
        return "in_sync".to_string();
    }

    // The DB hash may be stale, and `updated_at` is the wrong clock for the
    // comparison further down, so read the center from disk once and answer
    // both questions from the same walk.
    let center_entries =
        crate::core::content_hash::list_content_files(Path::new(&managed.central_path));

    if let Some(project_hash) = skill.content_hash.as_deref() {
        if project_hash == crate::core::content_hash::hash_entries(&center_entries) {
            return "in_sync".to_string();
        }
    }

    let Some(project_modified_at) = skill.last_modified_at else {
        return "diverged".to_string();
    };

    // The project side is a filesystem mtime, so the center has to be one too.
    // `updated_at` is a database column stamped when the row was written:
    // editing files in the library does not move it, and a metadata-only write
    // moves it while no content changed. Comparing the two rulers reported
    // "center is newer" for a project copy the user had just edited, and that
    // status invites a pull, which overwrites the edit — the diagnosis behind
    // #328.
    let Some(center_modified_at) = crate::core::content_hash::latest_modified_ms(&center_entries)
    else {
        return "diverged".to_string();
    };
    let threshold_ms = 1_000;
    if project_modified_at > center_modified_at + threshold_ms {
        "project_newer".to_string()
    } else if center_modified_at > project_modified_at + threshold_ms {
        "center_newer".to_string()
    } else {
        "diverged".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_sync_status, find_best_center_match};
    use crate::core::content_hash;
    use crate::core::project_scanner::ProjectSkillInfo;
    use crate::core::skill_store::SkillRecord;
    use crate::core::test_support::sample_managed_skill;
    use std::fs;
    use tempfile::tempdir;

    /// Build a library skill with the given identity fields, so a test can
    /// assert the order the layers resolve in.
    fn managed_skill_with_identity(
        id: &str,
        name: &str,
        central_path: String,
        content_hash: Option<String>,
    ) -> SkillRecord {
        SkillRecord {
            id: id.to_string(),
            name: name.to_string(),
            description: None,
            source_type: "skillssh".to_string(),
            source_ref: None,
            source_ref_resolved: None,
            source_subpath: None,
            source_branch: None,
            source_revision: None,
            remote_revision: None,
            central_path,
            content_hash,
            enabled: true,
            created_at: 0,
            updated_at: 0,
            status: "ok".to_string(),
            update_status: "unknown".to_string(),
            last_checked_at: None,
            last_check_error: None,
        }
    }

    fn sample_project_skill(
        path: String,
        content_hash: Option<String>,
        last_modified_at: Option<i64>,
    ) -> ProjectSkillInfo {
        ProjectSkillInfo {
            name: "Example Skill".to_string(),
            dir_name: "example-skill".to_string(),
            relative_path: "example-skill".to_string(),
            description: None,
            author: None,
            path,
            files: vec!["SKILL.md".to_string()],
            enabled: true,
            agent: "claude_code".to_string(),
            agent_display_name: "Claude Code".to_string(),
            tags: Vec::new(),
            in_center: true,
            sync_status: "project_only".to_string(),
            center_skill_id: Some("skill-1".to_string()),
            agents_overridden: false,
            alias_of: None,
            vendored: false,
            last_modified_at,
            content_hash,
        }
    }

    /// Build a project skill with the given directory name and agent, to
    /// stand in for one per-agent copy.
    fn project_skill_with_dir(
        dir_name: &str,
        path: String,
        content_hash: Option<String>,
        agent: &str,
    ) -> ProjectSkillInfo {
        ProjectSkillInfo {
            name: dir_name.to_string(),
            dir_name: dir_name.to_string(),
            relative_path: dir_name.to_string(),
            description: None,
            author: None,
            path,
            files: vec!["SKILL.md".to_string()],
            enabled: true,
            agent: agent.to_string(),
            agent_display_name: agent.to_string(),
            tags: Vec::new(),
            in_center: false,
            sync_status: "project_only".to_string(),
            center_skill_id: None,
            agents_overridden: false,
            alias_of: None,
            vendored: false,
            last_modified_at: Some(1_000),
            content_hash,
        }
    }

    /// Directory identity must win over a content hash several skills share.
    #[test]
    fn find_best_center_match_prefers_directory_identity_over_shared_hash() {
        let shared_hash = Some("same-content-hash".to_string());
        let project = project_skill_with_dir(
            "adapt",
            "/tmp/project/.claude/skills/adapt".to_string(),
            shared_hash.clone(),
            "claude_code",
        );
        let all_managed = vec![
            managed_skill_with_identity(
                "adapt-id",
                "adapt",
                "/tmp/center/adapt".to_string(),
                shared_hash.clone(),
            ),
            managed_skill_with_identity(
                "polish-id",
                "polish",
                "/tmp/center/polish".to_string(),
                shared_hash,
            ),
        ];

        let matched = find_best_center_match(&project, &all_managed).unwrap();

        assert_eq!(matched.id, "adapt-id");
    }

    /// The central directory name must win over a frontmatter name that repeats.
    #[test]
    fn find_best_center_match_uses_central_directory_before_frontmatter_name() {
        let project = project_skill_with_dir(
            "adapt",
            "/tmp/project/.claude/skills/adapt".to_string(),
            None,
            "claude_code",
        );
        let all_managed = vec![
            managed_skill_with_identity(
                "adapt-id",
                "impeccable",
                "/tmp/center/adapt".to_string(),
                None,
            ),
            managed_skill_with_identity(
                "layout-id",
                "impeccable",
                "/tmp/center/layout".to_string(),
                None,
            ),
        ];

        let matched = find_best_center_match(&project, &all_managed).unwrap();

        assert_eq!(matched.id, "adapt-id");
    }

    /// A unique content hash outranks a directory name owned by a different
    /// skill. A library holding both "Code Review" and "code-review" exports
    /// the first under the slug `code-review`, which is the second one's
    /// library directory — matching on the directory binds the copy to the
    /// wrong row, and that row is also where an import writes back.
    #[test]
    fn find_best_center_match_prefers_a_unique_hash_over_another_skills_directory() {
        let exported_hash = Some("code-review-content".to_string());
        let project = project_skill_with_dir(
            "code-review",
            "/tmp/project/.claude/skills/code-review".to_string(),
            exported_hash.clone(),
            "claude_code",
        );
        let all_managed = vec![
            managed_skill_with_identity(
                "spaced-id",
                "Code Review",
                "/tmp/center/Code Review".to_string(),
                exported_hash,
            ),
            managed_skill_with_identity(
                "slug-id",
                "code-review",
                "/tmp/center/code-review".to_string(),
                Some("unrelated-content".to_string()),
            ),
        ];

        let matched = find_best_center_match(&project, &all_managed).unwrap();

        assert_eq!(matched.id, "spaced-id");
    }

    #[test]
    fn classify_sync_status_uses_live_center_hash_when_db_hash_is_stale() {
        let center_dir = tempdir().unwrap();
        fs::write(center_dir.path().join("SKILL.md"), "# Example\n").unwrap();
        let live_hash = content_hash::hash_directory(center_dir.path()).unwrap();

        let managed = sample_managed_skill(
            center_dir.path().to_string_lossy().to_string(),
            Some("stale-db-hash".to_string()),
            1_000,
        );
        let project = sample_project_skill(
            center_dir.path().to_string_lossy().to_string(),
            Some(live_hash),
            Some(5_000),
        );

        assert_eq!(classify_sync_status(&project, Some(&managed)), "in_sync");
    }

    /// Newest content mtime of a directory, the same figure the project side
    /// is built from, so both sides of the comparison use one ruler.
    fn center_mtime_ms(dir: &std::path::Path) -> i64 {
        content_hash::latest_modified_ms(&content_hash::list_content_files(dir)).unwrap()
    }

    /// `updated_at` is a database column, not a filesystem mtime. With the
    /// project copy genuinely newer on disk, a much later `updated_at` must not
    /// flip the answer to "center_newer" — that reading invited a pull and
    /// overwrote the edit the user had just made (#328).
    #[test]
    fn classify_sync_status_ignores_the_db_column_when_the_project_is_newer_on_disk() {
        let center_dir = tempdir().unwrap();
        fs::write(center_dir.path().join("SKILL.md"), "# Center\n").unwrap();
        let center_mtime = center_mtime_ms(center_dir.path());

        let project_dir = tempdir().unwrap();
        fs::write(project_dir.path().join("SKILL.md"), "# Project changed\n").unwrap();
        let project_hash = content_hash::hash_directory(project_dir.path()).unwrap();

        let managed = sample_managed_skill(
            center_dir.path().to_string_lossy().to_string(),
            Some("stale-db-hash".to_string()),
            center_mtime + 60_000,
        );
        let project = sample_project_skill(
            project_dir.path().to_string_lossy().to_string(),
            Some(project_hash),
            Some(center_mtime + 5_000),
        );

        assert_eq!(
            classify_sync_status(&project, Some(&managed)),
            "project_newer"
        );
    }

    /// The other direction, and the reason the fix is not simply "always say
    /// project_newer": a center that really is ahead still reports so, with an
    /// `updated_at` old enough that only the real mtime can produce it.
    #[test]
    fn classify_sync_status_reports_a_center_that_is_newer_on_disk() {
        let center_dir = tempdir().unwrap();
        fs::write(center_dir.path().join("SKILL.md"), "# Center\n").unwrap();
        let center_mtime = center_mtime_ms(center_dir.path());

        let project_dir = tempdir().unwrap();
        fs::write(project_dir.path().join("SKILL.md"), "# Project older\n").unwrap();
        let project_hash = content_hash::hash_directory(project_dir.path()).unwrap();

        let managed = sample_managed_skill(
            center_dir.path().to_string_lossy().to_string(),
            Some("stale-db-hash".to_string()),
            0,
        );
        let project = sample_project_skill(
            project_dir.path().to_string_lossy().to_string(),
            Some(project_hash),
            Some(center_mtime - 5_000),
        );

        assert_eq!(
            classify_sync_status(&project, Some(&managed)),
            "center_newer"
        );
    }
}
