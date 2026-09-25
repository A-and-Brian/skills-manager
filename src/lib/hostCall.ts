import { invoke as tauriInvoke, type InvokeArgs, type InvokeOptions } from "@tauri-apps/api/core";
import { isHostScoped } from "./hostScope";

/** The remote host the app operates on, or null for this computer. */
let activeHostId: string | null = null;

export function setActiveHostId(hostId: string | null) {
  activeHostId = hostId;
}

export function getActiveHostId(): string | null {
  return activeHostId;
}

/**
 * A check that the app is still on the host active now. A late answer from a
 * machine the app has switched away from must not replace the current
 * machine's data.
 */
export function trackHost(): () => boolean {
  const hostId = activeHostId;
  return () => activeHostId === hostId;
}

/**
 * `invoke` for the whole app: host-scoped commands go to the active host
 * through `remote_invoke`, which returns the same JSON the local command
 * would; everything else runs here.
 */
export function invoke<T>(command: string, args?: InvokeArgs, options?: InvokeOptions): Promise<T> {
  if (activeHostId !== null && isHostScoped(command, args)) {
    return tauriInvoke<T>(
      "remote_invoke",
      { hostId: activeHostId, command, args: args ?? {} },
      options
    );
  }
  return tauriInvoke<T>(command, args, options);
}

/** `invoke` on this computer whatever host is active, for the few reads that
 *  must stay local although the same command is routed, like the proxy the
 *  app updater uses. */
export function invokeLocal<T>(command: string, args?: InvokeArgs, options?: InvokeOptions): Promise<T> {
  return tauriInvoke<T>(command, args, options);
}
