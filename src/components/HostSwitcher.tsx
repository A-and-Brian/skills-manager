import { useEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Check, ChevronsUpDown, Loader2, Monitor, Server, Settings2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../utils";
import { useApp } from "../context/AppContext";
import { settingsLink } from "../views/settings/categories";

/** Sidebar control choosing which machine the whole app operates on. */
export function HostSwitcher() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { remoteHosts, activeHost, hostSession, connectingHostId, switchHost } = useApp();
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    const handlePointer = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) setOpen(false);
    };
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      setOpen(false);
      triggerRef.current?.focus();
    };
    document.addEventListener("mousedown", handlePointer);
    document.addEventListener("keydown", handleEscape);
    return () => {
      document.removeEventListener("mousedown", handlePointer);
      document.removeEventListener("keydown", handleEscape);
    };
  }, [open]);

  const connectingHost = remoteHosts.find((host) => host.id === connectingHostId);
  const lost = hostSession?.lostMessage != null;
  const choose = (hostId: string | null) => {
    setOpen(false);
    if (hostId === (activeHost?.id ?? null) && !lost) return;
    void switchHost(hostId);
  };

  const option = (
    key: string,
    hostId: string | null,
    icon: typeof Monitor,
    label: string,
    detail: string,
    monoDetail: boolean
  ) => {
    const Icon = icon;
    const selected = hostId === (activeHost?.id ?? null);
    const connecting = hostId !== null && hostId === connectingHostId;
    return (
      <button
        key={key}
        type="button"
        role="menuitemradio"
        aria-checked={selected}
        disabled={connectingHostId !== null}
        onClick={() => choose(hostId)}
        className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] text-secondary transition-colors hover:bg-surface-hover disabled:cursor-wait"
      >
        <Icon className="h-3.5 w-3.5 shrink-0 text-muted" />
        <span className="min-w-0 flex-1">
          <span className="block truncate">{label}</span>
          <span className={cn("block truncate text-[11px] text-faint", monoDetail && "font-mono")}>{detail}</span>
        </span>
        {connecting ? (
          <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-muted" />
        ) : selected ? (
          <Check className="h-3.5 w-3.5 shrink-0 text-accent" />
        ) : null}
      </button>
    );
  };

  return (
    <div ref={containerRef} className="relative px-2.5 pb-2 shrink-0">
      <button
        ref={triggerRef}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "flex w-full items-center gap-2 rounded-md border px-2.5 py-1.5 text-left text-[13px] transition-colors outline-none focus-visible:ring-2 focus-visible:ring-border",
          activeHost
            ? "border-accent-border bg-accent-bg text-primary"
            : "border-border-subtle bg-surface text-secondary hover:bg-surface-hover"
        )}
      >
        {connectingHost ? (
          <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-muted" />
        ) : activeHost ? (
          <Server className={cn("h-3.5 w-3.5 shrink-0", lost ? "text-amber-500" : "text-accent")} />
        ) : (
          <Monitor className="h-3.5 w-3.5 shrink-0 text-muted" />
        )}
        <span className="min-w-0 flex-1 truncate">
          {connectingHost
            ? t("hostSwitcher.connecting", { name: connectingHost.name })
            : activeHost?.name ?? t("hostSwitcher.local")}
        </span>
        <ChevronsUpDown className="h-3.5 w-3.5 shrink-0 text-faint" />
      </button>

      {open && (
        <div
          role="menu"
          aria-label={t("hostSwitcher.label")}
          className="absolute inset-x-2.5 top-full z-40 mt-1 rounded-lg border border-border bg-surface p-1 shadow-lg"
        >
          {option("local", null, Monitor, t("hostSwitcher.local"), t("hostSwitcher.localDetail"), false)}
          {remoteHosts.map((host) => option(host.id, host.id, Server, host.name, host.ssh_target, true))}
          <div className="my-1 border-t border-border-subtle" />
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setOpen(false);
              navigate(settingsLink("remote"));
            }}
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] text-tertiary transition-colors hover:bg-surface-hover hover:text-secondary"
          >
            <Settings2 className="h-3.5 w-3.5 shrink-0 text-muted" />
            {t("hostSwitcher.manage")}
          </button>
        </div>
      )}
    </div>
  );
}
