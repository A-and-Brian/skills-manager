//! Moving a project's skills onto a chosen set of agents: which per-agent
//! deployments to add or remove, and carrying that out.
//!
//! Agents here are the project group keys from `agent_skill_configs`: one key
//! per project skills directory, however many adapters share it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde::Serialize;

use super::project_scanner::{AgentSkillConfig, ProjectSkillInfo};
use super::sync_engine::{self, ReplacePolicy, SyncMode, TargetState};

/// Why an agent the change would touch was left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The skill's agents were chosen by hand; bulk changes leave it alone.
    Overridden,
    /// No library skill to deploy from, so the agent cannot be added.
    NotInLibrary,
    /// The copy is a real directory. A bulk change only ever removes links.
    RealDirectory,
    /// The agent is selected but not installed, or turned off.
    UnavailableAgent,
    /// The agent reads a folder shared with other agents, which removing one
    /// agent must not take away from the rest.
    SharedDir,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedAgent {
    pub agent: String,
    pub reason: SkipReason,
}

/// What it takes to put one logical skill on exactly the desired agents.
#[derive(Debug, Clone, Serialize)]
pub struct SkillChange {
    pub relative_path: String,
    pub name: String,
    pub adds: Vec<String>,
    pub removes: Vec<String>,
    pub skipped: Vec<SkippedAgent>,
    /// Library copy the adds deploy from.
    #[serde(skip)]
    source: Option<PathBuf>,
    /// Adds land disabled when every existing copy is disabled, so adding an
    /// agent does not quietly switch the skill back on.
    #[serde(skip)]
    enabled: bool,
}

impl SkillChange {
    fn is_empty(&self) -> bool {
        self.adds.is_empty() && self.removes.is_empty() && self.skipped.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentChangePlan {
    /// Only skills with something to add, remove or report.
    pub skills: Vec<SkillChange>,
    /// Skills whose agents were chosen by hand, kept as they are.
    pub overridden: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailedAgent {
    pub agent: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillOutcome {
    pub relative_path: String,
    pub name: String,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub skipped: Vec<SkippedAgent>,
    pub failed: Vec<FailedAgent>,
}

/// Plan moving every skill in a project onto `desired`, except skills listed
/// in `overrides` (keyed by relative path), which are reported and left alone.
pub fn plan_agent_change(
    skills: &[ProjectSkillInfo],
    desired: &[String],
    available: &HashSet<String>,
    overrides: &HashMap<String, Vec<String>>,
    library_source: impl Fn(&ProjectSkillInfo) -> Option<PathBuf>,
) -> AgentChangePlan {
    let overridden: HashSet<String> = overrides.keys().map(|path| path.to_lowercase()).collect();
    let mut plan = AgentChangePlan::default();
    for variants in group_by_skill(skills) {
        let source = variants.iter().find_map(|variant| library_source(variant));
        let mut change = plan_skill_change(&variants, desired, available, source, false);
        if overridden.contains(&variants[0].relative_path.to_lowercase()) {
            plan.overridden += 1;
            let touched: Vec<String> = change
                .adds
                .drain(..)
                .chain(change.removes.drain(..))
                .collect();
            change.skipped = touched
                .into_iter()
                .map(|agent| SkippedAgent {
                    agent,
                    reason: SkipReason::Overridden,
                })
                .collect();
        }
        if !change.is_empty() {
            plan.skills.push(change);
        }
    }
    plan
}

/// Plan moving one skill onto `desired`. `variants` are its per-agent copies,
/// enabled or disabled, and must not be empty.
///
/// A real directory is only removed with `remove_real_dirs`, which is for an
/// explicit per-skill choice; set-level changes leave real directories alone.
pub fn plan_skill_change(
    variants: &[&ProjectSkillInfo],
    desired: &[String],
    available: &HashSet<String>,
    source: Option<PathBuf>,
    remove_real_dirs: bool,
) -> SkillChange {
    let mut present: Vec<&str> = Vec::new();
    for variant in variants {
        if !present.contains(&variant.agent.as_str()) {
            present.push(&variant.agent);
        }
    }

    let mut change = SkillChange {
        relative_path: variants[0].relative_path.clone(),
        name: variants[0].name.clone(),
        adds: Vec::new(),
        removes: Vec::new(),
        skipped: Vec::new(),
        enabled: variants.iter().any(|variant| variant.enabled),
        source,
    };

    for agent in desired {
        if present.contains(&agent.as_str()) {
            continue;
        }
        let reason = if !available.contains(agent) {
            Some(SkipReason::UnavailableAgent)
        } else if change.source.is_none() {
            Some(SkipReason::NotInLibrary)
        } else {
            None
        };
        match reason {
            Some(reason) => change.skipped.push(SkippedAgent {
                agent: agent.clone(),
                reason,
            }),
            None => change.adds.push(agent.clone()),
        }
    }

    for agent in present {
        if desired.iter().any(|key| key == agent) {
            continue;
        }
        let only_links = variants
            .iter()
            .filter(|variant| variant.agent == agent)
            .all(|variant| is_link(Path::new(&variant.path)));
        if only_links || remove_real_dirs {
            change.removes.push(agent.to_string());
        } else {
            change.skipped.push(SkippedAgent {
                agent: agent.to_string(),
                reason: SkipReason::RealDirectory,
            });
        }
    }

    change
}

/// Carry out planned changes, continuing past failures so one broken agent
/// does not strand the rest. Removals re-read what is on disk rather than
/// trusting the plan. `configured_mode` is the global `sync_mode` setting.
pub fn apply_agent_change(
    project_root: &Path,
    configs: &[AgentSkillConfig],
    changes: &[SkillChange],
    configured_mode: Option<&str>,
    remove_real_dirs: bool,
) -> Vec<SkillOutcome> {
    changes
        .iter()
        .map(|change| {
            apply_skill_change(
                project_root,
                configs,
                change,
                configured_mode,
                remove_real_dirs,
            )
        })
        .collect()
}

fn apply_skill_change(
    project_root: &Path,
    configs: &[AgentSkillConfig],
    change: &SkillChange,
    configured_mode: Option<&str>,
    remove_real_dirs: bool,
) -> SkillOutcome {
    let mut outcome = SkillOutcome {
        relative_path: change.relative_path.clone(),
        name: change.name.clone(),
        added: Vec::new(),
        removed: Vec::new(),
        skipped: change.skipped.clone(),
        failed: Vec::new(),
    };

    for agent in &change.adds {
        let result = agent_roots(project_root, configs, agent).and_then(|(root, disabled)| {
            let source = change
                .source
                .as_deref()
                .ok_or_else(|| anyhow!("No library skill to deploy from"))?;
            let root = if change.enabled { root } else { disabled };
            let mode = sync_engine::sync_mode_for_tool(agent, configured_mode);
            deploy_skill(source, &root, &change.relative_path, mode)
        });
        match result {
            Ok(()) => outcome.added.push(agent.clone()),
            Err(err) => outcome.failed.push(FailedAgent {
                agent: agent.clone(),
                error: format!("{err:#}"),
            }),
        }
    }

    for agent in &change.removes {
        let result = agent_roots(project_root, configs, agent).and_then(|(root, disabled)| {
            remove_deployments(&[root, disabled], &change.relative_path, remove_real_dirs)
        });
        match result {
            Ok(true) => outcome.removed.push(agent.clone()),
            Ok(false) => outcome.skipped.push(SkippedAgent {
                agent: agent.clone(),
                reason: SkipReason::RealDirectory,
            }),
            Err(err) => outcome.failed.push(FailedAgent {
                agent: agent.clone(),
                error: format!("{err:#}"),
            }),
        }
    }

    outcome
}

/// Deploy the library skill at `source` as `skills_root/relative_path`.
///
/// NoClobber: callers only deploy where the skill is not present yet, so
/// nothing here should need replacing. Belt and braces — `exists()` misses
/// dangling links and is racy against this write.
pub fn deploy_skill(
    source: &Path,
    skills_root: &Path,
    relative_path: &str,
    mode: SyncMode,
) -> Result<()> {
    std::fs::create_dir_all(skills_root)?;
    sync_engine::sync_skill(
        source,
        &skills_root.join(relative_path),
        mode,
        ReplacePolicy::NoClobber,
    )?;
    Ok(())
}

/// Remove one agent's copies of a skill from its enabled and disabled roots.
/// Returns `false`, touching nothing, when a copy is a real directory and
/// `remove_real_dirs` is off. Even with it, at most one real directory goes,
/// as deleting one agent's copy always took one.
fn remove_deployments(
    roots: &[PathBuf],
    relative_path: &str,
    remove_real_dirs: bool,
) -> Result<bool> {
    let targets = roots
        .iter()
        .map(|root| {
            let target = root.join(relative_path);
            let state = sync_engine::classify_target(&target, None)?;
            Ok((target, state))
        })
        .collect::<Result<Vec<_>>>()?;
    let real = targets
        .iter()
        .filter(|(_, state)| matches!(state, TargetState::RealDir | TargetState::RealFile))
        .count();
    if real > 0 && !remove_real_dirs {
        return Ok(false);
    }
    if real > 1 {
        return Err(anyhow!(
            "\"{relative_path}\" is a real folder both enabled and disabled for this agent — resolve manually"
        ));
    }
    for (target, state) in targets {
        sync_engine::remove_classified_target(&target, state)?;
    }
    Ok(true)
}

fn agent_roots(
    project_root: &Path,
    configs: &[AgentSkillConfig],
    agent: &str,
) -> Result<(PathBuf, PathBuf)> {
    let config = configs
        .iter()
        .find(|config| config.key == agent)
        .ok_or_else(|| anyhow!("Unknown agent: {agent}"))?;
    Ok((
        project_root.join(&config.relative_skills_dir),
        project_root.join(format!("{}-disabled", config.relative_skills_dir)),
    ))
}

fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

/// Per-agent copies of each logical skill, folded by relative path the way
/// the project view shows them.
fn group_by_skill(skills: &[ProjectSkillInfo]) -> Vec<Vec<&ProjectSkillInfo>> {
    let mut groups: Vec<Vec<&ProjectSkillInfo>> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for skill in skills {
        let key = skill.relative_path.to_lowercase();
        match index.get(&key) {
            Some(&at) => groups[at].push(skill),
            None => {
                index.insert(key, groups.len());
                groups.push(vec![skill]);
            }
        }
    }
    groups
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::core::project_scanner::read_project_skills;
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::{tempdir, TempDir};

    struct Project {
        _tmp: TempDir,
        root: PathBuf,
        library: PathBuf,
        configs: Vec<AgentSkillConfig>,
    }

    /// A project with Claude Code, Cursor and Codex, and a library holding
    /// one skill, `x`.
    fn project() -> Project {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("project");
        let library = tmp.path().join("library").join("x");
        fs::create_dir_all(&library).unwrap();
        fs::write(library.join("SKILL.md"), "---\nname: x\n---\n").unwrap();
        let configs = [
            ("claude_code", ".claude/skills"),
            ("cursor", ".cursor/skills"),
            ("codex", ".codex/skills"),
        ]
        .into_iter()
        .map(|(key, dir)| AgentSkillConfig {
            key: key.to_string(),
            display_name: key.to_string(),
            relative_skills_dir: dir.to_string(),
        })
        .collect();
        Project {
            _tmp: tmp,
            root,
            library,
            configs,
        }
    }

    impl Project {
        fn link(&self, dir: &str, name: &str) -> PathBuf {
            let path = self.root.join(dir).join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            symlink(&self.library, &path).unwrap();
            path
        }

        fn real(&self, dir: &str, name: &str) -> PathBuf {
            let path = self.root.join(dir).join(name);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("SKILL.md"), format!("---\nname: {name}\n---\n")).unwrap();
            path
        }

        fn plan(
            &self,
            desired: &[&str],
            available: &[&str],
            overrides: &HashMap<String, Vec<String>>,
        ) -> AgentChangePlan {
            plan_agent_change(
                &read_project_skills(&self.root, &self.configs),
                &keys(desired),
                &keys(available).into_iter().collect(),
                overrides,
                |skill| (skill.dir_name == "x").then(|| self.library.clone()),
            )
        }

        fn apply(&self, plan: &AgentChangePlan) -> Vec<SkillOutcome> {
            apply_agent_change(&self.root, &self.configs, &plan.skills, None, false)
        }
    }

    fn keys(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    const ALL: &[&str] = &["claude_code", "cursor", "codex"];

    fn skipped(agent: &str, reason: SkipReason) -> SkippedAgent {
        SkippedAgent {
            agent: agent.to_string(),
            reason,
        }
    }

    #[test]
    fn adding_an_agent_deploys_library_skills_and_reports_the_rest() {
        let project = project();
        project.link(".claude/skills", "x");
        project.real(".claude/skills", "local");

        let plan = project.plan(
            &["claude_code", "cursor", "codex"],
            &["claude_code", "cursor"],
            &HashMap::new(),
        );

        let local = plan
            .skills
            .iter()
            .find(|s| s.relative_path == "local")
            .unwrap();
        assert!(local.adds.is_empty());
        assert_eq!(
            local.skipped,
            vec![
                skipped("cursor", SkipReason::NotInLibrary),
                skipped("codex", SkipReason::UnavailableAgent),
            ]
        );
        let x = plan.skills.iter().find(|s| s.relative_path == "x").unwrap();
        assert_eq!(x.adds, vec!["cursor"]);
        assert_eq!(
            x.skipped,
            vec![skipped("codex", SkipReason::UnavailableAgent)]
        );

        let outcomes = project.apply(&plan);

        assert!(outcomes.iter().all(|outcome| outcome.failed.is_empty()));
        let deployed = project.root.join(".cursor/skills/x");
        assert_eq!(fs::read_link(&deployed).unwrap(), project.library);
        assert!(!project.root.join(".cursor/skills/local").exists());
    }

    #[test]
    fn removing_an_agent_unlinks_symlinks_and_keeps_real_directories() {
        let project = project();
        project.link(".claude/skills", "x");
        let cursor = project.link(".cursor/skills", "x");
        let codex = project.real(".codex/skills", "x");

        let plan = project.plan(&["claude_code"], ALL, &HashMap::new());

        assert_eq!(plan.skills.len(), 1);
        assert_eq!(plan.skills[0].removes, vec!["cursor"]);
        assert_eq!(
            plan.skills[0].skipped,
            vec![skipped("codex", SkipReason::RealDirectory)]
        );

        let outcomes = project.apply(&plan);

        assert_eq!(outcomes[0].removed, vec!["cursor"]);
        assert!(fs::symlink_metadata(&cursor).is_err());
        assert!(project.library.join("SKILL.md").is_file());
        assert!(codex.join("SKILL.md").is_file());
    }

    #[test]
    fn overridden_skills_are_reported_and_left_alone() {
        let project = project();
        project.link(".claude/skills", "x");
        let overrides = HashMap::from([("x".to_string(), keys(&["claude_code"]))]);

        let plan = project.plan(&["cursor"], ALL, &overrides);

        assert_eq!(plan.overridden, 1);
        assert!(plan.skills[0].adds.is_empty() && plan.skills[0].removes.is_empty());
        assert_eq!(
            plan.skills[0].skipped,
            vec![
                skipped("cursor", SkipReason::Overridden),
                skipped("claude_code", SkipReason::Overridden),
            ]
        );

        project.apply(&plan);

        assert!(project.root.join(".claude/skills/x").exists());
        assert!(!project.root.join(".cursor/skills/x").exists());
    }

    #[test]
    fn disabled_copies_count_as_present_and_adds_stay_disabled() {
        let project = project();
        project.link(".claude/skills-disabled", "x");
        let cursor = project.link(".cursor/skills-disabled", "x");

        let plan = project.plan(&["claude_code", "codex"], ALL, &HashMap::new());

        assert_eq!(plan.skills[0].adds, vec!["codex"]);
        assert_eq!(plan.skills[0].removes, vec!["cursor"]);

        project.apply(&plan);

        assert!(fs::symlink_metadata(&cursor).is_err());
        assert!(project
            .root
            .join(".codex/skills-disabled/x/SKILL.md")
            .is_file());
        assert!(!project.root.join(".codex/skills/x").exists());
    }

    /// The plan is advice; removal decides from what is on disk when it runs.
    #[test]
    fn a_link_replaced_by_a_real_directory_after_planning_is_kept() {
        let project = project();
        project.link(".claude/skills", "x");
        let cursor = project.link(".cursor/skills", "x");
        let plan = project.plan(&["claude_code"], ALL, &HashMap::new());
        assert_eq!(plan.skills[0].removes, vec!["cursor"]);

        fs::remove_file(&cursor).unwrap();
        project.real(".cursor/skills", "x");
        let outcomes = project.apply(&plan);

        assert!(outcomes[0].removed.is_empty());
        assert_eq!(
            outcomes[0].skipped,
            vec![skipped("cursor", SkipReason::RealDirectory)]
        );
        assert!(cursor.join("SKILL.md").is_file());
    }

    /// Deleting one agent's copy took one folder; with a real folder both
    /// enabled and disabled, neither is guessed at.
    #[test]
    fn a_per_skill_change_deletes_no_more_than_one_real_directory() {
        let project = project();
        project.link(".claude/skills", "x");
        let enabled = project.real(".codex/skills", "x");
        let disabled = project.real(".codex/skills-disabled", "x");
        let skills = read_project_skills(&project.root, &project.configs);
        let variants: Vec<&ProjectSkillInfo> = skills.iter().collect();
        let change = plan_skill_change(
            &variants,
            &keys(&["claude_code"]),
            &keys(ALL).into_iter().collect(),
            Some(project.library.clone()),
            true,
        );
        assert_eq!(change.removes, vec!["codex"]);

        let outcomes = apply_agent_change(&project.root, &project.configs, &[change], None, true);

        assert!(outcomes[0].removed.is_empty());
        assert_eq!(outcomes[0].failed[0].agent, "codex");
        assert!(enabled.join("SKILL.md").is_file());
        assert!(disabled.join("SKILL.md").is_file());
    }

    /// Unticking one agent on one skill is an explicit delete of that copy,
    /// as it was before agent selection existed.
    #[test]
    fn a_per_skill_change_may_remove_a_real_directory() {
        let project = project();
        project.link(".claude/skills", "x");
        let codex = project.real(".codex/skills", "x");
        let skills = read_project_skills(&project.root, &project.configs);
        let variants: Vec<&ProjectSkillInfo> = skills.iter().collect();

        let change = plan_skill_change(
            &variants,
            &keys(&["claude_code"]),
            &keys(ALL).into_iter().collect(),
            Some(project.library.clone()),
            true,
        );
        assert_eq!(change.removes, vec!["codex"]);

        let outcomes = apply_agent_change(&project.root, &project.configs, &[change], None, true);

        assert_eq!(outcomes[0].removed, vec!["codex"]);
        assert!(!codex.exists());
        assert!(project.root.join(".claude/skills/x").exists());
    }
}
