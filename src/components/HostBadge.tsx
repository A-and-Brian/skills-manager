import { Server } from "lucide-react";
import { useApp } from "../context/AppContext";

/** Names the remote host a settings section applies to; nothing on this computer. */
export function HostBadge() {
  const { activeHost } = useApp();
  if (!activeHost) return null;
  return (
    <span className="ml-2 inline-flex items-center gap-1 rounded-full border border-accent-border bg-accent-bg px-2 py-0.5 align-middle text-[11px] font-medium normal-case tracking-normal text-accent-light">
      <Server className="h-3 w-3" />
      {activeHost.name}
    </span>
  );
}
