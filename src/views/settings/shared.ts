import { pickPath } from "../../lib/pickPath";

export const GITHUB_URL = "https://github.com/xingkongliang/skills-manager";

// Compose the shared control classes from index.css rather than a parallel
// set — bg-background keeps fields readable against the surface-colored panel.
export const FIELD_CLASS = "app-input bg-background";
export const ACTION_BUTTON_CLASS = "app-button-secondary gap-1.5";
export const SEGMENTED_BUTTON_CLASS = "app-segmented-button flex items-center gap-1.5";

export function compactHomePath(path: string) {
  return path
    .replace(/\/Users\/[^/]+/, "~")
    .replace(/\/home\/[^/]+/, "~")
    .replace(/^[A-Za-z]:\\Users\\[^\\]+/, "~");
}

/** Choose a folder on the machine the app operates on; `startPath` only guides the remote browser. */
export async function pickDirectory(setter: (v: string) => void, startPath?: string) {
  const selected = await pickPath({ directory: true }, { startPath });
  if (selected) setter(selected);
}
