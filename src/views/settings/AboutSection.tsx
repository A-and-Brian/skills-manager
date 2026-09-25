import { useState } from "react";
import {
  RefreshCw,
  Settings2,
  Github,
  Globe,
  Loader2,
  ExternalLink,
  BookOpen,
  Bug,
  Download,
  FileArchive,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { openUrl } from "@tauri-apps/plugin-opener";
import { check as checkUpdater } from "@tauri-apps/plugin-updater";
import { useApp } from "../../context/AppContext";
import * as api from "../../lib/tauri";
import { getErrorMessage } from "../../lib/error";
import { ACTION_BUTTON_CLASS, GITHUB_URL } from "./shared";

const IS_WINDOWS = navigator.userAgent.includes("Windows");
const IS_MACOS = navigator.userAgent.includes("Mac");

/** Platforms whose updater artifact can replace the running install.
 *
 *  Linux is excluded on purpose: only the AppImage can be updated in place,
 *  and a .deb/.rpm install is indistinguishable from it here, so those users
 *  keep the download link rather than a button that fails for half of them. */
const CAN_INSTALL_IN_APP = IS_WINDOWS || IS_MACOS;

const RESTART_TOAST_ID = "app-update-restart";

const WEBSITE_URL = "https://skillsmanager.dev";

interface AboutSectionProps {
  reportingIssue: boolean;
  onReportIssue: () => void;
}

export function AboutSection({ reportingIssue, onReportIssue }: AboutSectionProps) {
  const { t } = useTranslation();
  const { openHelp, appUpdate, refreshAppUpdate } = useApp();
  const [openingGithub, setOpeningGithub] = useState(false);
  const [exportingLogs, setExportingLogs] = useState(false);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [installing, setInstalling] = useState(false);

  const handleOpenGithub = async () => {
    try {
      setOpeningGithub(true);
      await openUrl(GITHUB_URL);
    } catch (error) {
      console.error("Failed to open GitHub repository", error);
      toast.error(t("common.error"));
    } finally {
      setOpeningGithub(false);
    }
  };

  const handleExportLogs = async () => {
    setExportingLogs(true);
    try {
      const result = await api.exportLogsZip();
      toast.success(t("settings.exportLogsDone", { count: result.file_count }), {
        description: result.zip_path,
      });
    } catch (error) {
      console.error("Failed to export logs", error);
      toast.error(t("settings.exportLogsFailed"));
    } finally {
      setExportingLogs(false);
    }
  };

  const handleCheckUpdate = async () => {
    setCheckingUpdate(true);
    try {
      const info = await refreshAppUpdate();
      if (info.has_update) {
        toast.info(t("settings.updateAvailable", { version: info.latest_version }));
      } else {
        toast.success(t("settings.noUpdate"));
      }
    } catch {
      toast.error(t("settings.updateError"));
    } finally {
      setCheckingUpdate(false);
    }
  };

  const handleAutoUpdate = async () => {
    setInstalling(true);
    try {
      // Read-only image or Gatekeeper-translocated copy: the updater would
      // download the whole bundle and only then fail to swap it, so stop first
      // and say what to do instead.
      const blocker = await api.updateInstallBlocker();
      if (blocker) {
        toast.error(t("settings.updateRelocate"));
        return;
      }
      // The updater plugin does not inherit the app's proxy setting the way
      // `check_app_update` does. Without this, a user behind a proxy is told a
      // new version exists and then cannot install it. The proxy given to
      // check() is carried through to the download.
      const proxy = (await api.getSettings("proxy_url")) || undefined;
      const update = await checkUpdater(proxy ? { proxy } : undefined);
      if (!update) {
        toast.success(t("settings.noUpdate"));
        return;
      }
      toast.info(t("settings.installing"));
      await update.downloadAndInstall();
      // Installing was the user's choice; restarting is a second one. Offered
      // as a toast action rather than a modal so a stray keypress cannot end
      // the session mid-task, and it stays up until acted on.
      toast.success(t("settings.restartToApply"), {
        id: RESTART_TOAST_ID,
        duration: Infinity,
        action: {
          label: t("settings.restartNow"),
          onClick: () => {
            api.restartApp().catch((err) => {
              toast.error(getErrorMessage(err, t("common.error")));
            });
          },
        },
      });
    } catch (err) {
      console.error("In-app update failed:", err);
      toast.error(t("settings.updateError"));
      if (appUpdate?.release_url) {
        await openUrl(appUpdate.release_url);
      }
    } finally {
      setInstalling(false);
    }
  };

  return (
    <div className="app-panel flex flex-wrap items-start justify-between gap-3 p-4">
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <div className="w-8 h-8 rounded-lg bg-surface-hover border border-border flex items-center justify-center">
          <Settings2 className="w-4 h-4 text-accent" />
        </div>
        <div>
          <h3 className="text-[13px] font-semibold text-primary">{t("settings.version")}</h3>
          <p className="text-muted text-[13px]">
            {t("settings.tagline")}
            {appUpdate?.has_update && (
              <span className="ml-2 text-amber-500 font-medium">
                {t("settings.updateAvailable", { version: appUpdate.latest_version })}
              </span>
            )}
          </p>
        </div>
      </div>
      <div className="flex flex-wrap gap-2">
        {appUpdate?.has_update ? (
          CAN_INSTALL_IN_APP ? (
            <>
              <button
                type="button"
                onClick={handleAutoUpdate}
                disabled={installing}
                className={`${ACTION_BUTTON_CLASS} bg-accent text-white border-accent hover:opacity-90`}
              >
                {installing ? (
                  <Loader2 className="w-3 h-3 animate-spin" />
                ) : (
                  <Download className="w-3 h-3" />
                )}
                {installing ? t("settings.installing") : t("settings.installUpdate")}
              </button>
              <button
                type="button"
                onClick={() => { openUrl(appUpdate.release_url).catch(() => {}); }}
                className={`${ACTION_BUTTON_CLASS} bg-surface-hover hover:bg-surface-active text-tertiary border-border`}
              >
                <ExternalLink className="w-3 h-3" /> {t("settings.download")}
              </button>
            </>
          ) : (
            <button
              type="button"
              onClick={() => { openUrl(appUpdate.release_url).catch(() => {}); }}
              className={`${ACTION_BUTTON_CLASS} bg-accent text-white border-accent hover:opacity-90`}
            >
              <Download className="w-3 h-3" /> {t("settings.download")}
            </button>
          )
        ) : (
          <button
            type="button"
            onClick={handleCheckUpdate}
            disabled={checkingUpdate}
            className={`${ACTION_BUTTON_CLASS} bg-surface-hover hover:bg-surface-active text-tertiary border-border`}
          >
            {checkingUpdate ? (
              <Loader2 className="w-3 h-3 animate-spin" />
            ) : (
              <RefreshCw className="w-3 h-3" />
            )}
            {checkingUpdate ? t("settings.checking") : t("settings.checkUpdate")}
          </button>
        )}
        <button
          type="button"
          onClick={openHelp}
          className={`${ACTION_BUTTON_CLASS} bg-surface-hover hover:bg-surface-active text-tertiary border-border`}
        >
          <BookOpen className="w-3 h-3" /> {t("settings.help")}
        </button>
        <button
          type="button"
          onClick={onReportIssue}
          disabled={reportingIssue}
          title={t("settings.reportIssueHint")}
          className={`${ACTION_BUTTON_CLASS} bg-surface-hover hover:bg-surface-active text-tertiary border-border`}
        >
          {reportingIssue ? (
            <Loader2 className="w-3 h-3 animate-spin" />
          ) : (
            <Bug className="w-3 h-3" />
          )}
          {t("settings.reportIssue")}
        </button>
        <button
          type="button"
          onClick={handleExportLogs}
          disabled={exportingLogs}
          title={t("settings.exportLogsHint")}
          className={`${ACTION_BUTTON_CLASS} bg-surface-hover hover:bg-surface-active text-tertiary border-border`}
        >
          {exportingLogs ? (
            <Loader2 className="w-3 h-3 animate-spin" />
          ) : (
            <FileArchive className="w-3 h-3" />
          )}
          {t("settings.exportLogs")}
        </button>
        <button
          type="button"
          onClick={() => { openUrl(WEBSITE_URL).catch(() => {}); }}
          className={`${ACTION_BUTTON_CLASS} bg-surface-hover hover:bg-surface-active text-tertiary border-border`}
        >
          <Globe className="w-3 h-3" /> {t("settings.website")}
        </button>
        <button
          type="button"
          onClick={handleOpenGithub}
          disabled={openingGithub}
          className={`${ACTION_BUTTON_CLASS} bg-surface-hover hover:bg-surface-active text-tertiary border-border`}
        >
          <Github className="w-3 h-3" /> GitHub
        </button>
      </div>
    </div>
  );
}
