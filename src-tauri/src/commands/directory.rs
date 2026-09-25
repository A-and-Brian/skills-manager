use std::path::PathBuf;

use serde::Serialize;

use crate::core::error::AppError;

#[derive(Debug, Serialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Serialize)]
pub struct DirectoryListing {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<DirectoryEntry>,
}

#[tauri::command]
pub async fn list_directory(path: Option<String>) -> Result<DirectoryListing, AppError> {
    tauri::async_runtime::spawn_blocking(move || list_directory_core(path)).await?
}

/// One folder of the host's file system, for choosing paths on a machine the
/// native dialog can't reach. `None` lists the home folder. Folders come
/// first, then files, each in alphabetical order; hidden entries are kept
/// because agent folders (`.claude`, `.codex`, ...) are what people pick.
pub fn list_directory_core(path: Option<String>) -> Result<DirectoryListing, AppError> {
    let dir = match path {
        Some(path) => PathBuf::from(path),
        None => dirs::home_dir().ok_or_else(|| AppError::not_found("No home folder"))?,
    };
    if !dir.is_absolute() {
        return Err(AppError::invalid_input(format!(
            "Not an absolute path: {}",
            dir.display()
        )));
    }
    if !dir.is_dir() {
        return Err(AppError::invalid_input(format!(
            "Not a folder: {}",
            dir.display()
        )));
    }

    let mut entries: Vec<DirectoryEntry> = std::fs::read_dir(&dir)?
        .flatten()
        .map(|entry| {
            let path = entry.path();
            DirectoryEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: path.is_dir(),
                path: path.to_string_lossy().into_owned(),
            }
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });

    Ok(DirectoryListing {
        path: dir.to_string_lossy().into_owned(),
        parent: dir.parent().map(|p| p.to_string_lossy().into_owned()),
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::error::ErrorKind;
    use std::fs;

    #[test]
    fn lists_folders_before_files_alphabetically_with_hidden_entries() {
        let tmp = tempfile::tempdir().unwrap();
        for dir in ["beta", ".claude", "Alpha"] {
            fs::create_dir(tmp.path().join(dir)).unwrap();
        }
        for file in ["b.md", ".env", "A.txt"] {
            fs::write(tmp.path().join(file), "").unwrap();
        }

        let listing = list_directory_core(Some(tmp.path().to_string_lossy().into_owned())).unwrap();

        let names: Vec<(&str, bool)> = listing
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.is_dir))
            .collect();
        assert_eq!(
            names,
            vec![
                (".claude", true),
                ("Alpha", true),
                ("beta", true),
                (".env", false),
                ("A.txt", false),
                ("b.md", false),
            ]
        );
        assert_eq!(
            listing.entries[1].path,
            tmp.path().join("Alpha").to_string_lossy()
        );
        assert_eq!(
            listing.parent.as_deref(),
            tmp.path().parent().map(|p| p.to_string_lossy()).as_deref()
        );
    }

    #[test]
    fn the_root_has_no_parent() {
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        let listing = list_directory_core(Some(root.to_string())).unwrap();
        assert_eq!(listing.parent, None);
    }

    #[test]
    fn a_file_or_relative_path_is_invalid_input() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("notes.md");
        fs::write(&file, "").unwrap();

        for path in [
            file.to_string_lossy().into_owned(),
            "relative/dir".to_string(),
        ] {
            let err = list_directory_core(Some(path.clone())).unwrap_err();
            assert_eq!(err.kind, ErrorKind::InvalidInput, "{path}");
        }
    }

    #[test]
    fn no_path_lists_the_home_folder() {
        let listing = list_directory_core(None).unwrap();
        assert_eq!(PathBuf::from(listing.path), dirs::home_dir().unwrap());
    }
}
