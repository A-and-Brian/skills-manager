import { Check, DownloadCloud, Github, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../utils";
import type { ManagedSkill } from "../lib/tauri";

interface GitInstallTabProps {
  gitUrl: string;
  /** Clone and preview in flight. */
  gitLoading: boolean;
  /** Key to cancel the clone in flight with. */
  gitCancelKey: string | null;
  findInstalledByGitUrl: (url: string) => ManagedSkill | undefined;
  onGitUrlChange: (url: string) => void;
  onPreview: () => void;
  onCancelInstall: (cancelKey: string) => void;
}

/** The install page's git tab: clone a repo URL to preview its skills. */
export function GitInstallTab({
  gitUrl,
  gitLoading,
  gitCancelKey,
  findInstalledByGitUrl,
  onGitUrlChange,
  onPreview,
  onCancelInstall,
}: GitInstallTabProps) {
  const { t } = useTranslation();

  return (
    <div className="animate-in fade-in duration-300">
      <div className="app-panel max-w-lg p-5">
        <div className="mb-4 flex h-10 w-10 items-center justify-center rounded-lg border border-border bg-surface-hover">
          <Github className="h-5 w-5 text-tertiary" />
        </div>
        <h2 className="mb-1 text-[14px] font-semibold text-primary">{t("install.gitTitle")}</h2>
        <p className="mb-4 text-[13px] text-muted">{t("install.gitDesc")}</p>

        <div className="space-y-3">
          <div>
            <label className="mb-1 block text-[13px] font-medium text-tertiary">
              {t("install.repoUrl")}
            </label>
            <input
              type="text"
              value={gitUrl}
              onChange={(e) => onGitUrlChange(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter" && !gitLoading && gitUrl.trim()) onPreview(); }}
              placeholder={t("install.repoUrlPlaceholder")}
              disabled={gitLoading}
              className="app-input w-full bg-background"
            />
          </div>
          {gitUrl.trim() && findInstalledByGitUrl(gitUrl) && (
            <div className="flex items-center gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[13px] text-amber-400">
              <Check className="h-3.5 w-3.5 shrink-0" />
              <span>
                {t("install.gitAlreadyInstalled", { name: findInstalledByGitUrl(gitUrl)!.name })}
              </span>
            </div>
          )}
          <div className="flex gap-2 pt-2">
            {gitLoading ? (
              <button
                onClick={() => gitCancelKey && onCancelInstall(gitCancelKey)}
                className="inline-flex w-full items-center justify-center gap-2 rounded-lg border border-red-500/30 bg-red-500/10 px-4 py-2.5 text-[13px] font-medium text-red-400 transition-colors hover:bg-red-500/20"
                disabled={!gitCancelKey}
              >
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("install.cancel")}
              </button>
            ) : (
              <button
                onClick={onPreview}
                disabled={!gitUrl.trim()}
                className={cn(
                  "flex w-full",
                  gitUrl.trim() && findInstalledByGitUrl(gitUrl)
                    ? "app-button-secondary bg-background"
                    : "app-button-primary"
                )}
              >
                <DownloadCloud className="h-3.5 w-3.5" />
                {gitUrl.trim() && findInstalledByGitUrl(gitUrl)
                  ? t("install.gitReinstall")
                  : t("install.installClone")}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
