import { UNTAGGED_FILTER } from "./skillTags";

/**
 * Whether a skill passes the selected tag pills: it carries one of them, or it
 * has no tags and the untagged pill is selected. No selection matches all.
 */
export function matchesTagFilter(tags: readonly string[], filters: ReadonlySet<string>): boolean {
  if (filters.size === 0) return true;
  const matchUntagged = filters.has(UNTAGGED_FILTER) && tags.length === 0;
  return matchUntagged || tags.some((tag) => filters.has(tag));
}

/**
 * Swap `oldTag` for `newTag` in the filter set, or drop it when there is no
 * new tag, so the current filtering survives a rename or delete. Returns
 * `filters` itself when it does not hold `oldTag`.
 */
export function replaceTagInFilters(filters: Set<string>, oldTag: string, newTag?: string): Set<string> {
  if (!filters.has(oldTag)) return filters;
  const next = new Set(filters);
  next.delete(oldTag);
  if (newTag) next.add(newTag);
  return next;
}

/** Existing tags to offer for a skill: ones it lacks that contain the keyword, ignoring case. */
export function tagSuggestions(allTags: readonly string[], skillTags: readonly string[], keyword: string): string[] {
  const needle = keyword.trim().toLowerCase();
  return allTags.filter((tag) => {
    if (skillTags.includes(tag)) return false;
    if (!needle) return true;
    return tag.toLowerCase().includes(needle);
  });
}
