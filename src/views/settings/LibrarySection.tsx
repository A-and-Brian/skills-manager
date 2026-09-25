import { useState, useEffect } from "react";
import {
  Folder,
  FolderOpen,
  Link as LinkIcon,
  Copy,
  Loader2,
  ExternalLink,
  Pencil,
  RotateCcw,
  X,
  Check,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { cn } from "../../utils";
import * as api from "../../lib/tauri";
import { listenOnActiveHost } from "../../lib/hostEvents";
import { useApp } from "../../context/AppContext";
import { HostBadge } from "../../components/HostBadge";
import { useRemotePickerBlock } from "../../hooks/useRemotePickerBlock";
import {
  ACTION_BUTTON_CLASS,
  FIELD_CLASS,
  SEGMENTED_BUTTON_CLASS,
  compactHomePath,
  pickDirectory,
} from "./shared";

export function LibrarySection() {
  const { t } = useTranslation();
  const { activeHost, reconnectHost } = useApp();
  const pickerBlock = useRemotePickerBlock();
  // A host picks up a new library path when its session starts again.
  const announceRepoPathChange = () => {
    if (!activeHost) {
      toast.info(t("settings.repoPathRestartNotice"));
      return;
    }
    toast.info(t("settings.repoPathReconnectNotice", { name: activeHost.name }), {
      duration: 10000,
      action: { label: t("remoteSession.reconnect"), onClick: () => void reconnectHost() },
    });
  };
  const [syncMode, setSyncMode] = useState("symlink");
  const [defaultDeployMode, setDefaultDeployMode] = useState<api.ProjectDeployMode>("link");
  const [openingRepo, setOpeningRepo] = useState(false);
  const [centralRepoPath, setCentralRepoPath] = useState("");
  const [centralRepoPathOverride, setCentralRepoPathOverride] = useState<string | null>(null);
  const [editingCentralRepoPath, setEditingCentralRepoPath] = useState(false);
  const [centralRepoPathInput, setCentralRepoPathInput] = useState("");
  const [savingCentralRepoPath, setSavingCentralRepoPath] = useState(false);
  const [autoUpdateInterval, setAutoUpdateInterval] = useState("off");
  const [autoUpdateApply, setAutoUpdateApply] = useState("off");
  const [autoUpdateLastRun, setAutoUpdateLastRun] = useState<string | null>(null);

  useEffect(() => {
    api.getSettings("sync_mode").then((v) => { if (v) setSyncMode(v); });
    api.getSettings("default_project_deploy_mode").then((v) => { if (v === "copy") setDefaultDeployMode(v); });
    api.getCentralRepoPath().then((path) => {
      setCentralRepoPath(path);
      setCentralRepoPathInput(path);
    }).catch(() => {});
    api.getCentralRepoPathOverride().then(setCentralRepoPathOverride).catch(() => {});
    api.getSettings("auto_update_check_interval").then((v) => { if (v) setAutoUpdateInterval(v); });
    api.getSettings("auto_update_apply").then((v) => { if (v) setAutoUpdateApply(v); });
    // The `skills-auto-updated` listener may populate this concurrently, so
    // keep whichever timestamp is newer rather than blindly overwriting.
    api.getSettings("auto_update_last_run_at").then((v) => {
      if (!v) return;
      setAutoUpdateLastRun((prev) =>
        prev && Date.parse(prev) >= Date.parse(v) ? prev : v
      );
    });
  }, []);

  const handleSyncModeChange = async (mode: string) => {
    setSyncMode(mode);
    await api.setSettings("sync_mode", mode);
  };

  const handleDefaultDeployModeChange = async (mode: api.ProjectDeployMode) => {
    const previous = defaultDeployMode;
    setDefaultDeployMode(mode);
    try {
      await api.setSettings("default_project_deploy_mode", mode);
    } catch {
      setDefaultDeployMode(previous);
      toast.error(t("common.error"));
    }
  };

  const handleOpenRepoInFinder = async () => {
    try {
      setOpeningRepo(true);
      await api.openCentralRepoFolder();
    } catch (error) {
      console.error("Failed to open central repository folder", error);
      toast.error(t("common.error"));
    } finally {
      setOpeningRepo(false);
    }
  };

  const handleStartEditCentralRepoPath = () => {
    setCentralRepoPathInput(centralRepoPathOverride ?? centralRepoPath);
    setEditingCentralRepoPath(true);
  };

  const handleSaveCentralRepoPath = async () => {
    const trimmed = centralRepoPathInput.trim();
    if (!trimmed) {
      toast.error(t("settings.repoPathEmpty"));
      return;
    }
    setSavingCentralRepoPath(true);
    try {
      const nextPath = await api.setCentralRepoPath(trimmed);
      setCentralRepoPath(nextPath);
      setCentralRepoPathOverride(nextPath);
      setEditingCentralRepoPath(false);
      toast.success(t("settings.repoPathSaved"));
      announceRepoPathChange();
    } catch (error) {
      toast.error(String(error));
    } finally {
      setSavingCentralRepoPath(false);
    }
  };

  const handleResetCentralRepoPath = async () => {
    setSavingCentralRepoPath(true);
    try {
      const nextPath = await api.setCentralRepoPath(null);
      setCentralRepoPath(nextPath);
      setCentralRepoPathOverride(null);
      setCentralRepoPathInput(nextPath);
      setEditingCentralRepoPath(false);
      toast.success(t("settings.repoPathReset"));
      announceRepoPathChange();
    } catch (error) {
      toast.error(String(error));
    } finally {
      setSavingCentralRepoPath(false);
    }
  };

  const handleAutoUpdateIntervalChange = async (value: string) => {
    setAutoUpdateInterval(value);
    await api.setSettings("auto_update_check_interval", value);
  };

  const handleAutoUpdateApplyChange = async (value: string) => {
    setAutoUpdateApply(value);
    await api.setSettings("auto_update_apply", value);
  };

  // Keep the last-run timestamp in sync with both the background scheduler
  // and the tray's manual "Check for skill updates" so the user doesn't see
  // a stale value if Settings is open. Backend always persists `last_run_at`
  // first and then emits with the same `ran_at`, so reading from the payload
  // avoids a follow-up DB roundtrip.
  useEffect(() => {
    type AutoUpdatedPayload = { ran_at?: string };
    const unlistenPromise = listenOnActiveHost<AutoUpdatedPayload>("skills-auto-updated", (event) => {
      const ranAt = event.payload?.ran_at;
      if (ranAt) {
        setAutoUpdateLastRun(ranAt);
      }
    });
    return () => {
      unlistenPromise
        .then((unlisten) => unlisten())
        .catch(() => {});
    };
  }, []);

  const autoUpdateIntervalOptions = [
    { value: "off", label: t("settings.autoUpdate.intervalOff") },
    { value: "1h", label: t("settings.autoUpdate.interval1h") },
    { value: "6h", label: t("settings.autoUpdate.interval6h") },
    { value: "24h", label: t("settings.autoUpdate.interval24h") },
  ] as const;
  const autoUpdateApplyOptions = [
    { value: "off", label: t("settings.autoUpdate.applyOff") },
    { value: "on", label: t("settings.autoUpdate.applyOn") },
  ] as const;

  const displayedRepoPath = centralRepoPath
    ? compactHomePath(centralRepoPath)
    : t("common.loading");

  return (
    <section>
      <h2 className="app-section-title mb-3">
        {t("settings.categories.library")}
        <HostBadge />
      </h2>
      <div className="app-panel overflow-hidden divide-y divide-border-faint">
        {/* Repo path */}
        <div className="flex flex-wrap items-start justify-between gap-3 px-5 py-4">
          <div className="min-w-0 flex-1">
            <h3 className="text-[14px] font-semibold text-primary">{t("settings.repoPath")}</h3>
            <p className="mt-0.5 text-[12px] text-muted">{t("settings.repoPathDesc")}</p>
          </div>
          <div className="flex max-w-full flex-wrap items-center gap-2">
            {editingCentralRepoPath ? (
              <div className="flex min-w-[320px] max-w-full items-center gap-1">
                <input
                  type="text"
                  value={centralRepoPathInput}
                  onChange={(e) => setCentralRepoPathInput(e.target.value)}
                  className={`${FIELD_CLASS} min-w-0 flex-1 font-mono`}
                  autoFocus
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void handleSaveCentralRepoPath();
                    if (e.key === "Escape") {
                      setCentralRepoPathInput(centralRepoPathOverride ?? centralRepoPath);
                      setEditingCentralRepoPath(false);
                    }
                  }}
                />
                <button
                  type="button"
                  onClick={() => pickDirectory(setCentralRepoPathInput)}
                  disabled={savingCentralRepoPath || !!pickerBlock}
                  title={pickerBlock}
                  className={`${ACTION_BUTTON_CLASS} text-muted hover:text-secondary`}
                >
                  <FolderOpen className="w-3 h-3" />
                  {t("settings.selectFolder")}
                </button>
                <button
                  type="button"
                  onClick={() => void handleSaveCentralRepoPath()}
                  disabled={savingCentralRepoPath}
                  className={`${ACTION_BUTTON_CLASS} border-emerald-500/30 text-emerald-600 hover:bg-emerald-500/5 dark:text-emerald-400`}
                >
                  {savingCentralRepoPath ? (
                    <Loader2 className="w-3 h-3 animate-spin" />
                  ) : (
                    <Check className="w-3 h-3" />
                  )}
                  {t("common.save")}
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setCentralRepoPathInput(centralRepoPathOverride ?? centralRepoPath);
                    setEditingCentralRepoPath(false);
                  }}
                  disabled={savingCentralRepoPath}
                  className={`${ACTION_BUTTON_CLASS} text-muted hover:text-secondary`}
                >
                  <X className="w-3 h-3" />
                </button>
              </div>
            ) : (
              <div className="flex min-w-0 items-center gap-1.5 rounded-lg border border-border-subtle bg-background px-3 py-2">
                <Folder className="w-3 h-3 text-muted" />
                <span className="truncate text-[13px] font-mono text-tertiary">{displayedRepoPath}</span>
              </div>
            )}
            {!editingCentralRepoPath && (
              <button
                type="button"
                onClick={handleStartEditCentralRepoPath}
                className={`${ACTION_BUTTON_CLASS} text-muted hover:text-secondary`}
              >
                <Pencil className="w-3 h-3" />
                {t("settings.changeDir")}
              </button>
            )}
            {!editingCentralRepoPath && centralRepoPathOverride && (
              <button
                type="button"
                onClick={() => void handleResetCentralRepoPath()}
                disabled={savingCentralRepoPath}
                className={`${ACTION_BUTTON_CLASS} text-muted hover:text-secondary`}
              >
                {savingCentralRepoPath ? (
                  <Loader2 className="w-3 h-3 animate-spin" />
                ) : (
                  <RotateCcw className="w-3 h-3" />
                )}
                {t("settings.resetPath")}
              </button>
            )}
            <button
              type="button"
              onClick={handleOpenRepoInFinder}
              disabled={openingRepo || !!activeHost}
              title={activeHost ? t("remoteSession.finderUnavailable", { name: activeHost.name }) : undefined}
              className={cn(
                ACTION_BUTTON_CLASS,
                "border-accent-border bg-accent-bg text-accent",
                "hover:border-accent hover:bg-accent-bg",
                openingRepo && "cursor-wait opacity-70"
              )}
            >
              {openingRepo ? (
                <Loader2 className="w-3 h-3 animate-spin" />
              ) : (
                <ExternalLink className="w-3 h-3" />
              )}
              {t("settings.openInFinder")}
            </button>
          </div>
          <div className="w-full text-[12px] text-muted">
            {centralRepoPathOverride
              ? t("settings.repoPathCustomHint")
              : t("settings.repoPathDefaultHint")}
          </div>
        </div>

        {/* Sync mode */}
        <div className="flex flex-wrap items-start justify-between gap-3 px-5 py-4">
          <div className="min-w-0 flex-1">
            <h3 className="text-[14px] font-semibold text-primary">{t("settings.syncMode")}</h3>
            <p className="mt-0.5 text-[12px] text-muted">{t("settings.syncModeDesc")}</p>
          </div>
          <div className="app-segmented flex-wrap bg-background">
            <button
              onClick={() => handleSyncModeChange("symlink")}
              className={cn(
                SEGMENTED_BUTTON_CLASS,
                syncMode === "symlink" ? "bg-surface-active text-secondary" : "text-muted hover:text-tertiary"
              )}
            >
              <LinkIcon className="w-3 h-3" /> {t("settings.symlink")}
            </button>
            <button
              onClick={() => handleSyncModeChange("copy")}
              className={cn(
                SEGMENTED_BUTTON_CLASS,
                syncMode === "copy" ? "bg-surface-active text-secondary" : "text-muted hover:text-tertiary"
              )}
            >
              <Copy className="w-3 h-3" /> {t("settings.copy")}
            </button>
          </div>
        </div>

        {/* Default deploy mode for new projects */}
        <div className="flex flex-wrap items-start justify-between gap-3 px-5 py-4">
          <div className="min-w-0 flex-1">
            <h3 className="text-[14px] font-semibold text-primary">{t("settings.defaultProjectMode")}</h3>
            <p className="mt-0.5 text-[12px] text-muted">
              {t("settings.defaultProjectModeDesc")} {t(`project.settings.modeHint.${defaultDeployMode}`)}
            </p>
          </div>
          <div className="app-segmented flex-wrap bg-background">
            {(["link", "copy"] as const).map((mode) => (
              <button
                key={mode}
                onClick={() => handleDefaultDeployModeChange(mode)}
                className={cn(
                  SEGMENTED_BUTTON_CLASS,
                  defaultDeployMode === mode ? "bg-surface-active text-secondary" : "text-muted hover:text-tertiary"
                )}
              >
                {t(`project.settings.mode.${mode}`)}
              </button>
            ))}
          </div>
        </div>

        {/* Skill auto-update */}
        <div className="flex flex-wrap items-start justify-between gap-3 px-5 py-4">
          <div className="min-w-0 flex-1">
            <h3 className="text-[14px] font-semibold text-primary">
              {t("settings.autoUpdate.intervalLabel")}
            </h3>
            <p className="mt-0.5 text-[12px] text-muted">
              {t("settings.autoUpdate.intervalDesc")}
              {autoUpdateLastRun
                ? ` · ${t("settings.autoUpdate.lastRun", {
                    time: new Date(autoUpdateLastRun).toLocaleString(),
                  })}`
                : ""}
            </p>
          </div>
          <div className="app-segmented flex-wrap bg-background">
            {autoUpdateIntervalOptions.map((option) => (
              <button
                key={option.value}
                type="button"
                aria-pressed={autoUpdateInterval === option.value}
                onClick={() => handleAutoUpdateIntervalChange(option.value)}
                className={cn(
                  SEGMENTED_BUTTON_CLASS,
                  autoUpdateInterval === option.value
                    ? "bg-surface-active text-secondary"
                    : "text-muted hover:text-tertiary"
                )}
              >
                {option.label}
              </button>
            ))}
          </div>
        </div>

        <div className="flex flex-wrap items-start justify-between gap-3 px-5 py-4">
          <div className="min-w-0 flex-1">
            <h3 className="text-[14px] font-semibold text-primary">
              {t("settings.autoUpdate.applyLabel")}
            </h3>
            <p className="mt-0.5 text-[12px] text-muted">
              {t("settings.autoUpdate.applyDesc")}
            </p>
          </div>
          <div className="app-segmented flex-wrap bg-background">
            {autoUpdateApplyOptions.map((option) => (
              <button
                key={option.value}
                type="button"
                aria-pressed={autoUpdateApply === option.value}
                onClick={() => handleAutoUpdateApplyChange(option.value)}
                className={cn(
                  SEGMENTED_BUTTON_CLASS,
                  autoUpdateApply === option.value
                    ? "bg-surface-active text-secondary"
                    : "text-muted hover:text-tertiary"
                )}
              >
                {option.label}
              </button>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}
