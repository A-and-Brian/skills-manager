import { useMemo, useState } from "react";
import {
  AlertTriangle,
  Check,
  CheckCircle2,
  Cloud,
  Github,
  Loader2,
  Pencil,
  RefreshCw,
  Upload,
  Wrench,
  XCircle,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { cn } from "../utils";
import { formatSnapshotWhen } from "../lib/backupFormat";
import { gitBackupMode, pendingBreakdown } from "../lib/gitBackupMode";
import * as api from "../lib/tauri";
import type { GitBackupStatus, GitUpstreamHealth } from "../lib/tauri";

interface BackupStatusCardProps {
  gitStatus: GitBackupStatus | null;
  /** Saved remote URL; may be set before the repo exists. */
  remoteConfig: string;
  /** Plain-language reason the last backup failed, if it did. */
  backupError: string | null;
  deviceName: string;
  setDeviceName: (name: string) => void;
  /** The view's in-flight action, if any; disables the actions. */
  loading: string | null;
  /** The GitHub token was revoked or expired; offer a reconnect. */
  authErrorNeedsReconnect: boolean;
  setReconnectMode: (reconnect: boolean) => void;
  setRecoveryReason: (reason: GitUpstreamHealth) => void;
  setRecoveryOpen: (open: boolean) => void;
  setSetupOpen: (open: boolean) => void;
  onBackupNow: () => void;
}

/**
 * The backup page's status card: the backup state in plain language, the
 * repository, branch and (renamable) device name, and the next action.
 */
export function BackupStatusCard({
  gitStatus,
  remoteConfig,
  backupError,
  deviceName,
  setDeviceName,
  loading,
  authErrorNeedsReconnect,
  setReconnectMode,
  setRecoveryReason,
  setRecoveryOpen,
  setSetupOpen,
  onBackupNow,
}: BackupStatusCardProps) {
  const { t } = useTranslation();
  const [deviceNameDraft, setDeviceNameDraft] = useState("");
  const [deviceNameEditing, setDeviceNameEditing] = useState(false);

  const mode = useMemo(() => gitBackupMode(gitStatus, remoteConfig), [gitStatus, remoteConfig]);

  const statusMeta = useMemo(() => {
    // A failed backup stays visible (with a plain-language reason and a retry
    // action) instead of vanishing with the toast — §3.4 three-state language.
    if (backupError) {
      return {
        icon: XCircle,
        title: t("backup.status.failed"),
        description: backupError,
        className: "border-red-500/40 bg-red-500/10",
        iconClassName: "text-red-500",
      };
    }
    switch (mode) {
      case "loading":
        return {
          icon: Loader2,
          title: t("backup.status.loading"),
          description: t("backup.status.loadingDesc"),
          className: "border-border bg-surface",
          iconClassName: "text-muted animate-spin",
        };
      case "uninitialized":
      case "needs_remote":
        return {
          icon: Cloud,
          title: t("backup.status.notConnected"),
          description: t("backup.status.notConnectedDesc"),
          className: "border-border bg-surface",
          iconClassName: "text-muted",
        };
      case "needs_fix":
        return {
          icon: AlertTriangle,
          title: t("backup.status.needsFix"),
          description: t("backup.status.needsFixDesc"),
          className: "border-red-500/40 bg-red-500/10",
          iconClassName: "text-red-500",
        };
      case "pending_changes": {
        // Three distinct situations wear this state; naming them precisely
        // matters because "back up" reads as push-only and makes users fear
        // overwriting the remote when only remote updates exist.
        const { local: localCount, remote: remoteCount } = pendingBreakdown(gitStatus);
        const remoteOnly = remoteCount > 0 && localCount === 0;
        const both = remoteCount > 0 && localCount > 0;
        return {
          icon: remoteOnly ? RefreshCw : Upload,
          title: remoteOnly ? t("backup.status.remoteOnly") : t("backup.status.pending"),
          description: remoteOnly
            ? t("backup.status.remoteOnlyDesc", { remote: remoteCount })
            : both
              ? t("backup.status.pendingBothDesc", { local: localCount, remote: remoteCount })
              : (gitStatus?.changed_skill_count ?? 0) > 0
                ? t("backup.status.pendingSkills", { count: gitStatus?.changed_skill_count })
                : t("backup.status.pendingDesc", { local: localCount, remote: remoteCount }),
          className: "border-amber-500/40 bg-amber-500/10",
          iconClassName: "text-amber-600 dark:text-amber-400",
        };
      }
      case "up_to_date":
        return {
          icon: CheckCircle2,
          title: t("backup.status.synced"),
          description: t("backup.status.syncedDesc", {
            when: formatSnapshotWhen(gitStatus?.current_snapshot_tag ?? null) ?? t("backup.status.noSnapshot"),
          }),
          className: "border-emerald-500/30 bg-emerald-500/10",
          iconClassName: "text-emerald-600 dark:text-emerald-400",
        };
    }
  }, [backupError, gitStatus, mode, t]);

  const handleSaveDeviceName = async () => {
    const draft = deviceNameDraft.trim();
    setDeviceNameEditing(false);
    if (!draft || draft === deviceName) return;
    try {
      const saved = await api.backupSetDeviceName(draft);
      setDeviceName(saved);
      toast.success(t("backup.device.renamed"));
    } catch {
      toast.error(t("common.error"));
    }
  };

  const StatusIcon = statusMeta.icon;
  const canBackupNow = mode === "pending_changes" || mode === "up_to_date";
  const remoteLabel = gitStatus?.remote_url || remoteConfig || t("backup.connection.none");
  const branchLabel = gitStatus?.branch || t("backup.connection.unknown");

  return (
    <section className={cn("app-panel border p-4", statusMeta.className)}>
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="flex min-w-0 items-start gap-3">
          <div className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-md border border-border-subtle bg-surface">
            <StatusIcon className={cn("h-5 w-5", statusMeta.iconClassName)} />
          </div>
          <div className="min-w-0">
            <h2 className="text-[15px] font-semibold text-primary">{statusMeta.title}</h2>
            <p className="mt-1 text-[13px] leading-5 text-muted">{statusMeta.description}</p>
            <div className="mt-3 grid gap-2 text-[12px] text-tertiary sm:grid-cols-2">
              <div className="min-w-0">
                <div className="text-faint">{t("backup.connection.repository")}</div>
                <div className="truncate font-mono text-secondary" title={remoteLabel}>{remoteLabel}</div>
              </div>
              <div>
                <div className="text-faint">{t("backup.connection.branch")}</div>
                <div className="font-mono text-secondary">{branchLabel}</div>
              </div>
              <div className="min-w-0">
                <div className="text-faint">{t("backup.device.label")}</div>
                {deviceNameEditing ? (
                  <div className="mt-0.5 flex items-center gap-1">
                    <input
                      type="text"
                      value={deviceNameDraft}
                      onChange={(event) => setDeviceNameDraft(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") void handleSaveDeviceName();
                        if (event.key === "Escape") setDeviceNameEditing(false);
                      }}
                      autoFocus
                      maxLength={64}
                      className="h-6 min-w-0 flex-1 rounded-lg border border-border-subtle bg-background px-1.5 text-[12px] text-secondary outline-none focus:border-border"
                    />
                    <button
                      type="button"
                      onClick={handleSaveDeviceName}
                      className="rounded p-0.5 text-muted transition-colors hover:text-secondary"
                      title={t("common.save")}
                    >
                      <Check className="h-3.5 w-3.5" />
                    </button>
                  </div>
                ) : (
                  <div className="flex items-center gap-1">
                    <span className="truncate text-secondary">{deviceName || "-"}</span>
                    <button
                      type="button"
                      onClick={() => {
                        setDeviceNameDraft(deviceName);
                        setDeviceNameEditing(true);
                      }}
                      className="rounded p-0.5 text-faint transition-colors hover:text-secondary"
                      title={t("backup.device.rename")}
                    >
                      <Pencil className="h-3 w-3" />
                    </button>
                  </div>
                )}
              </div>
            </div>
          </div>
        </div>

        <div className="flex shrink-0 flex-wrap items-center gap-2">
          {authErrorNeedsReconnect && (
            <button
              type="button"
              onClick={() => setReconnectMode(true)}
              disabled={!!loading}
              className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 text-[13px] font-medium text-amber-700 transition-colors hover:bg-amber-500/15 disabled:opacity-50 dark:text-amber-300"
            >
              <Github className="h-3.5 w-3.5" />
              {t("backup.github.reconnect")}
            </button>
          )}
          {mode === "needs_fix" ? (
            <button
              type="button"
              onClick={() => {
                setRecoveryReason(gitStatus?.upstream_health ?? "unrelated_histories");
                setRecoveryOpen(true);
              }}
              disabled={!!loading}
              className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-red-500/40 bg-red-500/10 px-3 text-[13px] font-medium text-red-600 transition-colors hover:bg-red-500/15 disabled:opacity-50 dark:text-red-300"
            >
              <Wrench className="h-3.5 w-3.5" />
              {t("settings.gitRecoveryTitle")}
            </button>
          ) : mode === "uninitialized" || mode === "needs_remote" ? (
            <button
              type="button"
              onClick={() => setSetupOpen(true)}
              disabled={!!loading || !remoteConfig}
              className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-accent-border bg-accent-dark px-3 text-[13px] font-medium text-white transition-colors hover:bg-accent disabled:opacity-50"
            >
              {loading === "start" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Cloud className="h-3.5 w-3.5" />}
              {t("settings.gitStartBackup")}
            </button>
          ) : (
            <button
              type="button"
              onClick={onBackupNow}
              disabled={!!loading || !canBackupNow}
              className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-accent-border bg-accent-dark px-3 text-[13px] font-medium text-white transition-colors hover:bg-accent disabled:opacity-50"
            >
              {loading === "sync" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Upload className="h-3.5 w-3.5" />}
              {backupError
                ? t("backup.actions.retry")
                : mode === "up_to_date"
                  ? t("backup.actions.backupAgain")
                  : (gitStatus?.behind ?? 0) > 0
                    ? t("backup.actions.syncNow")
                    : t("backup.actions.backupNow")}
            </button>
          )}
        </div>
      </div>
    </section>
  );
}
