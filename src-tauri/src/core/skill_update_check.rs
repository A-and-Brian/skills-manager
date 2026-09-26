//! Deciding whether an installed skill has an update: resolving remotes off
//! the central-repo lock, then writing each skill's status under it.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use crate::core::{
    error::AppError,
    git_fetcher, installer,
    managed_skill::{managed_skill_by_id, ManagedSkillDto},
    skill_source::{git_source_from_skill, GitSkillSource},
    skill_store::{SkillRecord, SkillStore},
};

/// A distinct remote to resolve once during a batch check. Several skills can
/// share one — e.g. many skills installed from subdirectories of a single
/// monorepo — so keying by (clone_url, branch) collapses the redundant network
/// queries the per-skill loop used to make.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct RemoteKey {
    clone_url: String,
    branch: Option<String>,
}

impl From<GitSkillSource> for RemoteKey {
    fn from(source: GitSkillSource) -> Self {
        RemoteKey {
            clone_url: source.clone_url,
            branch: source.branch,
        }
    }
}

impl RemoteKey {
    /// Whether `source` still points at this remote. Subpath is deliberately
    /// ignored: two subdirectories of one repo share a head revision.
    fn matches(&self, source: &GitSkillSource) -> bool {
        self.clone_url == source.clone_url && self.branch == source.branch
    }
}

/// A remote revision resolved off the central-repo lock, tagged with the remote
/// it was resolved for. The tag is what makes it safe to apply later: a
/// reinstall keeps a skill's row and repoints its source
/// (`update_skill_after_reinstall`), so the applying side re-derives the key
/// from the freshly read record and drops a prefetch that no longer matches.
#[derive(Clone)]
pub struct PrefetchedRemote {
    pub(crate) key: RemoteKey,
    pub(crate) result: Result<String, String>,
}

/// Resolve one skill's remote revision *before* the caller takes the
/// central-repo lock. Every lock-holding update-check path goes through this:
/// holding the lock across a slow `ls-remote` is what made an unrelated
/// foreground operation fail with a 20s "repository is busy" (#315).
///
/// Returns `None` when there is nothing to resolve — a local skill, one still
/// inside its check TTL, or an unparseable source — in which case the check
/// itself does no network either.
pub fn prefetch_skill_remote(
    store: &SkillStore,
    skill_id: &str,
    force: bool,
    proxy_url: Option<&str>,
) -> Option<PrefetchedRemote> {
    let skill = store.get_skill_by_id(skill_id).ok().flatten()?;
    if !matches!(skill.source_type.as_str(), "git" | "skillssh") {
        return None;
    }
    if should_skip_update_check(store, &skill, force).unwrap_or(false) {
        return None;
    }
    let key = RemoteKey::from(git_source_from_skill(&skill).ok()?);
    let result =
        git_fetcher::resolve_remote_revision(&key.clone_url, key.branch.as_deref(), proxy_url)
            .map_err(|err| err.to_string());
    Some(PrefetchedRemote { key, result })
}

/// Upper bound on concurrent `ls-remote` queries during a batch check. Collapses
/// the wall-clock cost of a large library from "sum of every remote" to "slowest
/// single remote" without opening an unbounded number of git subprocesses.
const MAX_CHECK_CONCURRENCY: usize = 8;

/// Resolve each remote's head revision concurrently, without the central-repo
/// lock — these are read-only remote reads. A failed resolution is stored as
/// `Err(message)` so Phase B can mark just that remote's skills as errored
/// without aborting the batch.
pub(crate) fn resolve_remotes_concurrent(
    remotes: Vec<RemoteKey>,
    proxy_url: Option<String>,
) -> HashMap<RemoteKey, Result<String, String>> {
    resolve_concurrent(remotes, |key| {
        git_fetcher::resolve_remote_revision(
            &key.clone_url,
            key.branch.as_deref(),
            proxy_url.as_deref(),
        )
        .map_err(|err| err.to_string())
    })
}

/// Run `resolve` over every remote concurrently (bounded by
/// `MAX_CHECK_CONCURRENCY`) with work-stealing, and collect each result. Factored
/// out of [`resolve_remotes_concurrent`] so the concurrency contract is testable
/// with an injected resolver instead of live network: every remote is resolved
/// exactly once, a per-remote failure is stored as `Err` rather than aborting the
/// batch, and — since this function never touches `RepoLock` — resolution always
/// runs off the central-repo lock.
fn resolve_concurrent<F>(
    remotes: Vec<RemoteKey>,
    resolve: F,
) -> HashMap<RemoteKey, Result<String, String>>
where
    F: Fn(&RemoteKey) -> Result<String, String> + Sync,
{
    use std::sync::atomic::{AtomicUsize, Ordering};

    let next = AtomicUsize::new(0);
    let results: Mutex<HashMap<RemoteKey, Result<String, String>>> =
        Mutex::new(HashMap::with_capacity(remotes.len()));
    let worker_count = MAX_CHECK_CONCURRENCY.min(remotes.len().max(1));

    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            scope.spawn(|| loop {
                let idx = next.fetch_add(1, Ordering::Relaxed);
                let Some(key) = remotes.get(idx) else { break };

                let resolved = resolve(key);
                if let Ok(mut map) = results.lock() {
                    map.insert(key.clone(), resolved);
                }
            });
        }
    });

    results.into_inner().unwrap_or_default()
}

/// Check one skill end to end: resolve its remote, then write the status.
///
/// The caller must **not** hold the central-repo lock — the resolution here is
/// a network call. Paths that need the lock take it around
/// [`check_skill_update_internal_with_remote`] only, after prefetching.
pub fn check_skill_update_internal(
    store: &SkillStore,
    skill_id: &str,
    force: bool,
    proxy_url: Option<&str>,
) -> Result<ManagedSkillDto, AppError> {
    let prefetched = prefetch_skill_remote(store, skill_id, force, proxy_url);
    check_skill_update_internal_with_remote(store, skill_id, force, prefetched)
}

/// Write one skill's update status from an already-resolved remote revision.
///
/// This never touches the network — [`prefetch_skill_remote`] does that off the
/// central-repo lock, and callers hold the lock only for this write. A git
/// skill whose `prefetched` is missing or points at a remote the skill no
/// longer uses is left untouched for the next round.
pub fn check_skill_update_internal_with_remote(
    store: &SkillStore,
    skill_id: &str,
    force: bool,
    prefetched: Option<PrefetchedRemote>,
) -> Result<ManagedSkillDto, AppError> {
    let skill = store
        .get_skill_by_id(skill_id)
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found("Skill not found"))?;

    if should_skip_update_check(store, &skill, force)? {
        return managed_skill_by_id(store, skill_id);
    }

    match skill.source_type.as_str() {
        "git" | "skillssh" => {
            let git_source = git_source_from_skill(&skill)?;
            let metadata_updated = skill.source_ref_resolved.as_deref()
                != Some(git_source.clone_url.as_str())
                || skill.source_subpath.as_deref() != git_source.subpath.as_deref()
                || skill.source_branch.as_deref() != git_source.branch.as_deref();
            if metadata_updated {
                store
                    .update_skill_source_metadata(
                        &skill.id,
                        Some(&git_source.clone_url),
                        git_source.subpath.as_deref(),
                        git_source.branch.as_deref(),
                        skill.source_revision.as_deref(),
                    )
                    .map_err(AppError::db)?;
            }

            // Apply the revision resolved off the lock — but only if the skill
            // still points at the remote it was resolved for. A reinstall keeps
            // the row and repoints its source, so a stale prefetch would record
            // a status computed against the wrong remote.
            //
            // When nothing usable was prefetched, skip the skill instead of
            // resolving here: every caller of this function holds the
            // central-repo lock, and a network call under that lock is the
            // 20s "busy" failure the off-lock split exists to remove (#315).
            // The next round picks the skill up.
            let Some(remote_result) = prefetched
                .filter(|prefetched| prefetched.key.matches(&git_source))
                .map(|prefetched| prefetched.result)
            else {
                log::debug!(
                    "check update: no usable prefetched remote for {}, skipping this round",
                    skill.id
                );
                return managed_skill_by_id(store, skill_id);
            };
            match remote_result {
                Ok(remote_revision) => {
                    let update_status = match skill.source_revision.as_deref() {
                        Some(current) if current == remote_revision => "up_to_date",
                        Some(_) => "update_available",
                        None => "unknown",
                    };
                    store
                        .update_skill_check_state(
                            &skill.id,
                            Some(&remote_revision),
                            update_status,
                            None,
                        )
                        .map_err(AppError::db)?;
                }
                Err(message) => {
                    store
                        .update_skill_check_state(
                            &skill.id,
                            skill.remote_revision.as_deref(),
                            "error",
                            Some(&message),
                        )
                        .map_err(AppError::db)?;
                    return Err(AppError::git(message));
                }
            }
        }
        "local" | "import" => {
            let (status, error): (&str, Option<String>) = match skill.source_ref.as_deref() {
                Some(path) => {
                    let source_path = Path::new(path);
                    if !source_path.exists() {
                        (
                            "source_missing",
                            Some("Original source path no longer exists".to_string()),
                        )
                    } else {
                        match installer::hash_local_source(source_path) {
                            Ok(live_hash) => local_source_status(&skill, source_path, &live_hash),
                            Err(err) => ("error", Some(err.to_string())),
                        }
                    }
                }
                None => ("local_only", None),
            };
            store
                .update_skill_check_state(&skill.id, None, status, error.as_deref())
                .map_err(AppError::db)?;
        }
        _ => {
            store
                .update_skill_check_state(&skill.id, None, "unknown", None)
                .map_err(AppError::db)?;
        }
    }

    managed_skill_by_id(store, skill_id)
}

/// Classify a `local`/`import` skill against its freshly hashed source.
fn local_source_status(
    skill: &SkillRecord,
    source: &Path,
    live_hash: &str,
) -> (&'static str, Option<String>) {
    match skill.content_hash.as_deref() {
        None => ("local_only", None),
        Some(stored) if stored == live_hash => ("up_to_date", None),
        // The byte hashes disagree. Before offering an update, rule out the one
        // difference that is not one — see [`differs_only_by_line_endings`].
        Some(_) if differs_only_by_line_endings(source, Path::new(&skill.central_path)) => {
            ("up_to_date", None)
        }
        Some(_) => ("update_available", None),
    }
}

/// True when the original source and the library copy hold the same content in
/// two line-ending encodings, and nothing else.
///
/// A `local`/`import` skill is checked by hashing the user's own source path,
/// which is theirs to keep however they like — commonly a git working tree.
/// Git for Windows defaults to `core.autocrlf=true`, so on a Windows + macOS
/// pair the same checkout is CRLF on one machine and LF on the other while the
/// library copy (our own byte copy, or a copy synced from the other machine)
/// keeps the other encoding. Byte hashes then disagree forever and the skill
/// sits at "update available"; re-importing rewrites the library in the local
/// encoding, the other machine sees *its* copy drift, and the two devices push
/// the same skill back and forth. Nothing changed, so nothing should be offered.
///
/// Deliberately compares the two live trees rather than the stored hash: the
/// stored hash answers "what did we install?", and the question here is "do
/// these two directories differ right now?". Any failure to read either side
/// answers `false`, leaving the byte-hash verdict standing — this may only ever
/// suppress a false update, never assert sameness it could not establish.
fn differs_only_by_line_endings(source: &Path, central: &Path) -> bool {
    let (Ok(source_hash), Ok(central_hash)) = (
        installer::hash_local_source_eol_insensitive(source),
        crate::core::content_hash::hash_directory_eol_insensitive(central),
    ) else {
        return false;
    };
    source_hash == central_hash
}

pub(crate) fn should_skip_update_check(
    store: &SkillStore,
    skill: &SkillRecord,
    force: bool,
) -> Result<bool, AppError> {
    if force {
        return Ok(false);
    }

    let ttl_minutes = store
        .get_setting("update_check_ttl_minutes")
        .map_err(AppError::db)?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(60);
    let ttl_ms = ttl_minutes * 60 * 1000;
    let stable_status = !matches!(
        skill.update_status.as_str(),
        "unknown" | "checking" | "updating" | "error"
    );

    Ok(stable_status
        && skill
            .last_checked_at
            .map(|checked| chrono::Utc::now().timestamp_millis() - checked < ttl_ms)
            .unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::central_repo;
    use crate::core::test_support::{sample_skill, test_repo, write_skill_dir, TestRepo};
    use std::fs;

    // ── RemoteKey dedup (batch check_all fan-out) ──

    fn source(clone_url: &str, branch: Option<&str>, subpath: Option<&str>) -> GitSkillSource {
        GitSkillSource {
            clone_url: clone_url.to_string(),
            branch: branch.map(str::to_string),
            subpath: subpath.map(str::to_string),
            locator_skill_id: None,
        }
    }

    /// The whole point of keying Phase A by `RemoteKey`: skills installed from
    /// different subdirectories of the same monorepo (same clone_url + branch)
    /// must collapse to one network query, while a different branch stays
    /// distinct. This is what turns 4 `mattpocock/skills` skills into 1
    /// `ls-remote` instead of 4.
    #[test]
    fn remote_key_dedups_by_url_and_branch_ignoring_subpath() {
        let mut per_remote: HashMap<RemoteKey, usize> = HashMap::new();
        let skills = [
            source("https://github.com/mattpocock/skills.git", None, Some("a")),
            source("https://github.com/mattpocock/skills.git", None, Some("b")),
            source("https://github.com/mattpocock/skills.git", None, None),
            source("https://github.com/vercel/ai.git", None, None),
            // Same repo, different branch → must NOT collapse with the None-branch group.
            source(
                "https://github.com/mattpocock/skills.git",
                Some("next"),
                None,
            ),
        ];
        for s in skills {
            *per_remote.entry(RemoteKey::from(s)).or_insert(0) += 1;
        }

        assert_eq!(per_remote.len(), 3, "distinct remotes to query");
        assert_eq!(
            per_remote[&RemoteKey {
                clone_url: "https://github.com/mattpocock/skills.git".to_string(),
                branch: None,
            }],
            3,
            "three subpaths of one repo/branch share a single query"
        );
        assert_eq!(
            per_remote[&RemoteKey {
                clone_url: "https://github.com/mattpocock/skills.git".to_string(),
                branch: Some("next".to_string()),
            }],
            1,
            "a different branch is a separate remote"
        );
    }

    fn remote(url: &str, branch: Option<&str>) -> RemoteKey {
        RemoteKey {
            clone_url: url.to_string(),
            branch: branch.map(|b| b.to_string()),
        }
    }

    /// Work-stealing must cover every remote exactly once and collect each
    /// resolver result under its own key — this exercises the real concurrent
    /// loop, not just `RemoteKey`'s hashing.
    #[test]
    fn resolve_concurrent_resolves_every_remote_exactly_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let remotes: Vec<RemoteKey> = (0..20)
            .map(|i| remote(&format!("https://example.test/r{i}"), None))
            .collect();
        let calls = AtomicUsize::new(0);

        let out = resolve_concurrent(remotes.clone(), |key| {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(format!("rev:{}", key.clone_url))
        });

        assert_eq!(
            calls.load(Ordering::Relaxed),
            remotes.len(),
            "each remote resolved exactly once"
        );
        assert_eq!(out.len(), remotes.len());
        for key in &remotes {
            assert!(matches!(out.get(key), Some(Ok(v)) if *v == format!("rev:{}", key.clone_url)));
        }
    }

    /// A single remote failing must be stored as `Err` for that key alone and
    /// never abort the batch (the "检查全部 both crawled and popped failures" fix
    /// depends on this isolation).
    #[test]
    fn resolve_concurrent_isolates_per_remote_failures() {
        let ok = remote("https://example.test/ok", None);
        let bad = remote("https://example.test/bad", Some("main"));

        let out = resolve_concurrent(vec![ok.clone(), bad.clone()], |key| {
            if key.clone_url.ends_with("/bad") {
                Err("boom".to_string())
            } else {
                Ok("rev".to_string())
            }
        });

        assert!(matches!(out.get(&ok), Some(Ok(v)) if v == "rev"));
        assert!(matches!(out.get(&bad), Some(Err(e)) if e == "boom"));
    }

    /// The resolutions must genuinely overlap: with several remotes and a
    /// resolver that lingers, more than one worker is inside `resolve` at once.
    /// Because `resolve_concurrent` holds no `RepoLock`, this is also the proof
    /// that the network step runs off the central-repo lock.
    #[test]
    fn resolve_concurrent_runs_remotes_in_parallel() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let remotes: Vec<RemoteKey> = (0..8).map(|i| remote(&format!("r{i}"), None)).collect();
        let in_flight = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);

        let out = resolve_concurrent(remotes, |_key| {
            let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(20));
            in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok("rev".to_string())
        });

        assert_eq!(out.len(), 8);
        assert!(
            peak.load(Ordering::SeqCst) >= 2,
            "expected concurrent resolution, peak in-flight was {}",
            peak.load(Ordering::SeqCst)
        );
    }

    // ── Applying a prefetched remote under the lock ──

    /// A git-backed skill pinned at `old-rev` on `remote_url`.
    fn insert_git_skill(store: &SkillStore, id: &str, remote_url: &str) {
        let dir = write_skill_dir(id);
        let mut skill = sample_skill(id, id, &dir);
        skill.source_type = "git".to_string();
        skill.source_ref = Some(remote_url.to_string());
        skill.source_ref_resolved = Some(remote_url.to_string());
        skill.source_revision = Some("old-rev".to_string());
        skill.update_status = "unknown".to_string();
        store.insert_skill(&skill).unwrap();
    }

    fn prefetch(url: &str, revision: &str) -> Option<PrefetchedRemote> {
        Some(PrefetchedRemote {
            key: remote(url, None),
            result: Ok(revision.to_string()),
        })
    }

    /// The happy path: a prefetch resolved for the skill's own remote is applied.
    #[test]
    fn matching_prefetched_remote_is_applied() {
        let repo = test_repo();
        insert_git_skill(&repo.store, "skill-1", "https://example.test/a.git");

        let dto = check_skill_update_internal_with_remote(
            &repo.store,
            "skill-1",
            false,
            prefetch("https://example.test/a.git", "new-rev"),
        )
        .unwrap();

        assert_eq!(dto.update_status, "update_available");
        let stored = repo.store.get_skill_by_id("skill-1").unwrap().unwrap();
        assert_eq!(stored.remote_revision.as_deref(), Some("new-rev"));
    }

    /// A reinstall between the off-lock resolve and this write keeps the skill's
    /// row but repoints its source. The revision resolved for the *old* remote
    /// must not be recorded against the new one — it would show a fabricated
    /// "up to date"/"update available" for a source it was never read from.
    #[test]
    fn prefetched_remote_for_a_different_source_is_discarded() {
        let repo = test_repo();
        insert_git_skill(&repo.store, "skill-1", "https://example.test/new.git");

        let dto = check_skill_update_internal_with_remote(
            &repo.store,
            "skill-1",
            false,
            prefetch("https://example.test/old.git", "rev-of-old-remote"),
        )
        .unwrap();

        assert_eq!(
            dto.update_status, "unknown",
            "status left for the next round"
        );
        let stored = repo.store.get_skill_by_id("skill-1").unwrap().unwrap();
        assert_eq!(
            stored.remote_revision, None,
            "no revision from a stale remote"
        );
        assert_eq!(stored.last_checked_at, None, "the check did not complete");
    }

    /// Insert a `local` skill whose library copy is `central_body` and whose
    /// original source path holds `source_body`, with the stored hash recorded
    /// from the library copy exactly as an install would leave it.
    fn insert_local_skill(repo: &TestRepo, id: &str, central_body: &str, source_body: &str) {
        let central = central_repo::skills_dir().join(id);
        fs::create_dir_all(&central).unwrap();
        fs::write(central.join("SKILL.md"), central_body).unwrap();

        let source = repo._tmp.path().join(format!("{id}-source"));
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("SKILL.md"), source_body).unwrap();

        let mut skill = sample_skill(id, id, &central);
        skill.source_type = "local".to_string();
        skill.source_ref = Some(source.to_string_lossy().to_string());
        skill.content_hash = Some(crate::core::content_hash::hash_directory(&central).unwrap());
        skill.update_status = "unknown".to_string();
        repo.store.insert_skill(&skill).unwrap();
    }

    /// Drives the real check, because the wiring is where this can go wrong:
    /// the tiebreaker can be correct and still never be consulted. A Windows
    /// checkout of the same skill is CRLF while the library copy synced from a
    /// Mac is LF — byte hashes disagree, but there is no update to offer, and
    /// offering one starts a re-import ping-pong between the two machines.
    #[test]
    fn a_local_source_that_differs_only_in_line_endings_is_up_to_date() {
        let repo = test_repo();
        insert_local_skill(
            &repo,
            "skill-1",
            "---\nname: skill-1\n---\nbody\n",
            "---\r\nname: skill-1\r\n---\r\nbody\r\n",
        );

        let dto =
            check_skill_update_internal_with_remote(&repo.store, "skill-1", true, None).unwrap();

        assert_eq!(dto.update_status, "up_to_date");
    }

    /// A vanished library copy still has an update to offer. This is a
    /// regression guard on the end-to-end path, not proof of the empty-tree
    /// guard itself — a non-empty source cannot collide with an empty library,
    /// so what pins that collision is
    /// `content_hash::tests::an_empty_or_missing_directory_has_no_tiebreaker_hash`.
    #[test]
    fn a_missing_library_copy_is_not_up_to_date() {
        let repo = test_repo();
        insert_local_skill(
            &repo,
            "skill-1",
            "---\nname: skill-1\n---\nbody\n",
            "---\r\nname: skill-1\r\n---\r\nbody\r\n",
        );
        fs::remove_dir_all(central_repo::skills_dir().join("skill-1")).unwrap();

        let dto =
            check_skill_update_internal_with_remote(&repo.store, "skill-1", true, None).unwrap();

        assert_eq!(dto.update_status, "update_available");
    }

    /// The other half of the same wiring: the tiebreaker must not swallow a
    /// real edit. Without this, "always up to date" would pass the test above.
    #[test]
    fn a_local_source_with_a_real_edit_still_reports_an_update() {
        let repo = test_repo();
        insert_local_skill(
            &repo,
            "skill-1",
            "---\nname: skill-1\n---\nbody\n",
            "---\r\nname: skill-1\r\n---\r\nbody, rewritten\r\n",
        );

        let dto =
            check_skill_update_internal_with_remote(&repo.store, "skill-1", true, None).unwrap();

        assert_eq!(dto.update_status, "update_available");
    }

    /// A remote that failed to resolve off the lock still has to land as an
    /// `error` status here, not be swallowed as "nothing to apply" — the batch
    /// check counts that error and the card shows the reason.
    #[test]
    fn failed_prefetch_for_the_current_source_records_the_error() {
        let repo = test_repo();
        insert_git_skill(&repo.store, "skill-1", "https://example.test/a.git");

        let err = check_skill_update_internal_with_remote(
            &repo.store,
            "skill-1",
            false,
            Some(PrefetchedRemote {
                key: remote("https://example.test/a.git", None),
                result: Err("could not read from remote".to_string()),
            }),
        )
        .unwrap_err();

        assert!(err.message.contains("could not read from remote"));
        let stored = repo.store.get_skill_by_id("skill-1").unwrap().unwrap();
        assert_eq!(stored.update_status, "error");
        assert_eq!(
            stored.last_check_error.as_deref(),
            Some("could not read from remote")
        );
    }

    /// Callers hold the central-repo lock across this write, so a git skill with
    /// nothing prefetched must be skipped rather than resolved inline — that
    /// inline call is the lock-held network round-trip behind the 20s "busy"
    /// failures (#315).
    #[test]
    fn missing_prefetch_never_resolves_under_the_lock() {
        let repo = test_repo();
        insert_git_skill(&repo.store, "skill-1", "https://example.test/a.git");

        let dto =
            check_skill_update_internal_with_remote(&repo.store, "skill-1", false, None).unwrap();

        assert_eq!(dto.update_status, "unknown");
        let stored = repo.store.get_skill_by_id("skill-1").unwrap().unwrap();
        assert_eq!(stored.last_checked_at, None, "no network, no write");
    }
}
