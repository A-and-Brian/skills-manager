//! Removing managed skills from the library, shared by the GUI and the CLI.

use serde::Serialize;
use std::path::PathBuf;

use crate::core::{
    audit_log::AuditDraft, error::AppError, skill_store::SkillStore, sync_engine, sync_metadata,
};

#[derive(Debug, Serialize)]
pub struct BatchDeleteSkillsResult {
    pub deleted: usize,
    pub failed: Vec<String>,
}

pub fn delete_managed_skills_by_ids(
    store: &SkillStore,
    skill_ids: &[String],
) -> Result<BatchDeleteSkillsResult, AppError> {
    sync_metadata::with_repo_lock("delete skills", || {
        let mut deleted = 0;
        let mut failed = Vec::new();

        for skill_id in skill_ids {
            let Some(skill) = store.get_skill_by_id(skill_id)? else {
                store.log_audit(
                    AuditDraft::new("remove")
                        .skill(skill_id.clone(), "")
                        .fail("not found"),
                );
                failed.push(skill_id.clone());
                continue;
            };

            let targets = store.get_targets_for_skill(skill_id)?;
            for target in &targets {
                let target_path = PathBuf::from(&target.target_path);
                sync_engine::remove_target(&target_path).ok();
            }

            let central = PathBuf::from(&skill.central_path);
            if central.exists() {
                std::fs::remove_dir_all(&central).ok();
            }

            store.delete_skill(skill_id)?;
            store.log_audit(
                AuditDraft::new("remove")
                    .skill(skill_id.clone(), skill.name.clone())
                    .ok(),
            );
            deleted += 1;
        }

        if deleted > 0 {
            sync_metadata::write_all_from_db_unlocked(store)?;
        }

        Ok(BatchDeleteSkillsResult { deleted, failed })
    })
    .map_err(AppError::db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::skill_store::SkillTargetRecord;
    use crate::core::test_support::{sample_skill, test_repo, write_skill_dir};
    use std::fs;

    #[test]
    fn batch_delete_removes_skills_targets_and_stale_metadata_once() {
        let repo = test_repo();
        let skill_one_dir = write_skill_dir("skill-one");
        let skill_two_dir = write_skill_dir("skill-two");
        repo.store
            .insert_skill(&sample_skill("skill-1", "skill-one", &skill_one_dir))
            .unwrap();
        repo.store
            .insert_skill(&sample_skill("skill-2", "skill-two", &skill_two_dir))
            .unwrap();

        let target_dir = repo._tmp.path().join("target-skill-one");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("SKILL.md"), "# target").unwrap();
        repo.store
            .insert_target(&SkillTargetRecord {
                id: "target-1".to_string(),
                skill_id: "skill-1".to_string(),
                tool: "cursor".to_string(),
                target_path: target_dir.to_string_lossy().to_string(),
                mode: "symlink".to_string(),
                status: "ok".to_string(),
                synced_at: Some(1),
                last_error: None,
                source_hash: None,
            })
            .unwrap();

        sync_metadata::write_all_from_db_unlocked(&repo.store).unwrap();
        assert!(sync_metadata::metadata_dir()
            .join("skills/skill-1.json")
            .exists());
        assert!(sync_metadata::metadata_dir()
            .join("skills/skill-2.json")
            .exists());

        let result = delete_managed_skills_by_ids(
            &repo.store,
            &["skill-1".to_string(), "missing-skill".to_string()],
        )
        .unwrap();

        assert_eq!(result.deleted, 1);
        assert_eq!(result.failed, vec!["missing-skill".to_string()]);
        assert!(repo.store.get_skill_by_id("skill-1").unwrap().is_none());
        assert!(repo.store.get_skill_by_id("skill-2").unwrap().is_some());
        assert!(!skill_one_dir.exists());
        assert!(skill_two_dir.exists());
        assert!(!target_dir.exists());
        assert!(!sync_metadata::metadata_dir()
            .join("skills/skill-1.json")
            .exists());
        assert!(sync_metadata::metadata_dir()
            .join("skills/skill-2.json")
            .exists());
    }
}
