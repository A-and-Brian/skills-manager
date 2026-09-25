//! Project DTOs, and folding a project record into the one the UI lists.

use std::collections::HashMap;

use serde::Serialize;

use super::agents_model::read_workspace_skills;
use crate::core::project_scanner;
use crate::core::project_skill_match::{classify_sync_status, find_best_center_match};
use crate::core::skill_store::{ProjectRecord, SkillRecord};

#[derive(Serialize, Default)]
pub struct SyncHealthDto {
    pub in_sync: usize,
    pub project_newer: usize,
    pub center_newer: usize,
    pub diverged: usize,
    pub project_only: usize,
}

#[derive(Serialize)]
pub struct ProjectDto {
    pub id: String,
    pub name: String,
    pub path: String,
    pub workspace_type: String,
    pub linked_agent_name: Option<String>,
    pub supports_skill_toggle: bool,
    pub sort_order: i32,
    pub skill_count: usize,
    pub sync_health: SyncHealthDto,
    pub created_at: i64,
    pub updated_at: i64,
    /// Agent group keys the project deploys to; `None` means it never chose
    /// and uses every installed and enabled agent.
    pub agent_keys: Option<Vec<String>>,
    /// `"link"` or `"copy"` (skills vendored into `.agents/skills`).
    pub deploy_mode: String,
}

#[derive(Serialize)]
pub struct ProjectSkillDocumentDto {
    pub skill_name: String,
    pub filename: String,
    pub content: String,
}

#[derive(Serialize, Clone)]
pub struct ProjectAgentTargetDto {
    pub key: String,
    pub display_name: String,
    pub enabled: bool,
    pub installed: bool,
    pub is_custom: bool,
    /// Part of the project's agent selection.
    pub selected: bool,
    /// Project-relative skills folder, shared by every agent in the group.
    pub relative_skills_dir: String,
}

/// Convert a project record into its DTO, folding the copies of one logical
/// skill across agents together by relative path.
pub(super) fn project_to_dto(
    rec: &ProjectRecord,
    all_managed: &[SkillRecord],
    configs: &[project_scanner::AgentSkillConfig],
) -> ProjectDto {
    let skills = read_workspace_skills(rec, configs);
    let mut grouped_statuses: HashMap<String, String> = HashMap::new();

    for skill in &skills {
        let matched = find_best_center_match(skill, all_managed);
        let status = classify_sync_status(skill, matched);
        let key = skill.relative_path.to_lowercase();
        let existing = grouped_statuses
            .entry(key)
            .or_insert_with(|| status.clone());
        if sync_status_priority(&status) > sync_status_priority(existing) {
            *existing = status;
        }
    }

    let skill_count = grouped_statuses.len();
    let mut health = SyncHealthDto::default();
    for status in grouped_statuses.values() {
        match status.as_str() {
            "in_sync" => health.in_sync += 1,
            "project_newer" => health.project_newer += 1,
            "center_newer" => health.center_newer += 1,
            "diverged" => health.diverged += 1,
            _ => health.project_only += 1,
        }
    }

    ProjectDto {
        id: rec.id.clone(),
        name: rec.name.clone(),
        path: rec.path.clone(),
        workspace_type: rec.workspace_type.clone(),
        linked_agent_name: rec.linked_agent_name.clone(),
        supports_skill_toggle: rec.workspace_type != "linked" || rec.disabled_path.is_some(),
        sort_order: rec.sort_order,
        skill_count,
        sync_health: health,
        created_at: rec.created_at,
        updated_at: rec.updated_at,
        agent_keys: rec.agent_keys.clone(),
        deploy_mode: rec.deploy_mode.clone(),
    }
}

/// Severity of a sync status, used to reduce one logical skill's per-agent
/// copies to a single verdict: the worst one the group carries.
fn sync_status_priority(status: &str) -> u8 {
    match status {
        "diverged" => 5,
        "project_newer" => 4,
        "center_newer" => 3,
        "project_only" => 2,
        "in_sync" => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::project_to_dto;
    use crate::core::project_scanner::AgentSkillConfig;
    use crate::core::skill_store::ProjectRecord;
    use std::fs;
    use tempfile::tempdir;

    /// The sidebar project count dedupes by logical skill rather than adding
    /// up per-agent copies.
    #[test]
    fn project_to_dto_counts_logical_skills_not_agent_copies() {
        let tmp = tempdir().unwrap();
        let project_path = tmp.path().join("project");
        let claude_skill = project_path.join(".claude/skills/shared-skill");
        let codex_skill = project_path.join(".codex/skills/shared-skill");
        fs::create_dir_all(&claude_skill).unwrap();
        fs::create_dir_all(&codex_skill).unwrap();
        fs::write(claude_skill.join("SKILL.md"), "# Shared\n").unwrap();
        fs::write(codex_skill.join("SKILL.md"), "# Shared\n").unwrap();

        let record = ProjectRecord {
            id: "project-1".to_string(),
            name: "Project".to_string(),
            path: project_path.to_string_lossy().to_string(),
            workspace_type: "project".to_string(),
            linked_agent_key: None,
            linked_agent_name: None,
            disabled_path: None,
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
            agent_keys: None,
            deploy_mode: "link".to_string(),
        };
        let configs = vec![
            AgentSkillConfig {
                key: "claude_code".to_string(),
                display_name: "Claude Code".to_string(),
                relative_skills_dir: ".claude/skills".to_string(),
            },
            AgentSkillConfig {
                key: "codex".to_string(),
                display_name: "Codex".to_string(),
                relative_skills_dir: ".codex/skills".to_string(),
            },
        ];

        let dto = project_to_dto(&record, &[], &configs);

        assert_eq!(dto.skill_count, 1);
        assert_eq!(dto.sync_health.project_only, 1);
    }
}
