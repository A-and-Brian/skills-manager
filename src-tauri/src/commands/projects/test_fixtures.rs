//! Fixtures shared by the project command tests.

use std::fs;
use std::path::Path;

#[cfg(unix)]
use super::agents_model::{agent_skill_configs, read_workspace_skills, vendored_variant};
#[cfg(unix)]
use super::update_vendored_from_center;
#[cfg(unix)]
use crate::core::error::AppError;
use crate::core::skill_store::{ProjectRecord, SkillStore};
use crate::core::test_support::sample_managed_skill;

/// A store with two custom agents, `agent_a` (`.a/skills`) and `agent_b`
/// (`.b/skills`), a library skill `x`, and a project holding `x` for
/// agent_a as a link into the library.
pub(super) fn agent_selection_fixture(
    tmp: &Path,
    agent_keys: Option<Vec<String>>,
) -> (SkillStore, ProjectRecord) {
    let store = SkillStore::new(&tmp.join("test.db")).unwrap();
    let tools = serde_json::json!([
        { "key": "agent_a", "display_name": "Agent A", "skills_dir": tmp.join("a"),
          "project_relative_skills_dir": ".a/skills" },
        { "key": "agent_b", "display_name": "Agent B", "skills_dir": tmp.join("b"),
          "project_relative_skills_dir": ".b/skills" },
    ]);
    store
        .set_setting("custom_tools", &tools.to_string())
        .unwrap();

    let library = tmp.join("library").join("x");
    fs::create_dir_all(&library).unwrap();
    fs::write(library.join("SKILL.md"), "---\nname: x\n---\n").unwrap();
    store
        .insert_skill(&sample_managed_skill(
            library.to_string_lossy().to_string(),
            None,
            0,
        ))
        .unwrap();

    let project_path = tmp.join("project");
    fs::create_dir_all(project_path.join(".a/skills")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&library, project_path.join(".a/skills/x")).unwrap();

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
        agent_keys,
        deploy_mode: "link".to_string(),
    };
    store.insert_project(&record).unwrap();
    (store, record)
}

pub(super) fn keys(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

#[cfg(unix)]
/// Update `x` the way the command does when asked through agent_a's link.
pub(super) fn update_vendored_x(
    store: &SkillStore,
    record: &ProjectRecord,
) -> Result<(), AppError> {
    let skills = read_workspace_skills(record, &agent_skill_configs(store));
    let link = skills
        .iter()
        .find(|skill| skill.agent == "agent_a")
        .unwrap();
    let vendored = vendored_variant(record, &skills, link).unwrap();
    assert_ne!(vendored.path, link.path);
    update_vendored_from_center(vendored, &store.get_all_skills().unwrap())
}
