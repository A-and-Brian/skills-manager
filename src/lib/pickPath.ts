import { open } from "@tauri-apps/plugin-dialog";
import { getActiveHostId } from "./hostCall";

/** A folder, or one file with one of `files` extensions (given without dots). */
export type PickRequest =
  | { directory: true }
  | { files: string[]; filterName?: string; multiple?: false };

export interface PickOptions {
  /** Where the remote browser starts instead of the host's home folder. The
   *  native dialog keeps opening where it remembers, as it always has. */
  startPath?: string;
}

type RemotePicker = (request: PickRequest, opts: PickOptions) => Promise<string | null>;

let remotePicker: RemotePicker | null = null;

/** Set by the mounted remote folder browser; null when it unmounts. */
export function setRemotePicker(picker: RemotePicker | null) {
  remotePicker = picker;
}

/**
 * Ask for one path on the machine the app operates on: the native dialog on
 * this computer, the remote folder browser while a host is active. Resolves
 * null when the choice is cancelled.
 */
export async function pickPath(request: PickRequest, opts: PickOptions = {}): Promise<string | null> {
  if (getActiveHostId() !== null) {
    if (!remotePicker) throw new Error("The remote folder browser is not mounted");
    return remotePicker(request, opts);
  }
  const selected = await open(
    "directory" in request
      ? { directory: true, multiple: false }
      : { multiple: false, filters: [{ name: request.filterName ?? "Files", extensions: request.files }] }
  );
  return typeof selected === "string" ? selected : null;
}
