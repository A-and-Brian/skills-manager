import { describe, expect, it } from "vitest";
import type { GitBackupStatus } from "./tauri";
import { gitBackupMode, pendingBreakdown } from "./gitBackupMode";

function status(overrides: Partial<GitBackupStatus> = {}): GitBackupStatus {
  return {
    is_repo: true,
    remote_url: "git@github.com:me/backup.git",
    branch: "main",
    has_changes: false,
    changed_skill_count: 0,
    ahead: 0,
    behind: 0,
    last_commit: null,
    last_commit_time: null,
    current_snapshot_tag: null,
    restored_from_tag: null,
    upstream_health: "healthy",
    ...overrides,
  };
}

describe("gitBackupMode", () => {
  it("is loading until a status arrives", () => {
    expect(gitBackupMode(null, "")).toBe("loading");
  });

  it("is uninitialized when the folder is not a repo", () => {
    expect(gitBackupMode(status({ is_repo: false }), "")).toBe("uninitialized");
  });

  it("needs a remote when neither git nor the saved config has one", () => {
    expect(gitBackupMode(status({ remote_url: null }), "")).toBe("needs_remote");
  });

  it("accepts the saved remote config in place of git's remote", () => {
    expect(gitBackupMode(status({ remote_url: null }), "git@example.com:me/b.git")).toBe("up_to_date");
  });

  it("needs a fix for unrelated or detached histories", () => {
    expect(gitBackupMode(status({ upstream_health: "unrelated_histories" }), "")).toBe("needs_fix");
    expect(gitBackupMode(status({ upstream_health: "detached" }), "")).toBe("needs_fix");
  });

  it("treats a missing upstream as pending", () => {
    expect(gitBackupMode(status({ upstream_health: "no_upstream" }), "")).toBe("pending_changes");
  });

  it("is pending when ahead, behind or with local changes", () => {
    expect(gitBackupMode(status({ ahead: 1 }), "")).toBe("pending_changes");
    expect(gitBackupMode(status({ behind: 2 }), "")).toBe("pending_changes");
    expect(gitBackupMode(status({ has_changes: true }), "")).toBe("pending_changes");
  });

  it("is up to date otherwise", () => {
    expect(gitBackupMode(status(), "")).toBe("up_to_date");
  });
});

describe("pendingBreakdown", () => {
  it("counts commits on each side", () => {
    expect(pendingBreakdown(status({ ahead: 3, behind: 2 }))).toEqual({ local: 3, remote: 2 });
  });

  it("counts uncommitted changes as at least one local change", () => {
    expect(pendingBreakdown(status({ has_changes: true }))).toEqual({ local: 1, remote: 0 });
    expect(pendingBreakdown(status({ has_changes: true, ahead: 4 }))).toEqual({ local: 4, remote: 0 });
  });

  it("is zero without a status", () => {
    expect(pendingBreakdown(null)).toEqual({ local: 0, remote: 0 });
  });
});
