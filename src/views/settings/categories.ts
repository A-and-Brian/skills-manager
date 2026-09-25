export const SETTINGS_CATEGORIES = [
  "general",
  "agents",
  "library",
  "backup",
  "network",
  "remote",
  "about",
] as const;

export type SettingsCategory = (typeof SETTINGS_CATEGORIES)[number];

export const DEFAULT_SETTINGS_CATEGORY: SettingsCategory = "general";

/** The category a `/settings/:category` param names, or null if it names none. */
export function resolveSettingsCategory(param: string | undefined): SettingsCategory | null {
  return SETTINGS_CATEGORIES.find((category) => category === param) ?? null;
}

export function settingsPath(category: SettingsCategory = DEFAULT_SETTINGS_CATEGORY) {
  return `/settings/${category}`;
}

export function isSettingsPath(pathname: string) {
  return pathname === "/settings" || pathname.startsWith("/settings/");
}
