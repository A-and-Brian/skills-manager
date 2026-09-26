import { AlertTriangle, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import * as api from "../lib/tauri";
import type { GitBackupStatus, ManagedSkill } from "../lib/tauri";

interface BackupConflictsProps {
  conflicts: api.PendingConflict[];
  gitStatus: GitBackupStatus | null;
  /** This computer's skills, for display names; null while a host is active. */
  localSkills: ManagedSkill[] | null;
  /** Skill id of the conflict being resolved, if any. */
  resolvingConflict: string | null;
  /** The view's in-flight action, if any; disables the actions. */
  loading: string | null;
  onResolve: (skillId: string, action: api.ResolveConflictAction) => void;
}

/** Sync conflicts that need the user to pick a side (merge-engine design §4). */
export function BackupConflicts({
  conflicts,
  gitStatus,
  localSkills,
  resolvingConflict,
  loading,
  onResolve,
}: BackupConflictsProps) {
  const { t } = useTranslation();

  const conflictDisplayName = (conflict: api.PendingConflict) => {
    const managed = localSkills?.find((skill) => skill.id === conflict.skill_id);
    if (managed?.name) return managed.name;
    const fromPath = conflict.theirs_path?.split("/").pop();
    return fromPath || conflict.skill_id.slice(0, 8);
  };

  return (
    <section className="app-panel border-amber-500/40 bg-amber-500/5 p-4">
      <div className="mb-1 flex items-center gap-2">
        <AlertTriangle className="h-4 w-4 text-amber-600 dark:text-amber-300" />
        <h2 className="text-[14px] font-semibold text-secondary">
          {t("backup.conflicts.title")}
        </h2>
        <span className="rounded-full bg-amber-500/15 px-2 py-0.5 text-[11px] font-medium text-amber-700 dark:text-amber-300">
          {conflicts.length}
        </span>
      </div>
      <p className="mb-3 text-[13px] leading-5 text-muted">
        {t("backup.conflicts.desc")}
        {(gitStatus?.behind ?? 0) > 0 && (
          <>
            {" "}
            {t("backup.conflicts.autoPaused")}
          </>
        )}
      </p>
      <ul className="space-y-2">
        {conflicts.map((conflict) => {
          const busy = resolvingConflict === conflict.skill_id;
          return (
            <li
              key={conflict.skill_id}
              className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-border-subtle bg-bg-secondary px-3 py-2"
            >
              <div className="min-w-0">
                <div className="truncate text-[13px] font-medium text-primary">
                  {conflictDisplayName(conflict)}
                </div>
                <div className="text-[12px] text-muted">
                  {t("backup.conflicts.itemDesc")}
                </div>
              </div>
              <div className="flex shrink-0 flex-wrap items-center gap-1.5">
                {busy ? (
                  <Loader2 className="h-4 w-4 animate-spin text-muted" />
                ) : (
                  <>
                    <button
                      type="button"
                      onClick={() => onResolve(conflict.skill_id, "keep_local")}
                      disabled={!!resolvingConflict || !!loading}
                      className="rounded-lg border border-border-subtle px-2.5 py-1 text-[12px] font-medium text-secondary transition-colors hover:bg-surface-hover disabled:opacity-50"
                    >
                      {t("backup.conflicts.keepLocal")}
                    </button>
                    <button
                      type="button"
                      onClick={() => onResolve(conflict.skill_id, "use_remote")}
                      disabled={!!resolvingConflict || !!loading}
                      className="rounded-lg border border-border-subtle px-2.5 py-1 text-[12px] font-medium text-secondary transition-colors hover:bg-surface-hover disabled:opacity-50"
                    >
                      {t("backup.conflicts.useRemote")}
                    </button>
                    <button
                      type="button"
                      onClick={() => onResolve(conflict.skill_id, "keep_both")}
                      disabled={!!resolvingConflict || !!loading}
                      className="rounded-lg border border-border-subtle px-2.5 py-1 text-[12px] font-medium text-secondary transition-colors hover:bg-surface-hover disabled:opacity-50"
                    >
                      {t("backup.conflicts.keepBoth")}
                    </button>
                  </>
                )}
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
