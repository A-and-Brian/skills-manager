//! Moving a project's skills onto a chosen set of agents: which per-agent
//! deployments to add or remove, and carrying that out.
//!
//! Agents here are the project group keys from `agent_skill_configs`: one key
//! per project skills directory, however many adapters share it.
//!
//! A copy-mode project vendors each skill's files into `.agents/skills` (the
//! agents reading that folder use them as they are) and gives every other
//! agent a relative link there, so the layout survives a commit and a clone.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use serde::Serialize;

use super::project_scanner::{AgentSkillConfig, ProjectSkillInfo, VENDORED_SKILLS_DIR};
use super::sync_engine::{self, ReplacePolicy, SyncMode, TargetState};
use super::{content_hash, skill_metadata};

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
    /// In a copy-mode project, a link to something other than the vendored
    /// copy: the user's own, which a bulk change leaves alone.
    ForeignLink,
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
    /// Planned for a copy-mode project: adds link to the vendored copy.
    #[serde(skip)]
    vendored: bool,
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
/// A skill with a vendored copy on disk, left from before the project went
/// back to linking, is planned by the copy-mode rules: adds link to it.
pub fn plan_agent_change(
    project_root: &Path,
    configs: &[AgentSkillConfig],
    skills: &[ProjectSkillInfo],
    desired: &[String],
    available: &HashSet<String>,
    overrides: &HashMap<String, Vec<String>>,
    library_source: impl Fn(&ProjectSkillInfo) -> Option<PathBuf>,
) -> AgentChangePlan {
    plan_skills(skills, overrides, |variants| {
        let source = variants.iter().find_map(|variant| library_source(variant));
        if vendored_copy(project_root, &variants[0].relative_path).is_some() {
            plan_copy_skill_change(configs, variants, desired, available, source, false)
        } else {
            plan_skill_change(variants, desired, available, source, false)
        }
    })
}

/// [`plan_agent_change`] for a copy-mode project. On top of the skills, it
/// offers to remove agent links into `.agents/skills` whose vendored copy is
/// gone, which the scan cannot see.
pub fn plan_copy_agent_change(
    project_root: &Path,
    configs: &[AgentSkillConfig],
    skills: &[ProjectSkillInfo],
    desired: &[String],
    available: &HashSet<String>,
    overrides: &HashMap<String, Vec<String>>,
    library_source: impl Fn(&ProjectSkillInfo) -> Option<PathBuf>,
) -> AgentChangePlan {
    let mut plan = plan_skills(skills, overrides, |variants| {
        let source = variants.iter().find_map(|variant| library_source(variant));
        plan_copy_skill_change(configs, variants, desired, available, source, false)
    });
    for (agent, relative_path) in stale_vendored_links(project_root, configs) {
        if overrides
            .keys()
            .any(|path| path.eq_ignore_ascii_case(&relative_path))
        {
            continue;
        }
        let existing = plan
            .skills
            .iter_mut()
            .find(|change| change.relative_path.eq_ignore_ascii_case(&relative_path));
        match existing {
            Some(change) if change.removes.contains(&agent) => {}
            Some(change) => change.removes.push(agent),
            None => plan.skills.push(SkillChange {
                name: relative_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&relative_path)
                    .to_string(),
                relative_path,
                adds: Vec::new(),
                removes: vec![agent],
                skipped: Vec::new(),
                source: None,
                enabled: true,
                vendored: true,
            }),
        }
    }
    plan
}

/// Plan every logical skill with `plan_one`, turning the changes of skills in
/// `overrides` into [`SkipReason::Overridden`].
fn plan_skills(
    skills: &[ProjectSkillInfo],
    overrides: &HashMap<String, Vec<String>>,
    plan_one: impl Fn(&[&ProjectSkillInfo]) -> SkillChange,
) -> AgentChangePlan {
    let overridden: HashSet<String> = overrides.keys().map(|path| path.to_lowercase()).collect();
    let mut plan = AgentChangePlan::default();
    for variants in group_by_skill(skills) {
        let mut change = plan_one(&variants);
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
    plan_variants(variants, desired, available, source, remove_real_dirs, None)
}

/// [`plan_skill_change`] for a copy-mode project. Adds link to the vendored
/// copy, so they need a library skill only while there is none; the agents
/// reading `.agents/skills` hold the vendored copy itself and are never
/// removed ([`SkipReason::SharedDir`]).
pub fn plan_copy_skill_change(
    configs: &[AgentSkillConfig],
    variants: &[&ProjectSkillInfo],
    desired: &[String],
    available: &HashSet<String>,
    source: Option<PathBuf>,
    remove_real_dirs: bool,
) -> SkillChange {
    plan_variants(
        variants,
        desired,
        available,
        source,
        remove_real_dirs,
        Some(configs),
    )
}

/// `copy_mode` carries the agent groups of a copy-mode project.
fn plan_variants(
    variants: &[&ProjectSkillInfo],
    desired: &[String],
    available: &HashSet<String>,
    source: Option<PathBuf>,
    remove_real_dirs: bool,
    copy_mode: Option<&[AgentSkillConfig]>,
) -> SkillChange {
    let reads_vendored =
        |agent: &str| copy_mode.is_some_and(|configs| is_vendored_agent(configs, agent));
    let has_vendored = variants
        .iter()
        .any(|variant| reads_vendored(&variant.agent) && !is_link(Path::new(&variant.path)));

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
        vendored: copy_mode.is_some(),
    };

    for agent in desired {
        if present.contains(&agent.as_str()) {
            continue;
        }
        let reason = if !available.contains(agent) {
            Some(SkipReason::UnavailableAgent)
        } else if change.source.is_none() && !has_vendored {
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
        if reads_vendored(agent) {
            change.skipped.push(SkippedAgent {
                agent: agent.to_string(),
                reason: SkipReason::SharedDir,
            });
            continue;
        }
        // Copy mode takes only links into `.agents/skills`.
        let kept = variants
            .iter()
            .filter(|variant| variant.agent == agent)
            .find_map(|variant| {
                if !is_link(Path::new(&variant.path)) {
                    Some(SkipReason::RealDirectory)
                } else if copy_mode.is_some() && variant.alias_of.is_none() {
                    Some(SkipReason::ForeignLink)
                } else {
                    None
                }
            });
        match kept {
            Some(reason) if !remove_real_dirs => change.skipped.push(SkippedAgent {
                agent: agent.to_string(),
                reason,
            }),
            _ => change.removes.push(agent.to_string()),
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

    for agent in &change.removes {
        let result = agent_roots(project_root, configs, agent).and_then(|(root, disabled)| {
            let roots = [root, disabled];
            if change.vendored
                && !remove_real_dirs
                && has_foreign_link(project_root, &roots, &change.relative_path)
            {
                return Ok(Some(SkipReason::ForeignLink));
            }
            remove_deployments(&roots, &change.relative_path, remove_real_dirs)
                .map(|removed| (!removed).then_some(SkipReason::RealDirectory))
        });
        match result {
            Ok(None) => outcome.removed.push(agent.clone()),
            Ok(Some(reason)) => outcome.skipped.push(SkippedAgent {
                agent: agent.clone(),
                reason,
            }),
            Err(err) => outcome.failed.push(FailedAgent {
                agent: agent.clone(),
                error: format!("{err:#}"),
            }),
        }
    }

    // After removals, so a stale link an add replaces is already gone.
    for agent in &change.adds {
        let result = if change.vendored {
            add_vendored(project_root, configs, change, agent)
        } else {
            agent_roots(project_root, configs, agent).and_then(|(root, disabled)| {
                let source = change
                    .source
                    .as_deref()
                    .ok_or_else(|| anyhow!("No library skill to deploy from"))?;
                let root = if change.enabled { root } else { disabled };
                let mode = sync_engine::sync_mode_for_tool(agent, configured_mode);
                deploy_skill(source, &root, &change.relative_path, mode)
            })
        };
        match result {
            Ok(()) => outcome.added.push(agent.clone()),
            Err(err) => outcome.failed.push(FailedAgent {
                agent: agent.clone(),
                error: format!("{err:#}"),
            }),
        }
    }

    outcome
}

/// Add one agent to a skill of a copy-mode project: vendor it from the
/// library if it is not vendored yet, then link the agent to it.
fn add_vendored(
    project_root: &Path,
    configs: &[AgentSkillConfig],
    change: &SkillChange,
    agent: &str,
) -> Result<()> {
    let (vendored, enabled) = vendored_path(project_root, &change.relative_path, change.enabled);
    if sync_engine::classify_target(&vendored, None)? != TargetState::RealDir {
        let source = change
            .source
            .as_deref()
            .ok_or_else(|| anyhow!("No vendored copy or library skill to deploy from"))?;
        vendor_skill(source, &vendored)?;
    }
    link_agent(
        project_root,
        configs,
        agent,
        &change.relative_path,
        &vendored,
        enabled,
    )
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

/// Whether a copy of the skill under `roots` is a link that does not lead
/// into `.agents/skills`.
fn has_foreign_link(project_root: &Path, roots: &[PathBuf], relative_path: &str) -> bool {
    let vendored_roots = vendored_roots(project_root);
    roots.iter().any(|root| {
        let link = root.join(relative_path);
        is_link(&link)
            && !lexical_link_target(&link)
                .is_some_and(|target| vendored_roots.iter().any(|root| target.starts_with(root)))
    })
}

/// Whether an agent group's project folder is `.agents/skills` itself: its
/// agents read the vendored copy and need no link. `.` segments do not count.
pub fn is_vendored_dir(relative_skills_dir: &str) -> bool {
    normal_parts(relative_skills_dir).eq(normal_parts(VENDORED_SKILLS_DIR))
}

fn normal_parts(dir: &str) -> impl Iterator<Item = Component<'_>> {
    Path::new(dir)
        .components()
        .filter(|part| matches!(part, Component::Normal(_)))
}

pub fn is_vendored_agent(configs: &[AgentSkillConfig], agent: &str) -> bool {
    configs
        .iter()
        .any(|config| config.key == agent && is_vendored_dir(&config.relative_skills_dir))
}

/// A skill's vendored copy and whether it is enabled: where it is now, or
/// where `enabled` puts a new one.
pub fn vendored_path(project_root: &Path, relative_path: &str, enabled: bool) -> (PathBuf, bool) {
    let [on, off] = vendored_slots(project_root, relative_path);
    if exists(&on) {
        (on, true)
    } else if exists(&off) {
        (off, false)
    } else if enabled {
        (on, true)
    } else {
        (off, false)
    }
}

/// The skill's vendored copy, when it has one: a real directory, not a link.
pub fn vendored_copy(project_root: &Path, relative_path: &str) -> Option<PathBuf> {
    let (path, _) = vendored_path(project_root, relative_path, true);
    std::fs::symlink_metadata(&path)
        .is_ok_and(|metadata| metadata.is_dir())
        .then_some(path)
}

/// `.agents/skills` and its disabled twin.
fn vendored_roots(project_root: &Path) -> [PathBuf; 2] {
    [
        project_root.join(VENDORED_SKILLS_DIR),
        project_root.join(format!("{VENDORED_SKILLS_DIR}-disabled")),
    ]
}

/// Where a skill's vendored copy sits when enabled, and when disabled.
fn vendored_slots(project_root: &Path, relative_path: &str) -> [PathBuf; 2] {
    vendored_roots(project_root).map(|root| root.join(relative_path))
}

/// Vendor the library skill at `source` into `vendored` as real files. A copy
/// already there is kept when its content matches and refused otherwise, so
/// no edit made in the project is overwritten; a link there (a link-mode
/// deployment) is replaced.
pub fn vendor_skill(source: &Path, vendored: &Path) -> Result<()> {
    if sync_engine::classify_target(vendored, Some(source))? == TargetState::RealDir {
        if content_hash::hash_directory(vendored)? == content_hash::hash_directory(source)? {
            return Ok(());
        }
        bail!(
            "{:?} already exists in {VENDORED_SKILLS_DIR} with different content; \
             update it from the library or remove it first",
            vendored.file_name().unwrap_or_default()
        );
    }
    sync_engine::sync_skill(source, vendored, SyncMode::Copy, ReplacePolicy::NoClobber)?;
    Ok(())
}

/// Deploy a library skill to a copy-mode project: vendor it, then link every
/// agent in `agents` that does not read `.agents/skills` to it. The vendored
/// copy is written even with no agents. Fails without linking anything when
/// the vendored copy cannot be written; returns the agents whose link failed.
pub fn deploy_copy_mode(
    project_root: &Path,
    configs: &[AgentSkillConfig],
    source: &Path,
    relative_path: &str,
    agents: &[String],
) -> Result<Vec<FailedAgent>> {
    let (vendored, enabled) = vendored_path(project_root, relative_path, true);
    vendor_skill(source, &vendored)?;
    Ok(agents
        .iter()
        .filter_map(|agent| {
            link_agent(
                project_root,
                configs,
                agent,
                relative_path,
                &vendored,
                enabled,
            )
            .err()
            .map(|err| FailedAgent {
                agent: agent.clone(),
                error: format!("{err:#}"),
            })
        })
        .collect())
}

fn link_agent(
    project_root: &Path,
    configs: &[AgentSkillConfig],
    agent: &str,
    relative_path: &str,
    vendored: &Path,
    enabled: bool,
) -> Result<()> {
    if is_vendored_agent(configs, agent) {
        return Ok(());
    }
    let (root, disabled) = agent_roots(project_root, configs, agent)?;
    let root = if enabled { root } else { disabled };
    sync_engine::link_skill_relative(
        &root.join(relative_path),
        vendored,
        project_root,
        ReplacePolicy::NoClobber,
    )
}

/// Whether `agent`'s copy of a skill is the vendored copy or a link to it.
pub fn shares_vendored_copy(
    project_root: &Path,
    configs: &[AgentSkillConfig],
    agent: &str,
    relative_path: &str,
) -> bool {
    if is_vendored_agent(configs, agent) {
        return vendored_copy(project_root, relative_path).is_some();
    }
    let Ok(roots) = agent_roots(project_root, configs, agent) else {
        return false;
    };
    let slots = vendored_slots(project_root, relative_path);
    [roots.0, roots.1].iter().any(|root| {
        let link = root.join(relative_path);
        slots.iter().any(|slot| links_to(&link, slot))
    })
}

/// Enable or disable a vendored skill: move the vendored copy between
/// `.agents/skills` and its disabled twin, and move every agent link to it
/// along. Other copies of the skill are left as they are.
pub fn set_vendored_enabled(
    project_root: &Path,
    configs: &[AgentSkillConfig],
    relative_path: &str,
    enabled: bool,
) -> Result<()> {
    let [on, off] = vendored_slots(project_root, relative_path);
    let (from, to) = if enabled { (off, on) } else { (on, off) };
    match (exists(&from), exists(&to)) {
        (true, true) => bail!(
            "\"{relative_path}\" is both in {VENDORED_SKILLS_DIR} and its disabled folder — resolve manually"
        ),
        (true, false) => {
            crate::core::file_watcher::mute_self_writes(&from);
            crate::core::file_watcher::mute_self_writes(&to);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(&from, &to)?;
        }
        (false, true) => {}
        (false, false) => bail!("Vendored skill not found: {relative_path}"),
    }

    let slots = vendored_slots(project_root, relative_path);
    let mut failed = Vec::new();
    for config in link_configs(configs) {
        let (root, disabled) = config_roots(project_root, config);
        let new_link = if enabled { &root } else { &disabled }.join(relative_path);
        // Links to either slot, wherever they sit, all end up as one link.
        let links: Vec<PathBuf> = [root.join(relative_path), disabled.join(relative_path)]
            .into_iter()
            .filter(|link| slots.iter().any(|slot| links_to(link, slot)))
            .collect();
        if links.is_empty() {
            continue;
        }
        let policy = if links.contains(&new_link) {
            ReplacePolicy::Recorded { mode: "symlink" }
        } else {
            ReplacePolicy::NoClobber
        };
        // The new link first, so one that cannot be made leaves the old.
        let moved = sync_engine::link_skill_relative(&new_link, &to, project_root, policy)
            .and_then(|()| {
                links
                    .iter()
                    .filter(|link| **link != new_link)
                    .try_for_each(|link| {
                        sync_engine::remove_classified_target(link, TargetState::ForeignLink)
                    })
            });
        if let Err(err) = moved {
            failed.push(format!("{}: {err:#}", config.display_name));
        }
    }
    if !failed.is_empty() {
        bail!("Links not moved for {}", failed.join("; "));
    }
    Ok(())
}

/// Delete a vendored skill: first every agent link to it, then its files.
pub fn delete_vendored(
    project_root: &Path,
    configs: &[AgentSkillConfig],
    relative_path: &str,
) -> Result<()> {
    let (vendored, _) = vendored_path(project_root, relative_path, true);
    let slots = vendored_slots(project_root, relative_path);
    for config in link_configs(configs) {
        let (root, disabled) = config_roots(project_root, config);
        for link in [root.join(relative_path), disabled.join(relative_path)] {
            if slots.iter().any(|slot| links_to(&link, slot)) {
                sync_engine::remove_classified_target(&link, TargetState::ForeignLink)?;
            }
        }
    }
    sync_engine::remove_target(&vendored)
}

/// Agent links into `.agents/skills` that resolve to nothing, as
/// `(agent, relative path)`.
fn stale_vendored_links(
    project_root: &Path,
    configs: &[AgentSkillConfig],
) -> Vec<(String, String)> {
    let vendored_roots = vendored_roots(project_root);
    let mut stale = Vec::new();
    for config in link_configs(configs) {
        let (root, disabled) = config_roots(project_root, config);
        for root in [root, disabled] {
            collect_stale_links(&root, &root, &vendored_roots, &config.key, &mut stale);
        }
    }
    stale
}

/// Walks namespace directories only: a link inside a skill is its content.
fn collect_stale_links(
    root: &Path,
    dir: &Path,
    vendored_roots: &[PathBuf],
    agent: &str,
    stale: &mut Vec<(String, String)>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            let dangling = std::fs::metadata(&path).is_err();
            let into_vendored = lexical_link_target(&path)
                .is_some_and(|target| vendored_roots.iter().any(|root| target.starts_with(root)));
            if let (true, true, Ok(relative)) = (dangling, into_vendored, path.strip_prefix(root)) {
                let relative = relative.to_string_lossy().replace('\\', "/");
                stale.push((agent.to_string(), relative));
            }
        } else if file_type.is_dir() && !skill_metadata::is_valid_skill_dir(&path) {
            collect_stale_links(root, &path, vendored_roots, agent, stale);
        }
    }
}

/// How converting a link-mode project treats one agent's copy of a skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConvertAction {
    /// A link to the same content, re-pointed at the vendored copy.
    Relink,
    /// A real directory. Replacing it with a link would delete its files.
    KeepRealDir,
    /// A link to other content, the user's own.
    KeepForeignLink,
    /// Enabled where the vendored copy is disabled, or the other way round;
    /// relinking it would switch it.
    KeepMixedState,
}

#[derive(Debug, Clone, Serialize)]
pub struct VariantConversion {
    pub agent: String,
    pub action: ConvertAction,
    #[serde(skip)]
    path: PathBuf,
}

/// What converting to copy mode does to one logical skill.
#[derive(Debug, Clone, Serialize)]
pub struct SkillConversion {
    pub relative_path: String,
    pub name: String,
    /// Where the vendored copy comes from; `None` when it is vendored already.
    pub copy_from: Option<String>,
    /// Every agent copy outside `.agents/skills`.
    pub variants: Vec<VariantConversion>,
    #[serde(skip)]
    enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConversionOutcome {
    pub relative_path: String,
    pub name: String,
    /// The vendored copy was written.
    pub vendored: bool,
    pub relinked: Vec<String>,
    pub kept: Vec<VariantConversion>,
    pub failed: Vec<FailedAgent>,
    /// Why the vendored copy could not be written; nothing was relinked.
    pub error: Option<String>,
}

/// Plan converting a project to copy mode. Each skill is vendored from its
/// existing vendored copy, else from the first link, else the first real
/// directory; links to the same content become relative links to it and
/// everything else is kept and reported.
pub fn plan_convert_to_copy(
    project_root: &Path,
    configs: &[AgentSkillConfig],
    skills: &[ProjectSkillInfo],
) -> Vec<SkillConversion> {
    group_by_skill(skills)
        .iter()
        .filter_map(|variants| plan_skill_conversion(project_root, configs, variants))
        .collect()
}

fn plan_skill_conversion(
    project_root: &Path,
    configs: &[AgentSkillConfig],
    variants: &[&ProjectSkillInfo],
) -> Option<SkillConversion> {
    let (in_vendored, others): (Vec<&ProjectSkillInfo>, Vec<&ProjectSkillInfo>) = variants
        .iter()
        .copied()
        .partition(|variant| is_vendored_agent(configs, &variant.agent));
    let linked = |variant: &&&ProjectSkillInfo| is_link(Path::new(&variant.path));
    let existing = in_vendored.iter().find(|variant| !linked(variant));
    let source = existing
        .or_else(|| in_vendored.iter().chain(&others).find(linked))
        .or_else(|| others.iter().find(|variant| !linked(variant)))?;
    let wanted_enabled = match existing {
        Some(vendored) => vendored.enabled,
        None => variants.iter().any(|variant| variant.enabled),
    };
    // A slot already taken decides: the vendored copy is written there.
    let (vendored, enabled) = vendored_path(project_root, &source.relative_path, wanted_enabled);
    let copy_from = existing.is_none().then(|| {
        std::fs::canonicalize(&source.path)
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|_| source.path.clone())
    });

    let variants: Vec<VariantConversion> = others
        .iter()
        .filter_map(|variant| {
            let path = PathBuf::from(&variant.path);
            let action = if !is_link(&path) {
                ConvertAction::KeepRealDir
            } else if variant.enabled != enabled {
                ConvertAction::KeepMixedState
            } else if variant.content_hash != source.content_hash {
                ConvertAction::KeepForeignLink
            } else if std::fs::read_link(&path).ok()
                == sync_engine::relative_link_target(&path, &vendored, project_root).ok()
            {
                return None;
            } else {
                ConvertAction::Relink
            };
            Some(VariantConversion {
                agent: variant.agent.clone(),
                action,
                path,
            })
        })
        .collect();

    if copy_from.is_none() && variants.is_empty() {
        return None;
    }
    Some(SkillConversion {
        relative_path: source.relative_path.clone(),
        name: source.name.clone(),
        copy_from,
        variants,
        enabled,
    })
}

/// Carry out a conversion plan. Each copy is re-read before it is replaced:
/// only a link is ever swapped for a link.
pub fn apply_convert_to_copy(
    project_root: &Path,
    conversions: &[SkillConversion],
) -> Vec<ConversionOutcome> {
    conversions
        .iter()
        .map(|conversion| apply_skill_conversion(project_root, conversion))
        .collect()
}

fn apply_skill_conversion(project_root: &Path, conversion: &SkillConversion) -> ConversionOutcome {
    let mut outcome = ConversionOutcome {
        relative_path: conversion.relative_path.clone(),
        name: conversion.name.clone(),
        vendored: false,
        relinked: Vec::new(),
        kept: Vec::new(),
        failed: Vec::new(),
        error: None,
    };
    let (vendored, _) = vendored_path(project_root, &conversion.relative_path, conversion.enabled);
    if let Some(from) = &conversion.copy_from {
        if let Err(err) = vendor_skill(Path::new(from), &vendored) {
            outcome.error = Some(format!("{err:#}"));
            outcome.kept = conversion.variants.clone();
            return outcome;
        }
        outcome.vendored = true;
    }

    for variant in &conversion.variants {
        let still_a_link = is_link(&variant.path);
        if variant.action != ConvertAction::Relink || !still_a_link {
            outcome.kept.push(VariantConversion {
                action: if still_a_link {
                    variant.action
                } else {
                    ConvertAction::KeepRealDir
                },
                ..variant.clone()
            });
            continue;
        }
        let relinked = sync_engine::link_skill_relative(
            &variant.path,
            &vendored,
            project_root,
            ReplacePolicy::Recorded { mode: "symlink" },
        );
        match relinked {
            Ok(()) => outcome.relinked.push(variant.agent.clone()),
            Err(err) => outcome.failed.push(FailedAgent {
                agent: variant.agent.clone(),
                error: format!("{err:#}"),
            }),
        }
    }
    outcome
}

/// Agent groups that reach the vendored copy through a link.
fn link_configs(configs: &[AgentSkillConfig]) -> impl Iterator<Item = &AgentSkillConfig> {
    configs
        .iter()
        .filter(|config| !is_vendored_dir(&config.relative_skills_dir))
}

fn config_roots(project_root: &Path, config: &AgentSkillConfig) -> (PathBuf, PathBuf) {
    (
        project_root.join(&config.relative_skills_dir),
        project_root.join(format!("{}-disabled", config.relative_skills_dir)),
    )
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Whether `path` is a link naming `target`, whether or not it resolves.
fn links_to(path: &Path, target: &Path) -> bool {
    is_link(path) && lexical_link_target(path).as_deref() == Some(target)
}

/// Where a link points, worked out from the path alone so a link to
/// something gone still has an answer.
fn lexical_link_target(path: &Path) -> Option<PathBuf> {
    let joined = path.parent()?.join(std::fs::read_link(path).ok()?);
    let mut resolved = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            other => resolved.push(other),
        }
    }
    Some(resolved)
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
    Ok(config_roots(project_root, config))
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
        project_with(&[
            ("claude_code", ".claude/skills"),
            ("cursor", ".cursor/skills"),
            ("codex", ".codex/skills"),
        ])
    }

    fn project_with(agents: &[(&str, &str)]) -> Project {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("project");
        let library = tmp.path().join("library").join("x");
        fs::create_dir_all(&library).unwrap();
        fs::write(library.join("SKILL.md"), "---\nname: x\n---\n").unwrap();
        let configs = agents
            .iter()
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
                &self.root,
                &self.configs,
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

    // ── copy mode ──

    /// A copy-mode layout: Cline reads `.agents/skills` itself; Claude Code,
    /// Windsurf (one level deeper), Cursor and Codex need links.
    fn copy_project() -> Project {
        project_with(&[
            ("cline", ".agents/skills"),
            ("claude_code", ".claude/skills"),
            ("windsurf", ".codeium/windsurf/skills"),
            ("cursor", ".cursor/skills"),
            ("codex", ".codex/skills"),
        ])
    }

    impl Project {
        fn deploy_copy(&self, agents: &[&str]) -> Result<Vec<FailedAgent>> {
            deploy_copy_mode(&self.root, &self.configs, &self.library, "x", &keys(agents))
        }

        fn copy_plan(&self, desired: &[&str]) -> AgentChangePlan {
            plan_copy_agent_change(
                &self.root,
                &self.configs,
                &read_project_skills(&self.root, &self.configs),
                &keys(desired),
                &keys(&["cline", "claude_code", "windsurf", "cursor", "codex"])
                    .into_iter()
                    .collect(),
                &HashMap::new(),
                |skill| (skill.dir_name == "x").then(|| self.library.clone()),
            )
        }

        fn read_link(&self, path: &str) -> PathBuf {
            fs::read_link(self.root.join(path)).unwrap()
        }
    }

    fn is_real_dir(path: &Path) -> bool {
        fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
    }

    #[test]
    fn copy_mode_vendors_the_files_and_links_every_other_agent_relatively() {
        let project = copy_project();

        let failed = project
            .deploy_copy(&["cline", "claude_code", "windsurf"])
            .unwrap();

        assert!(failed.is_empty(), "{failed:?}");
        let vendored = project.root.join(".agents/skills/x");
        assert!(is_real_dir(&vendored));
        assert!(vendored.join("SKILL.md").is_file());
        assert_eq!(
            project.read_link(".claude/skills/x"),
            Path::new("../../.agents/skills/x")
        );
        assert_eq!(
            project.read_link(".codeium/windsurf/skills/x"),
            Path::new("../../../.agents/skills/x")
        );
        assert!(project
            .root
            .join(".codeium/windsurf/skills/x/SKILL.md")
            .is_file());
        assert!(!project.root.join(".cursor/skills/x").exists());
    }

    #[test]
    fn copy_mode_vendors_even_with_no_agents() {
        let project = copy_project();

        assert!(project.deploy_copy(&[]).unwrap().is_empty());

        assert!(is_real_dir(&project.root.join(".agents/skills/x")));
        assert!(!project.root.join(".claude").exists());
    }

    /// Redeploying the same content is fine; a vendored copy someone edited
    /// is never overwritten by a deploy.
    #[test]
    fn a_vendored_copy_is_reused_when_identical_and_refused_when_not() {
        let project = copy_project();
        project.deploy_copy(&["claude_code"]).unwrap();
        assert!(project
            .deploy_copy(&["claude_code", "cursor"])
            .unwrap()
            .is_empty());
        assert!(project.root.join(".cursor/skills/x/SKILL.md").is_file());

        let skill_md = project.root.join(".agents/skills/x/SKILL.md");
        fs::write(&skill_md, "edited in the repo").unwrap();
        let err = project.deploy_copy(&["codex"]).unwrap_err();

        assert!(err.to_string().contains("different content"), "{err}");
        assert_eq!(fs::read_to_string(&skill_md).unwrap(), "edited in the repo");
        assert!(!project.root.join(".codex/skills/x").exists());
    }

    /// A link-mode link to the library in the vendored folder becomes the
    /// vendored copy.
    #[test]
    fn a_library_link_in_the_vendored_folder_is_replaced_by_a_copy() {
        let project = copy_project();
        project.link(".agents/skills", "x");

        project.deploy_copy(&[]).unwrap();

        assert!(is_real_dir(&project.root.join(".agents/skills/x")));
        assert!(project.library.join("SKILL.md").is_file());
    }

    #[test]
    fn copy_planner_links_skills_without_a_library_match_and_keeps_the_vendored_folder() {
        let project = copy_project();
        project.real(".agents/skills", "local");

        let plan = project.copy_plan(&["claude_code"]);

        assert_eq!(plan.skills.len(), 1);
        assert_eq!(plan.skills[0].adds, vec!["claude_code"]);
        assert_eq!(
            plan.skills[0].skipped,
            vec![skipped("cline", SkipReason::SharedDir)]
        );

        let outcomes = project.apply(&plan);

        assert_eq!(outcomes[0].added, vec!["claude_code"]);
        assert_eq!(
            project.read_link(".claude/skills/local"),
            Path::new("../../.agents/skills/local")
        );
        assert!(is_real_dir(&project.root.join(".agents/skills/local")));
    }

    #[test]
    fn copy_planner_vendors_a_library_skill_before_linking_it() {
        let project = copy_project();
        project.link(".cursor/skills", "x");

        let plan = project.copy_plan(&["cursor", "claude_code"]);
        project.apply(&plan);

        assert!(is_real_dir(&project.root.join(".agents/skills/x")));
        assert_eq!(
            project.read_link(".claude/skills/x"),
            Path::new("../../.agents/skills/x")
        );
    }

    /// Links whose vendored copy is gone resolve to nothing, so the scan
    /// cannot see them; the plan offers to remove them anyway.
    #[test]
    fn copy_planner_offers_to_remove_links_into_a_vendored_copy_that_is_gone() {
        let project = copy_project();
        let stale = project.root.join(".claude/skills/team/gone");
        fs::create_dir_all(stale.parent().unwrap()).unwrap();
        symlink("../../../.agents/skills/team/gone", &stale).unwrap();
        let elsewhere = project.root.join(".claude/skills/elsewhere");
        symlink("/nonexistent/elsewhere", &elsewhere).unwrap();

        let plan = project.copy_plan(&["claude_code"]);

        assert_eq!(plan.skills.len(), 1);
        assert_eq!(plan.skills[0].relative_path, "team/gone");
        assert_eq!(plan.skills[0].removes, vec!["claude_code"]);

        let outcomes = project.apply(&plan);

        assert_eq!(outcomes[0].removed, vec!["claude_code"]);
        assert!(fs::symlink_metadata(&stale).is_err());
        assert!(fs::symlink_metadata(&elsewhere).is_ok());
    }

    /// A bulk change in a copy-mode project takes only links into
    /// `.agents/skills`; the user's own links elsewhere stay, also when a
    /// link was re-pointed after planning.
    #[test]
    fn copy_planner_keeps_links_that_do_not_lead_to_the_vendored_copy() {
        let project = copy_project();
        project.deploy_copy(&["claude_code"]).unwrap();
        let cursor = project.link(".cursor/skills", "x");

        let plan = project.copy_plan(&["cline"]);

        assert_eq!(plan.skills[0].removes, vec!["claude_code"]);
        assert_eq!(
            plan.skills[0].skipped,
            vec![skipped("cursor", SkipReason::ForeignLink)]
        );

        let claude = project.root.join(".claude/skills/x");
        fs::remove_file(&claude).unwrap();
        symlink(&project.library, &claude).unwrap();
        let outcomes = project.apply(&plan);

        assert!(outcomes[0].removed.is_empty());
        assert_eq!(
            outcomes[0].skipped,
            vec![
                skipped("cursor", SkipReason::ForeignLink),
                skipped("claude_code", SkipReason::ForeignLink),
            ]
        );
        assert_eq!(fs::read_link(&claude).unwrap(), project.library);
        assert_eq!(fs::read_link(&cursor).unwrap(), project.library);
    }

    /// A skill vendored before the project went back to linking keeps the
    /// copy-mode rules in a bulk change: a new agent links to the vendored
    /// copy, not to the library, and the vendored folder is never removed.
    #[test]
    fn a_link_mode_change_links_new_agents_to_an_existing_vendored_copy() {
        let project = copy_project();
        project.deploy_copy(&["claude_code"]).unwrap();
        let agents = ["cline", "claude_code", "cursor"];

        let plan = project.plan(&agents, &agents, &HashMap::new());

        assert_eq!(plan.skills[0].adds, vec!["cursor"]);
        project.apply(&plan);
        assert_eq!(
            project.read_link(".cursor/skills/x"),
            Path::new("../../.agents/skills/x")
        );

        let plan = project.plan(&["claude_code", "cursor"], &agents, &HashMap::new());

        assert_eq!(
            plan.skills[0].skipped,
            vec![skipped("cline", SkipReason::SharedDir)]
        );
    }

    #[test]
    fn copy_planner_leaves_stale_links_of_overridden_skills_alone() {
        let project = copy_project();
        let stale = project.root.join(".claude/skills/gone");
        fs::create_dir_all(stale.parent().unwrap()).unwrap();
        symlink("../../.agents/skills/gone", &stale).unwrap();
        let overrides = HashMap::from([("Gone".to_string(), keys(&["claude_code"]))]);

        let plan = plan_copy_agent_change(
            &project.root,
            &project.configs,
            &read_project_skills(&project.root, &project.configs),
            &keys(&["cline"]),
            &keys(&["cline", "claude_code"]).into_iter().collect(),
            &overrides,
            |_| None,
        );

        assert!(plan.skills.is_empty(), "{:?}", plan.skills);
    }

    #[test]
    fn toggling_a_vendored_skill_moves_it_and_repoints_its_links() {
        let project = copy_project();
        project.deploy_copy(&["claude_code", "windsurf"]).unwrap();
        let cursor = project.real(".cursor/skills", "x");

        set_vendored_enabled(&project.root, &project.configs, "x", false).unwrap();

        assert!(!project.root.join(".agents/skills/x").exists());
        assert!(is_real_dir(&project.root.join(".agents/skills-disabled/x")));
        assert!(fs::symlink_metadata(project.root.join(".claude/skills/x")).is_err());
        assert_eq!(
            project.read_link(".claude/skills-disabled/x"),
            Path::new("../../.agents/skills-disabled/x")
        );
        assert!(project
            .root
            .join(".codeium/windsurf/skills-disabled/x/SKILL.md")
            .is_file());
        assert!(cursor.join("SKILL.md").is_file());

        set_vendored_enabled(&project.root, &project.configs, "x", true).unwrap();

        assert!(is_real_dir(&project.root.join(".agents/skills/x")));
        assert_eq!(
            project.read_link(".claude/skills/x"),
            Path::new("../../.agents/skills/x")
        );
        assert!(fs::symlink_metadata(project.root.join(".claude/skills-disabled/x")).is_err());
    }

    #[test]
    fn deleting_a_vendored_skill_removes_its_links_and_nothing_else() {
        let project = copy_project();
        project.deploy_copy(&["claude_code", "windsurf"]).unwrap();
        let cursor = project.real(".cursor/skills", "x");

        assert!(shares_vendored_copy(
            &project.root,
            &project.configs,
            "claude_code",
            "x"
        ));
        assert!(!shares_vendored_copy(
            &project.root,
            &project.configs,
            "cursor",
            "x"
        ));
        delete_vendored(&project.root, &project.configs, "x").unwrap();

        assert!(fs::symlink_metadata(project.root.join(".agents/skills/x")).is_err());
        assert!(fs::symlink_metadata(project.root.join(".claude/skills/x")).is_err());
        assert!(fs::symlink_metadata(project.root.join(".codeium/windsurf/skills/x")).is_err());
        assert!(cursor.join("SKILL.md").is_file());
        assert!(project.library.join("SKILL.md").is_file());
    }

    /// Links to either slot, from either of an agent's folders, follow a
    /// toggle and go with a delete: none is left dangling.
    #[test]
    fn toggling_and_deleting_find_links_to_either_slot() {
        let project = copy_project();
        project.real(".agents/skills-disabled", "x");
        let claude = project.root.join(".claude/skills/x");
        fs::create_dir_all(claude.parent().unwrap()).unwrap();
        symlink("../../.agents/skills-disabled/x", &claude).unwrap();

        set_vendored_enabled(&project.root, &project.configs, "x", true).unwrap();

        assert_eq!(
            fs::read_link(&claude).unwrap(),
            Path::new("../../.agents/skills/x")
        );
        assert!(claude.join("SKILL.md").is_file());

        let stale = project.root.join(".cursor/skills-disabled/x");
        fs::create_dir_all(stale.parent().unwrap()).unwrap();
        symlink("../../.agents/skills-disabled/x", &stale).unwrap();
        delete_vendored(&project.root, &project.configs, "x").unwrap();

        assert!(fs::symlink_metadata(&claude).is_err());
        assert!(fs::symlink_metadata(&stale).is_err());
    }

    fn actions(variants: &[VariantConversion]) -> Vec<(&str, ConvertAction)> {
        let mut actions: Vec<(&str, ConvertAction)> = variants
            .iter()
            .map(|variant| (variant.agent.as_str(), variant.action))
            .collect();
        actions.sort_by_key(|&(agent, action)| (agent, action as u8));
        actions
    }

    /// The link-mode layout a convert meets: absolute links to the library,
    /// a real directory, a link to something else, and copies whose enabled
    /// state differs.
    #[test]
    fn converting_vendors_each_skill_and_relinks_only_links_to_the_same_content() {
        let project = copy_project();
        project.link(".claude/skills", "x");
        project.link(".codeium/windsurf/skills", "x");
        let real = project.real(".cursor/skills", "x");
        let other = project.root.join("elsewhere/x");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("SKILL.md"), "---\nname: x\n---\nother\n").unwrap();
        let foreign = project.root.join(".codex/skills/x");
        fs::create_dir_all(foreign.parent().unwrap()).unwrap();
        symlink(&other, &foreign).unwrap();
        let mixed = project.link(".claude/skills-disabled", "x");
        project.link(".claude/skills-disabled", "y");

        let skills = read_project_skills(&project.root, &project.configs);
        let plan = plan_convert_to_copy(&project.root, &project.configs, &skills);

        let x = plan.iter().find(|c| c.relative_path == "x").unwrap();
        let library = fs::canonicalize(&project.library).unwrap();
        assert_eq!(x.copy_from.as_deref(), Some(library.to_str().unwrap()));
        assert_eq!(
            actions(&x.variants),
            vec![
                ("claude_code", ConvertAction::Relink),
                ("claude_code", ConvertAction::KeepMixedState),
                ("codex", ConvertAction::KeepForeignLink),
                ("cursor", ConvertAction::KeepRealDir),
                ("windsurf", ConvertAction::Relink),
            ]
        );

        let outcomes = apply_convert_to_copy(&project.root, &plan);

        let x_outcome = outcomes.iter().find(|o| o.relative_path == "x").unwrap();
        assert!(x_outcome.vendored && x_outcome.error.is_none());
        assert_eq!(x_outcome.kept.len(), 3);
        assert!(is_real_dir(&project.root.join(".agents/skills/x")));
        assert_eq!(
            project.read_link(".claude/skills/x"),
            Path::new("../../.agents/skills/x")
        );
        assert_eq!(
            project.read_link(".codeium/windsurf/skills/x"),
            Path::new("../../../.agents/skills/x")
        );
        assert!(real.join("SKILL.md").is_file() && !is_link(&real));
        assert_eq!(fs::read_link(&foreign).unwrap(), other);
        assert_eq!(fs::read_link(&mixed).unwrap(), project.library);
        // A skill disabled everywhere is vendored disabled.
        assert!(is_real_dir(&project.root.join(".agents/skills-disabled/y")));
        assert_eq!(
            project.read_link(".claude/skills-disabled/y"),
            Path::new("../../.agents/skills-disabled/y")
        );

        // Converting again has nothing left to vendor or relink.
        let skills = read_project_skills(&project.root, &project.configs);
        let again = plan_convert_to_copy(&project.root, &project.configs, &skills);
        assert!(again.iter().all(|c| c.copy_from.is_none()
            && c.variants.iter().all(|v| v.action != ConvertAction::Relink)));
        assert!(again.iter().all(|c| c.relative_path != "y"));
    }

    /// The vendored copy lands in the slot its folder already has, disabled
    /// here, so an enabled link is kept rather than pointed at it.
    #[test]
    fn converting_into_an_existing_disabled_slot_keeps_enabled_links() {
        let project = copy_project();
        project.link(".agents/skills-disabled", "x");
        let claude = project.link(".claude/skills", "x");

        let skills = read_project_skills(&project.root, &project.configs);
        let plan = plan_convert_to_copy(&project.root, &project.configs, &skills);

        assert_eq!(
            actions(&plan[0].variants),
            vec![("claude_code", ConvertAction::KeepMixedState)]
        );

        let outcomes = apply_convert_to_copy(&project.root, &plan);

        assert!(outcomes[0].vendored && outcomes[0].relinked.is_empty());
        assert!(is_real_dir(&project.root.join(".agents/skills-disabled/x")));
        assert_eq!(fs::read_link(&claude).unwrap(), project.library);
    }

    /// The plan is advice: a link that became a real directory since is kept.
    #[test]
    fn converting_keeps_a_link_that_became_a_real_directory() {
        let project = copy_project();
        project.link(".claude/skills", "x");
        let cursor = project.link(".cursor/skills", "x");
        let skills = read_project_skills(&project.root, &project.configs);
        let plan = plan_convert_to_copy(&project.root, &project.configs, &skills);

        fs::remove_file(&cursor).unwrap();
        project.real(".cursor/skills", "x");
        let outcomes = apply_convert_to_copy(&project.root, &plan);

        assert_eq!(outcomes[0].relinked, vec!["claude_code"]);
        assert_eq!(
            actions(&outcomes[0].kept),
            vec![("cursor", ConvertAction::KeepRealDir)]
        );
        assert!(cursor.join("SKILL.md").is_file() && !is_link(&cursor));
    }
}
