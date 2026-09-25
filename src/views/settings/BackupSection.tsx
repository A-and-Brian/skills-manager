import { useState, useEffect } from "react";
import { Link as LinkIcon, Unlink, Loader2, ExternalLink } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useNavigate } from "react-router-dom";
import { ToggleSwitch } from "../../components/ToggleSwitch";
import * as api from "../../lib/tauri";
import { ACTION_BUTTON_CLASS, FIELD_CLASS } from "./shared";

export function BackupSection() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [gitRemoteInput, setGitRemoteInput] = useState("");
  const [gitRemoteSaving, setGitRemoteSaving] = useState(false);
  const [gitRemoteDisconnecting, setGitRemoteDisconnecting] = useState(false);
  const [gitEngineGit2, setGitEngineGit2] = useState(false);
  // Object merge is the default since 3d-β; "system" is the opt-out.
  const [gitMergeEngineObject, setGitMergeEngineObject] = useState(true);

  useEffect(() => {
    // The saved setting is the single source of truth. Do not backfill from
    // `.git/config` — that made a cleared URL reappear on reopen (#260).
    api.getSettings("git_backup_remote_url").then((v) => {
      setGitRemoteInput(v?.trim() || "");
    }).catch(() => {});
    api.getSettings("git_backup_engine").then((v) => {
      setGitEngineGit2(v?.trim() === "git2");
    }).catch(() => {});
    api.getSettings("merge_engine").then((v) => {
      setGitMergeEngineObject((v ?? "").trim() !== "system");
    }).catch(() => {});
  }, []);

  const handleSaveGitRemote = async () => {
    setGitRemoteSaving(true);
    try {
      // Credentials embedded in the URL go to the OS keychain; only the
      // sanitized URL is persisted (backup redesign §3.7).
      const trimmed = gitRemoteInput.trim();
      const effective = trimmed ? await api.gitBackupSanitizeRemoteUrl(trimmed) : "";
      await api.setSettings("git_backup_remote_url", effective);
      setGitRemoteInput(effective);
      toast.success(t("settings.gitConfigSaved"));
    } catch {
      toast.error(t("common.error"));
    } finally {
      setGitRemoteSaving(false);
    }
  };

  const handleDisconnectGitRemote = async () => {
    setGitRemoteDisconnecting(true);
    try {
      await api.gitBackupRemoveRemote();
      setGitRemoteInput("");
      toast.success(t("settings.gitDisconnected"));
    } catch {
      toast.error(t("common.error"));
    } finally {
      setGitRemoteDisconnecting(false);
    }
  };

  return (
    <section>
      <h2 className="app-section-title mb-3">
        {t("settings.gitSyncConfig")}
      </h2>
      <div className="app-panel overflow-hidden divide-y divide-border-faint">
        <div className="px-4 py-3">
          <h3 className="text-[14px] font-semibold text-primary">{t("settings.gitRemoteUrl")}</h3>
          <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
            <p className="mt-0.5 text-[12px] text-muted">{t("settings.gitSyncConfigDesc")}</p>
            <button
              type="button"
              onClick={() => navigate("/backup")}
              className={`${ACTION_BUTTON_CLASS} bg-surface-hover hover:bg-surface-active text-tertiary border-border`}
            >
              <ExternalLink className="w-3 h-3" />
              {t("settings.openBackupPage")}
            </button>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <input
              type="text"
              value={gitRemoteInput}
              onChange={(e) => setGitRemoteInput(e.target.value)}
              placeholder={t("settings.gitRemoteUrlPlaceholder")}
              className={`${FIELD_CLASS} min-w-0 flex-1 font-mono`}
            />
            <button
              onClick={handleSaveGitRemote}
              disabled={gitRemoteSaving}
              className={`${ACTION_BUTTON_CLASS} bg-surface-hover hover:bg-surface-active text-tertiary border-border`}
            >
              {gitRemoteSaving ? (
                <Loader2 className="w-3 h-3 animate-spin" />
              ) : (
                <LinkIcon className="w-3 h-3" />
              )}
              {t("common.save")}
            </button>
            <button
              onClick={handleDisconnectGitRemote}
              disabled={gitRemoteDisconnecting}
              className={`${ACTION_BUTTON_CLASS} bg-surface-hover hover:bg-surface-active text-tertiary border-border`}
            >
              {gitRemoteDisconnecting ? (
                <Loader2 className="w-3 h-3 animate-spin" />
              ) : (
                <Unlink className="w-3 h-3" />
              )}
              {t("settings.gitDisconnect")}
            </button>
          </div>
          <p className="text-[12px] text-muted mt-2">{t("settings.gitDisconnectHint")}</p>
          <div className="mt-3 flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="text-[14px] font-semibold text-primary">{t("settings.gitEngineGit2")}</div>
              <p className="mt-0.5 text-[12px] text-muted">{t("settings.gitEngineGit2Desc")}</p>
            </div>
            <ToggleSwitch
              className="mt-1"
              checked={gitEngineGit2}
              title={t("settings.gitEngineGit2")}
              onChange={async () => {
                const next = !gitEngineGit2;
                setGitEngineGit2(next);
                try {
                  await api.setSettings("git_backup_engine", next ? "git2" : "system");
                  toast.success(t("common.success"));
                } catch {
                  setGitEngineGit2(!next);
                  toast.error(t("common.error"));
                }
              }}
            />
          </div>
          <div className="mt-3 flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="text-[14px] font-semibold text-primary">{t("settings.gitMergeEngineObject")}</div>
              <p className="mt-0.5 text-[12px] text-muted">{t("settings.gitMergeEngineObjectDesc")}</p>
            </div>
            <ToggleSwitch
              className="mt-1"
              checked={gitMergeEngineObject}
              title={t("settings.gitMergeEngineObject")}
              onChange={async () => {
                const next = !gitMergeEngineObject;
                setGitMergeEngineObject(next);
                try {
                  await api.setSettings("merge_engine", next ? "object" : "system");
                  toast.success(t("common.success"));
                } catch {
                  setGitMergeEngineObject(!next);
                  toast.error(t("common.error"));
                }
              }}
            />
          </div>
        </div>
      </div>
    </section>
  );
}
