import { Loader2, Monitor, RefreshCw, Server, WifiOff } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../utils";
import { useApp } from "../context/AppContext";

/** Strip under the title bar saying which machine the app is changing. */
export function RemoteBanner() {
  const { t } = useTranslation();
  const { activeHost, hostSession, connectingHostId, switchHost } = useApp();
  if (!activeHost) return null;

  const lost = hostSession?.lostMessage != null;
  const info = hostSession?.info;
  const detail = lost
    ? hostSession?.lostMessage || undefined
    : info && `Skills Manager ${info.version} · ${info.os}/${info.arch} · ${info.base_dir}`;
  const reconnecting = connectingHostId === activeHost.id;
  // Nothing else may start while a switch is connecting.
  const busy = connectingHostId !== null;
  const buttonClass =
    "inline-flex shrink-0 items-center gap-1.5 rounded-md px-2 py-1 text-[12px] font-medium transition-colors disabled:opacity-60";

  return (
    <div
      role="status"
      className={cn(
        "flex shrink-0 items-center gap-2.5 border-b px-5 py-1.5 text-[12px]",
        lost
          ? "border-amber-500/30 bg-amber-500/10 text-amber-800 dark:text-amber-200"
          : "border-accent-border bg-accent-bg text-secondary"
      )}
    >
      {lost ? (
        <WifiOff className="h-3.5 w-3.5 shrink-0" />
      ) : (
        <Server className="h-3.5 w-3.5 shrink-0 text-accent" />
      )}
      <p className="min-w-0 flex-1 truncate" title={detail}>
        {lost ? (
          t("remoteSession.lost", { name: activeHost.name })
        ) : (
          <>
            {t("remoteSession.operatingOn")}{" "}
            <span className="font-semibold text-primary">{activeHost.name}</span>
            <span className="text-faint"> · </span>
            <span className="font-mono text-muted">{activeHost.ssh_target}</span>
          </>
        )}
      </p>
      {lost && (
        <button
          type="button"
          onClick={() => void switchHost(activeHost.id)}
          disabled={busy}
          className={cn(buttonClass, "bg-amber-500/15 hover:bg-amber-500/25")}
        >
          {reconnecting ? (
            <Loader2 className="h-3 w-3 animate-spin" />
          ) : (
            <RefreshCw className="h-3 w-3" />
          )}
          {t("remoteSession.reconnect")}
        </button>
      )}
      <button
        type="button"
        onClick={() => void switchHost(null)}
        disabled={busy}
        className={cn(buttonClass, "text-tertiary hover:bg-surface-hover hover:text-secondary")}
      >
        <Monitor className="h-3 w-3" />
        {t("remoteSession.switchToLocal")}
      </button>
    </div>
  );
}
