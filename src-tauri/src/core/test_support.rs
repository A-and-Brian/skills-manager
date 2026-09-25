//! Fixtures shared by the core and command tests.

use std::fs;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};

use crate::core::{
    central_repo,
    skill_store::{SkillRecord, SkillStore},
};

pub(crate) struct TestRepo {
    _lock: std::sync::MutexGuard<'static, ()>,
    pub(crate) _tmp: TempDir,
    pub(crate) store: SkillStore,
}

impl Drop for TestRepo {
    fn drop(&mut self) {
        central_repo::set_test_base_dir_override(None);
    }
}

pub(crate) fn test_repo() -> TestRepo {
    let lock = central_repo::test_base_dir_lock();
    let tmp = tempdir().unwrap();
    let base = tmp.path().join("repo");
    central_repo::set_test_base_dir_override(Some(base.clone()));
    fs::create_dir_all(central_repo::skills_dir()).unwrap();
    let store = SkillStore::new(&base.join("test.db")).unwrap();
    TestRepo {
        _lock: lock,
        _tmp: tmp,
        store,
    }
}

pub(crate) fn write_skill_dir(name: &str) -> PathBuf {
    let dir = central_repo::skills_dir().join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), format!("---\nname: {name}\n---\n")).unwrap();
    dir
}

pub(crate) fn sample_skill(id: &str, name: &str, central_path: &Path) -> SkillRecord {
    SkillRecord {
        id: id.to_string(),
        name: name.to_string(),
        description: None,
        source_type: "import".to_string(),
        source_ref: Some(central_path.to_string_lossy().to_string()),
        source_ref_resolved: None,
        source_subpath: None,
        source_branch: None,
        source_revision: None,
        remote_revision: None,
        central_path: central_path.to_string_lossy().to_string(),
        content_hash: None,
        enabled: true,
        created_at: 1,
        updated_at: 1,
        status: "ok".to_string(),
        update_status: "local_only".to_string(),
        last_checked_at: None,
        last_check_error: None,
    }
}
