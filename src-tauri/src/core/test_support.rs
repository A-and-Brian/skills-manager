//! Fixtures shared by the core and command tests.

use std::fs;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};

use crate::core::{
    central_repo, git_credentials,
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

pub(crate) fn write_skill(dir: &Path, name: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: d\n---\nbody\n"),
    )
    .unwrap();
}

pub(crate) struct TestEnv {
    _lock: std::sync::MutexGuard<'static, ()>,
    pub(crate) _tmp: tempfile::TempDir,
    pub(crate) store: SkillStore,
    pub(crate) skills_dir: std::path::PathBuf,
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        central_repo::set_test_base_dir_override(None);
    }
}

/// Isolated base dir (askpass script, skills repo, DB) + mock keyring.
pub(crate) fn test_env() -> TestEnv {
    git_credentials::use_mock_keyring();
    let lock = central_repo::test_base_dir_lock();
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("base");
    central_repo::set_test_base_dir_override(Some(base.clone()));
    let skills_dir = central_repo::skills_dir();
    std::fs::create_dir_all(&skills_dir).unwrap();
    let store = SkillStore::new(&base.join("test.db")).unwrap();
    TestEnv {
        _lock: lock,
        _tmp: tmp,
        store,
        skills_dir,
    }
}

pub(crate) fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

pub(crate) fn sample_managed_skill(
    central_path: String,
    content_hash: Option<String>,
    updated_at: i64,
) -> SkillRecord {
    SkillRecord {
        id: "skill-1".to_string(),
        name: "Example Skill".to_string(),
        description: None,
        source_type: "local".to_string(),
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
        updated_at,
        status: "ok".to_string(),
        update_status: "local_only".to_string(),
        last_checked_at: None,
        last_check_error: None,
    }
}
