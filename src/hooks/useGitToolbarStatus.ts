import { useCallback, useEffect, useState } from "react";
import * as api from "../lib/tauri";
import type { GitBackupStatus, ManagedSkill } from "../lib/tauri";

/**
 * Backup status for the library toolbar. Loaded once, refetched (with `git
 * fetch`) on window focus or visibility, and refreshed locally after skill
 * changes. Backup is this computer's, so nothing runs while `onRemote`.
 */
export function useGitToolbarStatus(onRemote: boolean, skills: ManagedSkill[]) {
  const [gitStatus, setGitStatus] = useState<GitBackupStatus | null>(null);
  const [gitRemoteConfig, setGitRemoteConfig] = useState("");

  const refreshGitStatus = useCallback(async () => {
    try {
      await api.gitBackupFetch().catch(() => {});
      const status = await api.gitBackupStatus();
      setGitStatus(status);
    } catch {
      // not critical
    }
  }, []);

  // Local-only status refresh: no `git fetch`, so it can fire from
  // dependency-driven effects without driving the file-watcher → refresh
  // → fetch feedback loop.
  const refreshGitStatusLocal = useCallback(async () => {
    try {
      const status = await api.gitBackupStatus();
      setGitStatus(status);
    } catch {
      // not critical
    }
  }, []);

  useEffect(() => {
    if (onRemote) return;
    (async () => {
      const savedRemote = (await api.getSettings("git_backup_remote_url").catch(() => null))?.trim() || "";
      const status = await api.gitBackupStatus().catch(() => null);
      setGitStatus(status);
      // The saved setting is the single source of truth. Do not backfill from
      // `.git/config` — that made a cleared URL reappear after disconnect (#260).
      setGitRemoteConfig(savedRemote);
    })();
  }, [onRemote]);

  useEffect(() => {
    if (onRemote) return;
    const handleWindowFocus = () => {
      refreshGitStatus();
    };
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        refreshGitStatus();
      }
    };

    window.addEventListener("focus", handleWindowFocus);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      window.removeEventListener("focus", handleWindowFocus);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [refreshGitStatus, onRemote]);

  useEffect(() => {
    if (onRemote) return;
    const timer = window.setTimeout(() => {
      refreshGitStatusLocal();
    }, 400);
    return () => window.clearTimeout(timer);
  }, [skills, refreshGitStatusLocal, onRemote]);

  return { gitStatus, gitRemoteConfig };
}
