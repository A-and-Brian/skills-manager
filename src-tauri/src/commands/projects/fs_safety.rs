//! Path checks and filesystem moves for skills inside a workspace: keeping
//! paths under their root, and enabling or disabling a skill on disk.

use std::path::{Component, Path};

use crate::core::{error::AppError, sync_engine};

pub(crate) fn ensure_safe_skill_relative_path(skill_relative_path: &str) -> Result<(), AppError> {
    if skill_relative_path.trim().is_empty() {
        return Err(AppError::invalid_input("Invalid skill directory path"));
    }
    let mut saw_component = false;
    for component in Path::new(skill_relative_path).components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(AppError::invalid_input("Invalid skill directory path"));
        }
        saw_component = true;
    }
    if !saw_component {
        return Err(AppError::invalid_input("Invalid skill directory path"));
    }
    Ok(())
}

pub(crate) fn ensure_dir_within_root(path: &Path, root: &Path) -> Result<(), AppError> {
    // First check that the lexical path (before symlink resolution) is under root.
    // This ensures the link itself lives where expected.
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let abs_root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()?.join(root)
    };
    if !abs_path.starts_with(&abs_root) {
        return Err(AppError::invalid_input("Invalid skill directory path"));
    }
    Ok(())
}

pub(super) fn remove_workspace_skill_target(path: &Path) -> Result<(), AppError> {
    sync_engine::remove_target(path).map_err(AppError::io)
}

// Walks upward from `start`, removing each empty directory until reaching
// (and including) `root`. Stops at the first non-empty directory or any
// other error. `fs::remove_dir` only succeeds on empty directories, so this
// will never delete a directory that still holds skills.
fn cleanup_empty_dirs_up_to(start: &Path, root: &Path) {
    let Ok(root_canonical) = std::fs::canonicalize(root) else {
        return;
    };
    let mut current = start.to_path_buf();
    loop {
        let Ok(current_canonical) = std::fs::canonicalize(&current) else {
            return;
        };
        if !current_canonical.starts_with(&root_canonical) {
            return;
        }
        if std::fs::remove_dir(&current).is_err() {
            return;
        }
        if current_canonical == root_canonical {
            return;
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => return,
        }
    }
}

fn remove_symlink_entry(path: &Path) -> Result<(), AppError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(AppError::io(err)),
    };
    if !metadata.file_type().is_symlink() {
        return Err(AppError::invalid_input(
            "Duplicate skill entry is not a symlink — resolve manually",
        ));
    }
    sync_engine::remove_target(path).map_err(AppError::io)
}

pub(super) fn set_project_skill_enabled_state(
    skills_dir: &Path,
    disabled_dir: &Path,
    skill_relative_path: &str,
    enabled: bool,
) -> Result<(), AppError> {
    ensure_safe_skill_relative_path(skill_relative_path)?;

    let enabled_path = skills_dir.join(skill_relative_path);
    let disabled_path = disabled_dir.join(skill_relative_path);

    if enabled {
        if enabled_path.is_dir() {
            ensure_dir_within_root(&enabled_path, skills_dir)?;
            if disabled_path.exists() {
                ensure_dir_within_root(&disabled_path, disabled_dir)?;
                remove_symlink_entry(&disabled_path)?;
                if let Some(parent) = disabled_path.parent() {
                    cleanup_empty_dirs_up_to(parent, disabled_dir);
                }
            }
            return Ok(());
        }

        if !disabled_path.is_dir() {
            return Err(AppError::not_found(
                "Skill directory not found in skills-disabled",
            ));
        }
        ensure_dir_within_root(&disabled_path, disabled_dir)?;
        if let Some(parent) = enabled_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if enabled_path.exists() {
            return Err(AppError::invalid_input(
                "Skill already exists in skills directory",
            ));
        }
        std::fs::rename(&disabled_path, &enabled_path)?;
        if let Some(parent) = disabled_path.parent() {
            cleanup_empty_dirs_up_to(parent, disabled_dir);
        }
        return Ok(());
    }

    if disabled_path.is_dir() {
        ensure_dir_within_root(&disabled_path, disabled_dir)?;
        if enabled_path.exists() {
            ensure_dir_within_root(&enabled_path, skills_dir)?;
            remove_symlink_entry(&enabled_path)?;
        }
        return Ok(());
    }

    if !enabled_path.is_dir() {
        return Err(AppError::not_found("Skill directory not found"));
    }
    ensure_dir_within_root(&enabled_path, skills_dir)?;
    if let Some(parent) = disabled_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if disabled_path.exists() {
        return Err(AppError::invalid_input(
            "Skill already exists in skills-disabled directory",
        ));
    }
    std::fs::rename(&enabled_path, &disabled_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{remove_workspace_skill_target, set_project_skill_enabled_state};
    use crate::core::error::ErrorKind;
    use std::fs;
    use tempfile::tempdir;

    #[cfg(unix)]
    #[test]
    fn remove_workspace_skill_target_removes_symlink_without_touching_target() {
        let tmp = tempdir().unwrap();
        let real = tmp.path().join("real-skill");
        let link = tmp.path().join("linked-skill");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("SKILL.md"), "# hello").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        remove_workspace_skill_target(&link).unwrap();

        assert!(!link.exists());
        assert!(real.exists());
        assert!(real.join("SKILL.md").exists());
    }

    #[cfg(windows)]
    #[test]
    fn remove_workspace_skill_target_removes_directory_symlink_without_touching_target() {
        let tmp = tempdir().unwrap();
        let real = tmp.path().join("real-skill");
        let link = tmp.path().join("linked-skill");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("SKILL.md"), "# hello").unwrap();
        std::os::windows::fs::symlink_dir(&real, &link).unwrap();

        remove_workspace_skill_target(&link).unwrap();

        assert!(!link.exists());
        assert!(real.exists());
        assert!(real.join("SKILL.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn set_project_skill_enabled_state_disabling_cleans_duplicate_symlink_without_touching_target()
    {
        use std::os::unix::fs::symlink;

        let tmp = tempdir().unwrap();
        let central_skill = tmp.path().join("central").join("understand-diff");
        let skills_root = tmp.path().join("skills");
        let disabled_root = tmp.path().join("skills-disabled");
        let relative_path = "understand-diff";

        fs::create_dir_all(&central_skill).unwrap();
        fs::write(
            central_skill.join("SKILL.md"),
            "---\nname: understand-diff\n---\n",
        )
        .unwrap();
        fs::create_dir_all(&skills_root).unwrap();
        fs::create_dir_all(&disabled_root).unwrap();

        symlink(&central_skill, skills_root.join(relative_path)).unwrap();
        symlink(&central_skill, disabled_root.join(relative_path)).unwrap();

        set_project_skill_enabled_state(&skills_root, &disabled_root, relative_path, false)
            .unwrap();

        assert!(!skills_root.join(relative_path).exists());
        assert!(disabled_root.join(relative_path).exists());
        assert!(central_skill.exists());
        assert!(central_skill.join("SKILL.md").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn set_project_skill_enabled_state_enabling_cleans_duplicate_symlink_without_touching_target() {
        use std::os::unix::fs::symlink;

        let tmp = tempdir().unwrap();
        let central_skill = tmp.path().join("central").join("understand-diff");
        let skills_root = tmp.path().join("skills");
        let disabled_root = tmp.path().join("skills-disabled");
        let relative_path = "understand-diff";

        fs::create_dir_all(&central_skill).unwrap();
        fs::write(
            central_skill.join("SKILL.md"),
            "---\nname: understand-diff\n---\n",
        )
        .unwrap();
        fs::create_dir_all(&skills_root).unwrap();
        fs::create_dir_all(&disabled_root).unwrap();

        symlink(&central_skill, skills_root.join(relative_path)).unwrap();
        symlink(&central_skill, disabled_root.join(relative_path)).unwrap();

        set_project_skill_enabled_state(&skills_root, &disabled_root, relative_path, true).unwrap();

        assert!(skills_root.join(relative_path).exists());
        assert!(!disabled_root.join(relative_path).exists());
        assert!(central_skill.exists());
        assert!(central_skill.join("SKILL.md").is_file());
    }

    #[test]
    fn set_project_skill_enabled_state_enabling_removes_emptied_disabled_dir() {
        let tmp = tempdir().unwrap();
        let skills_root = tmp.path().join("skills");
        let disabled_root = tmp.path().join("skills-disabled");
        let relative_path = "my-skill";

        let real_disabled = disabled_root.join(relative_path);
        fs::create_dir_all(&skills_root).unwrap();
        fs::create_dir_all(&real_disabled).unwrap();
        fs::write(real_disabled.join("SKILL.md"), "---\nname: my-skill\n---\n").unwrap();

        set_project_skill_enabled_state(&skills_root, &disabled_root, relative_path, true).unwrap();

        assert!(skills_root.join(relative_path).join("SKILL.md").is_file());
        assert!(!disabled_root.exists());
    }

    #[test]
    fn set_project_skill_enabled_state_enabling_keeps_disabled_dir_when_other_skills_remain() {
        let tmp = tempdir().unwrap();
        let skills_root = tmp.path().join("skills");
        let disabled_root = tmp.path().join("skills-disabled");
        let relative_path = "skill-a";

        let real_disabled_a = disabled_root.join(relative_path);
        let real_disabled_b = disabled_root.join("skill-b");
        fs::create_dir_all(&skills_root).unwrap();
        fs::create_dir_all(&real_disabled_a).unwrap();
        fs::create_dir_all(&real_disabled_b).unwrap();
        fs::write(
            real_disabled_a.join("SKILL.md"),
            "---\nname: skill-a\n---\n",
        )
        .unwrap();
        fs::write(
            real_disabled_b.join("SKILL.md"),
            "---\nname: skill-b\n---\n",
        )
        .unwrap();

        set_project_skill_enabled_state(&skills_root, &disabled_root, relative_path, true).unwrap();

        assert!(skills_root.join(relative_path).join("SKILL.md").is_file());
        assert!(disabled_root.is_dir());
        assert!(real_disabled_b.join("SKILL.md").is_file());
    }

    #[test]
    fn set_project_skill_enabled_state_enabling_removes_empty_nested_disabled_dirs() {
        let tmp = tempdir().unwrap();
        let skills_root = tmp.path().join("skills");
        let disabled_root = tmp.path().join("skills-disabled");
        let relative_path = "category/sub/skill-a";

        let real_disabled = disabled_root.join(relative_path);
        fs::create_dir_all(&skills_root).unwrap();
        fs::create_dir_all(&real_disabled).unwrap();
        fs::write(real_disabled.join("SKILL.md"), "---\nname: skill-a\n---\n").unwrap();

        set_project_skill_enabled_state(&skills_root, &disabled_root, relative_path, true).unwrap();

        assert!(skills_root.join(relative_path).join("SKILL.md").is_file());
        assert!(!disabled_root.exists());
    }

    #[test]
    fn set_project_skill_enabled_state_rejects_real_dir_duplicate_on_enable() {
        let tmp = tempdir().unwrap();
        let skills_root = tmp.path().join("skills");
        let disabled_root = tmp.path().join("skills-disabled");
        let relative_path = "my-skill";

        let real_enabled = skills_root.join(relative_path);
        let real_disabled = disabled_root.join(relative_path);
        fs::create_dir_all(&real_enabled).unwrap();
        fs::write(real_enabled.join("SKILL.md"), "---\nname: my-skill\n---\n").unwrap();
        fs::create_dir_all(&real_disabled).unwrap();
        fs::write(real_disabled.join("SKILL.md"), "---\nname: my-skill\n---\n").unwrap();

        let err =
            set_project_skill_enabled_state(&skills_root, &disabled_root, relative_path, true)
                .unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidInput);
        // Both real dirs must still exist
        assert!(real_enabled.join("SKILL.md").exists());
        assert!(real_disabled.join("SKILL.md").exists());
    }

    #[test]
    fn set_project_skill_enabled_state_rejects_real_dir_duplicate_on_disable() {
        let tmp = tempdir().unwrap();
        let skills_root = tmp.path().join("skills");
        let disabled_root = tmp.path().join("skills-disabled");
        let relative_path = "my-skill";

        let real_enabled = skills_root.join(relative_path);
        let real_disabled = disabled_root.join(relative_path);
        fs::create_dir_all(&real_enabled).unwrap();
        fs::write(real_enabled.join("SKILL.md"), "---\nname: my-skill\n---\n").unwrap();
        fs::create_dir_all(&real_disabled).unwrap();
        fs::write(real_disabled.join("SKILL.md"), "---\nname: my-skill\n---\n").unwrap();

        let err =
            set_project_skill_enabled_state(&skills_root, &disabled_root, relative_path, false)
                .unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidInput);
        assert!(real_enabled.join("SKILL.md").exists());
        assert!(real_disabled.join("SKILL.md").exists());
    }
}
