export const INSTALL_TABS = ["market", "local", "git"] as const;

export type InstallTab = (typeof INSTALL_TABS)[number];

export interface InstallSearch {
  tab?: InstallTab;
}

/**
 * Keeps `?tab=` only when it names a known install tab. Always returns the
 * `tab` key: the router layers the result over the raw query, so leaving the
 * key out would let an unknown `?tab=` value through.
 */
export function parseInstallSearch(search: Record<string, unknown>): InstallSearch {
  return { tab: INSTALL_TABS.find((known) => known === search.tab) };
}
