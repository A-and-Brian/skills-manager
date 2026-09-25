use serde::Serialize;
use std::path::{Path, PathBuf};

use super::project_deploy::is_vendored_dir;
use super::{content_hash, skill_metadata};

/// Where a copy-mode project vendors its skills, the skills.sh convention.
/// Disabled skills sit in the `-disabled` twin, like every agent folder.
pub const VENDORED_SKILLS_DIR: &str = ".agents/skills";

/// Lightweight config describing where an agent keeps project-level skills.
#[derive(Debug, Clone)]
pub struct AgentSkillConfig {
    pub key: String,
    pub display_name: String,
    /// Relative path from project root to the skills directory (e.g. ".claude/skills").
    pub relative_skills_dir: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectSkillInfo {
    pub name: String,
    pub dir_name: String,
    #[serde(default)]
    pub relative_path: String,
    pub description: Option<String>,
    /// SKILL.md frontmatter author, when the skill names one.
    #[serde(default)]
    pub author: Option<String>,
    pub path: String,
    pub files: Vec<String>,
    pub enabled: bool,
    /// Agent key that owns this skill (e.g. "claude_code", "cursor").
    #[serde(default)]
    pub agent: String,
    /// Human-readable agent name (e.g. "Claude Code", "Cursor").
    #[serde(default)]
    pub agent_display_name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub in_center: bool,
    #[serde(default)]
    pub sync_status: String,
    #[serde(default)]
    pub center_skill_id: Option<String>,
    /// The user chose this skill's agents by hand, so bulk agent changes skip it.
    #[serde(default)]
    pub agents_overridden: bool,
    /// Relative path of the vendored skill this copy links to, when it is a
    /// link into `.agents/skills` (or its disabled twin).
    #[serde(default)]
    pub alias_of: Option<String>,
    /// This copy is a vendored copy: real files in `.agents/skills` (or its
    /// disabled twin), which links from other agents may read.
    #[serde(default)]
    pub vendored: bool,
    #[serde(skip_serializing)]
    pub last_modified_at: Option<i64>,
    #[serde(skip_serializing)]
    pub content_hash: Option<String>,
}

/// Read skills from all configured agents' project-level skill directories.
pub fn read_project_skills(
    project_path: &Path,
    agent_configs: &[AgentSkillConfig],
) -> Vec<ProjectSkillInfo> {
    let mut skills = Vec::new();

    for config in agent_configs {
        let first = skills.len();
        let skills_dir = project_path.join(&config.relative_skills_dir);
        let disabled_dir = project_path.join(format!("{}-disabled", config.relative_skills_dir));

        read_skills_from_dir(
            &skills_dir,
            true,
            &config.key,
            &config.display_name,
            &mut skills,
            true,
        );
        read_skills_from_dir(
            &disabled_dir,
            false,
            &config.key,
            &config.display_name,
            &mut skills,
            true,
        );
        if is_vendored_dir(&config.relative_skills_dir) {
            for skill in &mut skills[first..] {
                skill.vendored = !is_link(Path::new(&skill.path));
            }
        }
    }

    let vendored_roots: Vec<PathBuf> = [
        VENDORED_SKILLS_DIR.to_string(),
        format!("{VENDORED_SKILLS_DIR}-disabled"),
    ]
    .iter()
    .filter_map(|dir| std::fs::canonicalize(project_path.join(dir)).ok())
    .collect();
    for skill in &mut skills {
        skill.alias_of = vendored_alias(Path::new(&skill.path), &vendored_roots);
    }

    skills.sort_by_key(|a| a.name.to_lowercase());
    skills
}

fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

/// The vendored skill a link resolves to, as a path relative to its root.
fn vendored_alias(path: &Path, vendored_roots: &[PathBuf]) -> Option<String> {
    if !is_link(path) {
        return None;
    }
    let resolved = std::fs::canonicalize(path).ok()?;
    vendored_roots.iter().find_map(|root| {
        let relative = resolved.strip_prefix(root).ok()?;
        (!relative.as_os_str().is_empty()).then(|| relative.to_string_lossy().replace('\\', "/"))
    })
}

pub fn read_linked_workspace_skills(
    skills_root: &Path,
    disabled_root: Option<&Path>,
    agent_key: &str,
    agent_display_name: &str,
    recursive: bool,
) -> Vec<ProjectSkillInfo> {
    let mut skills = Vec::new();
    read_skills_from_dir(
        skills_root,
        true,
        agent_key,
        agent_display_name,
        &mut skills,
        recursive,
    );
    if let Some(disabled_root) = disabled_root {
        read_skills_from_dir(
            disabled_root,
            false,
            agent_key,
            agent_display_name,
            &mut skills,
            recursive,
        );
    }
    skills.sort_by_key(|a| a.name.to_lowercase());
    skills
}

fn should_skip_dir(root: &Path, dir: &Path) -> bool {
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.starts_with('.') {
        return true;
    }

    // Ignore embedded plugin/cache bundle layouts such as:
    // <bundle>/<version>/skills/<skill>/SKILL.md
    // The workspace root itself may be named "skills", so only skip nested
    // container directories that introduce another "skills" subtree.
    dir != root && dir.join("skills").is_dir()
}

fn read_skills_from_dir(
    dir: &Path,
    enabled: bool,
    agent: &str,
    agent_display_name: &str,
    skills: &mut Vec<ProjectSkillInfo>,
    recursive: bool,
) {
    if !dir.is_dir() {
        return;
    }
    let mut visited = std::collections::HashSet::new();
    if let Ok(canon) = std::fs::canonicalize(dir) {
        visited.insert(canon);
    }
    read_skills_from_dir_recursive(
        dir,
        dir,
        enabled,
        agent,
        agent_display_name,
        skills,
        &mut visited,
        recursive,
    );
}

#[allow(clippy::too_many_arguments)]
fn read_skills_from_dir_recursive(
    root: &Path,
    current: &Path,
    enabled: bool,
    agent: &str,
    agent_display_name: &str,
    skills: &mut Vec<ProjectSkillInfo>,
    visited: &mut std::collections::HashSet<PathBuf>,
    recursive: bool,
) {
    let Ok(entries) = std::fs::read_dir(current) else {
        return;
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        if skill_metadata::is_valid_skill_dir(&path) {
            let dir_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let relative_path = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            let meta = skill_metadata::parse_skill_md(&path);
            let name = meta
                .name
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| dir_name.clone());

            let files = list_files(&path);

            // One recursive walk feeds both the content hash and the
            // last-modified time (#248). Previously this ran three separate
            // walks per skill: `hash_directory`, plus `latest_modified_millis`
            // which called `canonicalize()` on every node, so the workspace
            // scan cost scaled at ~3× the necessary syscalls per skill.
            let content_entries = content_hash::list_content_files(&path);
            let content_hash = Some(content_hash::hash_entries(&content_entries));
            let last_modified_at = content_hash::latest_modified_ms(&content_entries);

            skills.push(ProjectSkillInfo {
                name,
                dir_name: dir_name.clone(),
                relative_path,
                description: meta.description,
                author: meta.author,
                path: path.to_string_lossy().to_string(),
                files,
                enabled,
                agent: agent.to_string(),
                agent_display_name: agent_display_name.to_string(),
                tags: Vec::new(),
                in_center: false,
                sync_status: "project_only".to_string(),
                center_skill_id: None,
                agents_overridden: false,
                alias_of: None,
                vendored: false,
                last_modified_at,
                content_hash,
            });
            continue;
        }

        // Only check visited set before recursing into namespace dirs
        // to prevent symlink cycles. Skill dirs (above) are leaf nodes and
        // are allowed to alias the same canonical target.

        if !recursive || should_skip_dir(root, &path) {
            continue;
        }

        let canon = match std::fs::canonicalize(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if !visited.insert(canon) {
            continue;
        }
        read_skills_from_dir_recursive(
            root,
            &path,
            enabled,
            agent,
            agent_display_name,
            skills,
            visited,
            recursive,
        );
    }
}

/// Scan a root directory for projects containing any agent's skills directory.
pub fn scan_projects_in_dir(
    root: &Path,
    max_depth: usize,
    agent_configs: &[AgentSkillConfig],
) -> Vec<String> {
    let mut results = Vec::new();
    scan_recursive(root, 0, max_depth, agent_configs, &mut results);
    results.sort();
    results
}

fn has_any_agent_skills(dir: &Path, agent_configs: &[AgentSkillConfig]) -> bool {
    agent_configs
        .iter()
        .any(|config| dir.join(&config.relative_skills_dir).is_dir())
}

fn scan_recursive(
    dir: &Path,
    depth: usize,
    max_depth: usize,
    agent_configs: &[AgentSkillConfig],
    results: &mut Vec<String>,
) {
    if depth > max_depth {
        return;
    }

    if has_any_agent_skills(dir, agent_configs) {
        results.push(dir.to_string_lossy().to_string());
        return; // don't recurse into subdirectories of a matched project
    }

    if depth == max_depth {
        return;
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            // Skip hidden directories and common non-project dirs
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.')
                || name == "node_modules"
                || name == "target"
                || name == "__pycache__"
            {
                continue;
            }
            scan_recursive(&path, depth + 1, max_depth, agent_configs, results);
        }
    }
}

fn list_files(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name() {
                    files.push(name.to_string_lossy().to_string());
                }
            }
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::{read_linked_workspace_skills, read_project_skills, AgentSkillConfig};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn reads_nested_project_skills_recursively() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join(".hermes").join("skills");
        let nested_skill = root.join("research").join("web-search");
        fs::create_dir_all(&nested_skill).unwrap();
        fs::write(
            nested_skill.join("SKILL.md"),
            "---\nname: Web Search\ndescription: Nested skill\n---\n",
        )
        .unwrap();

        let configs = vec![AgentSkillConfig {
            key: "hermes".to_string(),
            display_name: "Hermes".to_string(),
            relative_skills_dir: ".hermes/skills".to_string(),
        }];

        let skills = read_project_skills(tmp.path(), &configs);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].dir_name, "web-search");
        assert_eq!(skills[0].relative_path, "research/web-search");
        assert_eq!(skills[0].name, "Web Search");
    }

    #[test]
    fn prefers_skill_dir_over_namespace_parent_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join(".hermes").join("skills");
        let namespace = root.join("research");
        let nested_skill = namespace.join("web-search");
        fs::create_dir_all(&nested_skill).unwrap();
        fs::write(namespace.join("notes.txt"), "namespace").unwrap();
        fs::write(nested_skill.join("SKILL.md"), "# Nested").unwrap();

        let configs = vec![AgentSkillConfig {
            key: "hermes".to_string(),
            display_name: "Hermes".to_string(),
            relative_skills_dir: ".hermes/skills".to_string(),
        }];

        let skills = read_project_skills(tmp.path(), &configs);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].relative_path, "research/web-search");
    }

    #[test]
    fn linked_workspace_skips_hidden_dirs_and_embedded_bundle_skills() {
        let tmp = tempdir().unwrap();
        let skills_root = tmp.path().join("skills");
        let disabled_root = tmp.path().join("skills-disabled");

        let top_level_skill = skills_root.join("understand");
        fs::create_dir_all(&top_level_skill).unwrap();
        fs::write(
            top_level_skill.join("SKILL.md"),
            "---\nname: understand\n---\n",
        )
        .unwrap();

        let hidden_skill = skills_root
            .join(".claude")
            .join("skills")
            .join("hidden-skill");
        fs::create_dir_all(&hidden_skill).unwrap();
        fs::write(
            hidden_skill.join("SKILL.md"),
            "---\nname: hidden-skill\n---\n",
        )
        .unwrap();

        let embedded_enabled = skills_root
            .join("understand-anything")
            .join("understand-anything")
            .join("311f2ad1aca5")
            .join("skills")
            .join("understand");
        fs::create_dir_all(&embedded_enabled).unwrap();
        fs::write(
            embedded_enabled.join("SKILL.md"),
            "---\nname: understand\n---\n",
        )
        .unwrap();

        let disabled_skill = disabled_root.join("understand-diff");
        fs::create_dir_all(&disabled_skill).unwrap();
        fs::write(
            disabled_skill.join("SKILL.md"),
            "---\nname: understand-diff\n---\n",
        )
        .unwrap();

        let embedded_disabled = disabled_root
            .join("claude-plugins-official")
            .join("superpowers")
            .join("5.0.7")
            .join("skills")
            .join("brainstorming");
        fs::create_dir_all(&embedded_disabled).unwrap();
        fs::write(
            embedded_disabled.join("SKILL.md"),
            "---\nname: brainstorming\n---\n",
        )
        .unwrap();

        let skills = read_linked_workspace_skills(
            &skills_root,
            Some(&disabled_root),
            "linked",
            "Linked",
            true,
        );

        let names: Vec<&str> = skills.iter().map(|skill| skill.name.as_str()).collect();
        assert_eq!(names, vec!["understand", "understand-diff"]);
        assert_eq!(
            skills
                .iter()
                .filter(|skill| skill.name == "understand")
                .count(),
            1
        );
        assert!(skills
            .iter()
            .any(|skill| skill.name == "understand" && skill.enabled));
        assert!(skills
            .iter()
            .any(|skill| skill.name == "understand-diff" && !skill.enabled));
    }

    #[test]
    fn linked_workspace_flat_scan_ignores_nested_skills() {
        let tmp = tempdir().unwrap();
        let skills_root = tmp.path().join("skills");

        let top_level_skill = skills_root.join("codex-tool");
        fs::create_dir_all(&top_level_skill).unwrap();
        fs::write(
            top_level_skill.join("SKILL.md"),
            "---\nname: codex-tool\n---\n",
        )
        .unwrap();

        let nested_skill = skills_root.join("vendor").join("nested-tool");
        fs::create_dir_all(&nested_skill).unwrap();
        fs::write(
            nested_skill.join("SKILL.md"),
            "---\nname: nested-tool\n---\n",
        )
        .unwrap();

        let skills = read_linked_workspace_skills(&skills_root, None, "codex", "Codex", false);

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "codex-tool");
    }

    /// A link into `.agents/skills` names the vendored skill it reads; the
    /// vendored copy itself and links elsewhere name none. Only real files in
    /// `.agents/skills` are a vendored copy, not a link kept there.
    #[cfg(unix)]
    #[test]
    fn links_into_the_vendored_folder_report_what_they_alias() {
        use std::os::unix::fs::symlink;
        use std::path::Path;

        let tmp = tempdir().unwrap();
        let project = tmp.path();
        let vendored = project.join(".agents/skills/team/x");
        fs::create_dir_all(&vendored).unwrap();
        fs::write(vendored.join("SKILL.md"), "---\nname: x\n---\n").unwrap();
        let library = project.join("library/y");
        fs::create_dir_all(&library).unwrap();
        fs::write(library.join("SKILL.md"), "---\nname: y\n---\n").unwrap();
        fs::create_dir_all(project.join(".claude/skills")).unwrap();
        symlink(
            Path::new("../../.agents/skills/team/x"),
            project.join(".claude/skills/x"),
        )
        .unwrap();
        symlink(&library, project.join(".claude/skills/y")).unwrap();
        symlink(&library, project.join(".agents/skills/y")).unwrap();

        let configs = [
            ("cline", ".agents/skills"),
            ("claude_code", ".claude/skills"),
        ]
        .into_iter()
        .map(|(key, dir)| AgentSkillConfig {
            key: key.to_string(),
            display_name: key.to_string(),
            relative_skills_dir: dir.to_string(),
        })
        .collect::<Vec<_>>();
        let skills = read_project_skills(project, &configs);

        let alias = |agent: &str, name: &str| {
            skills
                .iter()
                .find(|skill| skill.agent == agent && skill.name == name)
                .unwrap()
                .alias_of
                .clone()
        };
        assert_eq!(alias("claude_code", "x"), Some("team/x".to_string()));
        assert_eq!(alias("cline", "x"), None);
        assert_eq!(alias("claude_code", "y"), None);

        let vendored: Vec<(&str, &str)> = skills
            .iter()
            .filter(|skill| skill.vendored)
            .map(|skill| (skill.agent.as_str(), skill.name.as_str()))
            .collect();
        assert_eq!(vendored, vec![("cline", "x")]);
    }
}
