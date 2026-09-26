import { Copy, ExternalLink, Github, Loader2 } from "lucide-react";
import { writeText as clipboardWriteText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { GithubDeviceFlowStart } from "../lib/tauri";

const GITHUB_TOKEN_URL =
  "https://github.com/settings/tokens/new?scopes=repo&description=Skills%20Manager%20Backup";

interface GithubConnectCardProps {
  /** Reconnecting after the token was revoked, rather than a first connect. */
  reconnectMode: boolean;
  /** The device-flow code being waited on, if any. */
  deviceInfo: GithubDeviceFlowStart | null;
  /** The view's in-flight action, if any; `"github"` while connecting. */
  loading: string | null;
  githubRepoName: string;
  setGithubRepoName: (name: string) => void;
  /** Show the personal access token form instead of only device sign-in. */
  patMode: boolean;
  setPatMode: (patMode: boolean) => void;
  githubToken: string;
  setGithubToken: (token: string) => void;
  githubError: string | null;
  setGithubError: (error: string | null) => void;
  onDeviceFlow: () => void;
  onCancelDeviceFlow: () => void;
  onConnect: () => void;
}

/**
 * Connect the backup to a GitHub repo: device-flow sign-in, or a personal
 * access token.
 */
export function GithubConnectCard({
  reconnectMode,
  deviceInfo,
  loading,
  githubRepoName,
  setGithubRepoName,
  patMode,
  setPatMode,
  githubToken,
  setGithubToken,
  githubError,
  setGithubError,
  onDeviceFlow,
  onCancelDeviceFlow,
  onConnect,
}: GithubConnectCardProps) {
  const { t } = useTranslation();

  return (
    <section className="app-panel p-4">
      <div className="mb-3 flex items-center gap-2">
        <Github className="h-4 w-4 text-muted" />
        <h2 className="text-[14px] font-semibold text-secondary">
          {reconnectMode ? t("backup.github.reconnectTitle") : t("backup.github.title")}
        </h2>
      </div>
      <p className="mb-3 text-[13px] leading-5 text-muted">{t("backup.github.desc")}</p>

      {deviceInfo ? (
        <div className="space-y-3">
          <div className="flex flex-col items-center gap-2 rounded-md border border-border-subtle bg-bg-secondary px-4 py-4">
            <div className="font-mono text-[26px] font-bold tracking-[0.25em] text-primary">
              {deviceInfo.user_code}
            </div>
            <button
              type="button"
              onClick={() => {
                void clipboardWriteText(deviceInfo.user_code);
                toast.success(t("backup.github.deviceCodeCopied"));
              }}
              className="inline-flex items-center gap-1 text-[12px] text-muted transition-colors hover:text-secondary"
            >
              <Copy className="h-3 w-3" />
              {t("backup.github.deviceCopyCode")}
            </button>
          </div>
          <p className="text-[13px] leading-5 text-muted">
            {t("backup.github.deviceWaitDesc", { uri: deviceInfo.verification_uri })}
          </p>
          <div className="flex items-center justify-between gap-2">
            <span className="inline-flex items-center gap-1.5 text-[12px] text-muted">
              <Loader2 className="h-3 w-3 animate-spin" />
              {t("backup.github.deviceWaiting")}
            </span>
            <button
              type="button"
              onClick={onCancelDeviceFlow}
              className="rounded-lg px-2.5 py-1 text-[12px] font-medium text-tertiary transition-colors hover:bg-surface-hover hover:text-secondary"
            >
              {t("common.cancel")}
            </button>
          </div>
        </div>
      ) : (
        <div className="space-y-2">
          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={onDeviceFlow}
              disabled={!!loading}
              className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-accent-border bg-accent-dark px-3 text-[13px] font-medium text-white transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
            >
              {loading === "github" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Github className="h-3.5 w-3.5" />}
              {loading === "github" ? t("backup.github.connecting") : t("backup.github.deviceSignIn")}
            </button>
            <input
              type="text"
              value={githubRepoName}
              onChange={(event) => setGithubRepoName(event.target.value)}
              disabled={loading === "github"}
              title={t("backup.github.repoLabel")}
              className="h-8 w-52 rounded-lg border border-border-subtle bg-background px-2.5 font-mono text-[13px] text-secondary outline-none transition-colors focus:border-border disabled:opacity-50"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
            />
          </div>

          {patMode ? (
            <>
              <div className="flex flex-wrap items-center gap-2">
                <input
                  type="password"
                  value={githubToken}
                  onChange={(event) => {
                    setGithubToken(event.target.value);
                    setGithubError(null);
                  }}
                  placeholder={t("backup.github.tokenPlaceholder")}
                  disabled={loading === "github"}
                  className="h-8 min-w-0 flex-1 rounded-lg border border-border-subtle bg-background px-2.5 font-mono text-[13px] text-secondary outline-none transition-colors focus:border-border disabled:opacity-50"
                  autoCapitalize="none"
                  autoCorrect="off"
                  spellCheck={false}
                />
                <button
                  type="button"
                  onClick={onConnect}
                  disabled={!!loading || !githubToken.trim()}
                  className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-border bg-surface-hover px-2.5 text-[13px] font-medium text-tertiary transition-colors hover:bg-surface-active disabled:cursor-not-allowed disabled:opacity-50"
                >
                  {t("backup.github.connect")}
                </button>
              </div>
              <button
                type="button"
                onClick={() => void openUrl(GITHUB_TOKEN_URL)}
                className="inline-flex items-center gap-1 text-[12px] text-muted transition-colors hover:text-secondary"
              >
                <ExternalLink className="h-3 w-3" />
                {t("backup.github.tokenHint")}
              </button>
            </>
          ) : (
            <button
              type="button"
              onClick={() => setPatMode(true)}
              className="text-[12px] text-muted transition-colors hover:text-secondary"
            >
              {t("backup.github.patToggle")}
            </button>
          )}

          {githubError && (
            <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-[12px] leading-5 text-red-600 dark:text-red-300">
              {githubError}
            </div>
          )}
        </div>
      )}
    </section>
  );
}
