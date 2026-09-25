import { useState, useEffect } from "react";
import {
  Folder,
  FolderOpen,
  Link as LinkIcon,
  Copy,
  Loader2,
  ExternalLink,
  Sun,
  Moon,
  Monitor,
  Type,
  Pencil,
  RotateCcw,
  X,
  Check,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { cn } from "../../utils";
import { useThemeContext } from "../../context/ThemeContext";
import { ToggleSwitch } from "../../components/ToggleSwitch";
import * as api from "../../lib/tauri";
import { applyTextSize } from "../../lib/textScale";
import type { Theme } from "../../hooks/useTheme";
import {
  ACTION_BUTTON_CLASS,
  FIELD_CLASS,
  SEGMENTED_BUTTON_CLASS,
  compactHomePath,
  pickDirectory,
} from "./shared";

export function GlobalConfigSection() {
  const { t, i18n } = useTranslation();
  const { theme, setTheme } = useThemeContext();
  const [syncMode, setSyncMode] = useState("symlink");
  const [closeAction, setCloseAction] = useState("");
  const [showTrayIcon, setShowTrayIcon] = useState(true);
  const [openingRepo, setOpeningRepo] = useState(false);
  const [centralRepoPath, setCentralRepoPath] = useState("");
  const [centralRepoPathOverride, setCentralRepoPathOverride] = useState<string | null>(null);
  const [editingCentralRepoPath, setEditingCentralRepoPath] = useState(false);
  const [centralRepoPathInput, setCentralRepoPathInput] = useState("");
  const [savingCentralRepoPath, setSavingCentralRepoPath] = useState(false);
  const [textSize, setTextSize] = useState("default");

  useEffect(() => {
    api.getSettings("sync_mode").then((v) => { if (v) setSyncMode(v); });
    api.getSettings("close_action").then((v) => { setCloseAction(v ?? ""); });
    api.getSettings("show_tray_icon").then((v) => {
      const normalized = (v ?? "true").trim().toLowerCase();
      setShowTrayIcon(!(normalized === "false" || normalized === "0" || normalized === "no" || normalized === "off"));
    });
    api.getSettings("text_size").then((v) => { if (v) { setTextSize(v); applyTextSize(v); } });
    api.getCentralRepoPath().then((path) => {
      setCentralRepoPath(path);
      setCentralRepoPathInput(path);
    }).catch(() => {});
    api.getCentralRepoPathOverride().then(setCentralRepoPathOverride).catch(() => {});
  }, []);

  const handleSyncModeChange = async (mode: string) => {
    setSyncMode(mode);
    await api.setSettings("sync_mode", mode);
  };

  const handleCloseActionChange = async (action: string) => {
    if (action === "hide" && !showTrayIcon) return;
    setCloseAction(action);
    await api.setSettings("close_action", action);
  };

  const handleShowTrayIconChange = async (enabled: boolean) => {
    setShowTrayIcon(enabled);
    await api.setSettings("show_tray_icon", enabled ? "true" : "false");
    if (!enabled && closeAction === "hide") {
      setCloseAction("close");
      await api.setSettings("close_action", "close");
    }
  };

  const handleLanguageChange = (lng: string) => {
    localStorage.setItem("language", lng);
    i18n.changeLanguage(lng);
    api.setSettings("language", lng);
  };

  const handleTextSizeChange = (size: string) => {
    setTextSize(size);
    applyTextSize(size);
    api.setSettings("text_size", size);
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
      toast.info(t("settings.repoPathRestartNotice"));
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
      toast.info(t("settings.repoPathRestartNotice"));
    } catch (error) {
      toast.error(String(error));
    } finally {
      setSavingCentralRepoPath(false);
    }
  };

  const themeOptions: Array<{ value: Theme; label: string; icon: typeof Sun }> = [
    { value: "light", label: t("settings.themeLight"), icon: Sun },
    { value: "dark", label: t("settings.themeDark"), icon: Moon },
    { value: "system", label: t("settings.themeSystem"), icon: Monitor },
  ];
  const displayedRepoPath = centralRepoPath
    ? compactHomePath(centralRepoPath)
    : t("common.loading");

  return (
    <section>
      <h2 className="app-section-title mb-3">
        {t("settings.globalConfig")}
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
                  disabled={savingCentralRepoPath}
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
              disabled={openingRepo}
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

        {/* Theme */}
        <div className="flex flex-wrap items-start justify-between gap-3 px-5 py-4">
          <div className="min-w-0 flex-1">
            <h3 className="text-[14px] font-semibold text-primary">{t("settings.theme")}</h3>
            <p className="mt-0.5 text-[12px] text-muted">{t("settings.themeDesc")}</p>
          </div>
          <div className="app-segmented flex-wrap bg-background">
            {themeOptions.map((opt) => {
              const Icon = opt.icon;
              return (
                <button
                  key={opt.value}
                  onClick={() => setTheme(opt.value)}
                  className={cn(
                    SEGMENTED_BUTTON_CLASS,
                    theme === opt.value ? "bg-surface-active text-secondary" : "text-muted hover:text-tertiary"
                  )}
                >
                  <Icon className="w-3 h-3" /> {opt.label}
                </button>
              );
            })}
          </div>
        </div>

        {/* Text size */}
        <div className="flex flex-wrap items-start justify-between gap-3 px-5 py-4">
          <div className="min-w-0 flex-1">
            <h3 className="text-[14px] font-semibold text-primary">{t("settings.textSize")}</h3>
            <p className="mt-0.5 text-[12px] text-muted">{t("settings.textSizeDesc")}</p>
          </div>
          <div className="app-segmented flex-wrap bg-background">
            {([
              { value: "small", label: t("settings.textSizeSmall") },
              { value: "default", label: t("settings.textSizeDefault") },
              { value: "large", label: t("settings.textSizeLarge") },
              { value: "xlarge", label: t("settings.textSizeXLarge") },
            ] as const).map((opt) => (
              <button
                key={opt.value}
                onClick={() => handleTextSizeChange(opt.value)}
                className={cn(
                  SEGMENTED_BUTTON_CLASS,
                  textSize === opt.value ? "bg-surface-active text-secondary" : "text-muted hover:text-tertiary"
                )}
              >
                {opt.value === "small" && <Type className="w-2.5 h-2.5" />}
                {opt.value === "default" && <Type className="w-3 h-3" />}
                {opt.value === "large" && <Type className="w-3.5 h-3.5" />}
                {opt.value === "xlarge" && <Type className="w-4 h-4" />}
                {opt.label}
              </button>
            ))}
          </div>
        </div>

        {/* Language */}
        <div className="flex flex-wrap items-start justify-between gap-3 px-5 py-4">
          <div className="min-w-0 flex-1">
            <h3 className="text-[14px] font-semibold text-primary">{t("settings.language")}</h3>
          </div>
          <div className="app-segmented flex-wrap bg-background">
            {([
              { value: "zh", label: "简体中文" },
              { value: "zh-TW", label: "繁體中文" },
              { value: "en", label: "English" },
            ] as const).map((opt) => (
              <button
                key={opt.value}
                onClick={() => handleLanguageChange(opt.value)}
                className={cn(
                  SEGMENTED_BUTTON_CLASS,
                  i18n.language === opt.value
                    ? "bg-surface-active text-secondary"
                    : "text-muted hover:text-tertiary"
                )}
              >
                {opt.label}
              </button>
            ))}
          </div>
        </div>

        {/* Close action */}
        <div className="flex flex-wrap items-start justify-between gap-3 px-5 py-4">
          <div className="min-w-0 flex-1">
            <h3 className="text-[14px] font-semibold text-primary">{t("settings.closeAction")}</h3>
            <p className="mt-0.5 text-[12px] text-muted">{t("settings.closeActionDesc")}</p>
            {!showTrayIcon && (
              <p className="text-[12px] text-muted mt-1">{t("settings.trayIconOffHint")}</p>
            )}
          </div>
          <div className="app-segmented flex-wrap bg-background">
            {(["", "hide", "close"] as const).map((val) => (
              <button
                key={val}
                onClick={() => handleCloseActionChange(val)}
                disabled={val === "hide" && !showTrayIcon}
                className={cn(
                  SEGMENTED_BUTTON_CLASS,
                  closeAction === val ? "bg-surface-active text-secondary" : "text-muted hover:text-tertiary",
                  val === "hide" && !showTrayIcon && "opacity-50 cursor-not-allowed hover:text-muted"
                )}
              >
                {t(`settings.closeAction_${val || "ask"}`)}
              </button>
            ))}
          </div>
        </div>

        {/* Tray icon */}
        <div className="flex flex-wrap items-start justify-between gap-3 px-5 py-4">
          <div className="min-w-0 flex-1">
            <h3 className="text-[14px] font-semibold text-primary">{t("settings.trayIcon")}</h3>
            <p className="mt-0.5 text-[12px] text-muted">{t("settings.trayIconDesc")}</p>
          </div>
          <ToggleSwitch
            className="mt-1"
            checked={showTrayIcon}
            onChange={() => handleShowTrayIconChange(!showTrayIcon)}
            title={showTrayIcon ? t("settings.trayIcon_on") : t("settings.trayIcon_off")}
          />
        </div>
      </div>
    </section>
  );
}
