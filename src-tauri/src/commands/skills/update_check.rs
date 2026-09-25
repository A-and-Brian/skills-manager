//! Checking skills for available updates.

use std::collections::{HashMap, HashSet};
use tauri::State;

use crate::core::{
    error::AppError,
    host::HostCtx,
    managed_skill::ManagedSkillDto,
    repo_lock::RepoLock,
    skill_source::git_source_from_skill,
    skill_update_check::{
        check_skill_update_internal_with_remote, prefetch_skill_remote, resolve_remotes_concurrent,
        should_skip_update_check, PrefetchedRemote, RemoteKey,
    },
};

#[tauri::command]
pub async fn check_skill_update(
    skill_id: String,
    force: Option<bool>,
    ctx: State<'_, HostCtx>,
) -> Result<ManagedSkillDto, AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || check_skill_update_core(&ctx, skill_id, force))
        .await?
}

pub fn check_skill_update_core(
    ctx: &HostCtx,
    skill_id: String,
    force: Option<bool>,
) -> Result<ManagedSkillDto, AppError> {
    let store = ctx.store.clone();
    let proxy_url = store.proxy_url();
    let force = force.unwrap_or(false);
    // Resolve first, take the lock second. Holding it across `ls-remote`
    // meant one check of a slow remote could occupy the repository for the
    // whole round-trip and fail every concurrent operation (#315).
    let prefetched = prefetch_skill_remote(&store, &skill_id, force, proxy_url.as_deref());
    let _lock = RepoLock::acquire_foreground("check skill update").map_err(AppError::db)?;
    check_skill_update_internal_with_remote(&store, &skill_id, force, prefetched)
}

#[tauri::command]
pub async fn check_all_skill_updates(
    force: Option<bool>,
    ctx: State<'_, HostCtx>,
) -> Result<(), AppError> {
    let ctx = ctx.inner().clone();
    tauri::async_runtime::spawn_blocking(move || check_all_skill_updates_core(&ctx, force)).await?
}

pub fn check_all_skill_updates_core(ctx: &HostCtx, force: Option<bool>) -> Result<(), AppError> {
    let store = ctx.store.clone();
    let proxy_url = store.proxy_url();
    let force_check = force.unwrap_or(false);
    let skills = store.get_all_skills().map_err(AppError::db)?;

    // ── Phase A: resolve every distinct remote once, concurrently ──
    // Collect the git-backed skills that still need a network check keyed by
    // (clone_url, branch). Skills installed from subdirectories of the same
    // monorepo collapse to a single `ls-remote`, and each remote is queried
    // off the central-repo lock so a slow remote (e.g. vercel/ai's ref
    // advertisement runs ~30s) never starves a concurrent check into a 20s
    // lock-timeout "busy" failure — the reason "检查全部" both crawled and
    // popped failures.
    let mut remotes: HashSet<RemoteKey> = HashSet::new();
    for skill in &skills {
        if !matches!(skill.source_type.as_str(), "git" | "skillssh") {
            continue;
        }
        match should_skip_update_check(&store, skill, force_check) {
            Ok(true) => continue,
            Ok(false) => {}
            // A transient skip-decision error (e.g. a settings read) must not
            // abort the whole batch: fall through so Phase B still checks this
            // skill and collects any real failure per-skill, as before.
            Err(err) => log::warn!(
                "check all: skip-decision for {} failed, checking anyway: {}",
                skill.id,
                err.message
            ),
        }
        if let Ok(source) = git_source_from_skill(skill) {
            remotes.insert(RemoteKey::from(source));
        }
    }
    let remote_revisions = if remotes.is_empty() {
        HashMap::new()
    } else {
        resolve_remotes_concurrent(remotes.into_iter().collect(), proxy_url.clone())
    };

    // ── Phase B: apply the resolved revisions + local-source checks ──
    // Phase A already did every network read, so this loop only computes and
    // writes each skill's status columns. Re-take the central-repo lock per
    // skill around that write — the same guard the pre-concurrent code used so
    // a concurrent manual install/update can't race the `update_status` write
    // — but now the lock is never held across a slow `ls-remote`, because the
    // network happened off the lock in Phase A, and the apply step itself
    // can't reach the network. A skill whose source moved (or whose TTL
    // expired) between the two phases has no usable prefetch and is simply
    // left for the next round. Lock contention is still reported per skill
    // so the caller knows the check didn't complete for it.
    let mut failed = Vec::new();
    for skill in &skills {
        let prefetched = if matches!(skill.source_type.as_str(), "git" | "skillssh") {
            git_source_from_skill(skill).ok().and_then(|source| {
                let key = RemoteKey::from(source);
                remote_revisions
                    .get(&key)
                    .cloned()
                    .map(|result| PrefetchedRemote { key, result })
            })
        } else {
            None
        };
        let _lock = match RepoLock::acquire("check skill update") {
            Ok(lock) => lock,
            Err(err) => {
                failed.push(format!("{}: {}", skill.id, err));
                continue;
            }
        };
        if let Err(err) =
            check_skill_update_internal_with_remote(&store, &skill.id, force_check, prefetched)
        {
            // Surface the real per-skill reason so a batch that "just fails"
            // is diagnosable from the logs, not only the aggregated toast.
            log::warn!("check all: {} failed: {}", skill.id, err.message);
            failed.push(format!("{}: {}", skill.id, err));
        }
    }

    if failed.is_empty() {
        Ok(())
    } else {
        Err(AppError::internal(format!(
            "Failed to check {} skill(s): {}",
            failed.len(),
            failed.join("; ")
        )))
    }
}
