import type { GitBackupStatus } from "./tauri";

export type GitBackupMode =
  | "loading"
  | "uninitialized"
  | "needs_remote"
  | "needs_fix"
  | "up_to_date"
  | "pending_changes";

/**
 * The state the backup UI shows for a git status. `remoteConfig` is the saved
 * remote URL, which counts as a remote even before git reports one.
 */
export function gitBackupMode(status: GitBackupStatus | null, remoteConfig: string): GitBackupMode {
  if (!status) return "loading";
  if (!status.is_repo) return "uninitialized";
  if (!status.remote_url && !remoteConfig) return "needs_remote";
  if (
    status.upstream_health === "unrelated_histories"
    || status.upstream_health === "detached"
  ) {
    return "needs_fix";
  }
  // First-push case: remote is set but upstream tracking is not yet established.
  // Treat as a normal pending sync — the push path will set upstream automatically.
  if (status.upstream_health === "no_upstream") return "pending_changes";
  if (status.has_changes || status.ahead > 0 || status.behind > 0) return "pending_changes";
  return "up_to_date";
}

/**
 * How many changes wait on each side. Uncommitted changes count as at least
 * one local change even before they are committed.
 */
export function pendingBreakdown(status: GitBackupStatus | null): { local: number; remote: number } {
  return {
    local: Math.max(status?.ahead ?? 0, status?.has_changes ? 1 : 0),
    remote: status?.behind ?? 0,
  };
}
