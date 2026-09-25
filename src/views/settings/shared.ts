import { open as dialogOpen } from "@tauri-apps/plugin-dialog";

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

export async function pickDirectory(setter: (v: string) => void) {
  const selected = await dialogOpen({ directory: true, multiple: false });
  if (selected && typeof selected === "string") {
    setter(selected);
  }
}
