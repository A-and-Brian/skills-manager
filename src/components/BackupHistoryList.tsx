import { History, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../utils";
import { displaySnapshotLabel } from "../lib/backupFormat";
import type { GitBackupStatus, GitBackupVersion } from "../lib/tauri";

function formatDateTime(iso: string) {
  if (!iso) return "-";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString();
}

interface BackupHistoryListProps {
  versions: GitBackupVersion[];
  versionsLoading: boolean;
  gitStatus: GitBackupStatus | null;
  /** Tag of the snapshot being restored, if any. */
  restoringVersionTag: string | null;
  onRefresh: () => void;
  /** Ask to restore a snapshot; the view confirms first. */
  onRestore: (tag: string) => void;
}

/** The backup snapshots, newest first, each restorable. */
export function BackupHistoryList({
  versions,
  versionsLoading,
  gitStatus,
  restoringVersionTag,
  onRefresh,
  onRestore,
}: BackupHistoryListProps) {
  const { t } = useTranslation();

  return (
    <section className="app-panel p-4">
      <div className="mb-3 flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <History className="h-4 w-4 text-muted" />
          <h2 className="text-[14px] font-semibold text-secondary">{t("backup.history.title")}</h2>
        </div>
        <button
          type="button"
          onClick={onRefresh}
          disabled={versionsLoading || !gitStatus?.is_repo}
          className="inline-flex h-7 items-center gap-1.5 rounded-lg px-2 text-[13px] text-muted transition-colors hover:bg-surface-hover hover:text-secondary disabled:opacity-50"
        >
          <RefreshCw className={cn("h-3 w-3", versionsLoading && "animate-spin")} />
          {t("settings.refresh")}
        </button>
      </div>

      {versionsLoading ? (
        <div className="py-6 text-center text-[13px] text-muted">{t("mySkills.gitVersionLoading")}</div>
      ) : versions.length === 0 ? (
        <div className="rounded-md border border-dashed border-border-subtle py-6 text-center text-[13px] text-muted">
          {t("backup.history.empty")}
        </div>
      ) : (
        <div className="max-h-[360px] space-y-1.5 overflow-auto pr-1">
          {versions.map((version) => (
            <div
              key={version.tag}
              className="flex items-center justify-between gap-3 rounded-md border border-border-subtle bg-bg-secondary px-3 py-2"
            >
              <div className="min-w-0">
                <div className="truncate text-[13px] font-semibold text-secondary">
                  {displaySnapshotLabel(version.tag)}
                </div>
                <div className="truncate text-[12px] text-muted">{version.message || version.commit}</div>
                <div className="text-[11px] text-faint">
                  {version.author ? `${version.author} · ` : ""}
                  {version.commit} · {formatDateTime(version.committed_at)}
                </div>
              </div>
              <button
                type="button"
                onClick={() => onRestore(version.tag)}
                disabled={!!restoringVersionTag}
                className="shrink-0 rounded-lg border border-border-subtle px-2 py-1 text-[12px] font-medium text-secondary transition-colors hover:bg-surface-hover disabled:opacity-50"
              >
                {restoringVersionTag === version.tag
                  ? t("mySkills.gitVersionRestoring")
                  : t("mySkills.gitVersionRestore")}
              </button>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
