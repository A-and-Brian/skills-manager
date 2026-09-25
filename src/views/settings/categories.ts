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

/** Link options for a settings category, for `<Link>` and `navigate()`. */
export function settingsLink(category: SettingsCategory = DEFAULT_SETTINGS_CATEGORY) {
  return { to: "/settings/{-$category}", params: { category } } as const;
}

export function isSettingsPath(pathname: string) {
  return pathname === "/settings" || pathname.startsWith("/settings/");
}
