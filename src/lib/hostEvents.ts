import { listen, type EventCallback } from "@tauri-apps/api/event";
import { getActiveHostId } from "./hostCall";

/**
 * Whether an event is about the machine the app is showing. Events a host
 * forwards carry its `host_id`; this app's own events carry none.
 */
export function isForActiveHost(payload: unknown, activeHostId: string | null): boolean {
  const hostId =
    typeof payload === "object" && payload !== null
      ? (payload as { host_id?: unknown }).host_id
      : undefined;
  return typeof hostId === "string" ? hostId === activeHostId : activeHostId === null;
}

/** `listen`, minus events from a machine the app isn't showing. */
export function listenOnActiveHost<T>(event: string, handler: EventCallback<T>) {
  return listen<T>(event, (e) => {
    if (isForActiveHost(e.payload, getActiveHostId())) handler(e);
  });
}
