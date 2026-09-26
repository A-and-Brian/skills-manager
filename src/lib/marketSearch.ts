import type { SkillsShSkill } from "./tauri";

export const MARKET_SEARCH_CACHE_TTL_MS = 120_000;
export const MARKET_SEARCH_CACHE_MAX_ENTRIES = 150;

export interface MarketSearchCacheEntry {
  timestamp: number;
  data: SkillsShSkill[];
}

export function marketSearchCacheKey(query: string, limit: number) {
  return `${query.toLowerCase()}|${limit}`;
}

/** Drops expired entries, then the oldest ones until the cache fits its cap. */
export function pruneMarketSearchCache(cache: Map<string, MarketSearchCacheEntry>, now: number) {
  for (const [key, value] of Array.from(cache.entries())) {
    if (now - value.timestamp >= MARKET_SEARCH_CACHE_TTL_MS) {
      cache.delete(key);
    }
  }

  if (cache.size <= MARKET_SEARCH_CACHE_MAX_ENTRIES) {
    return;
  }

  const sorted = Array.from(cache.entries()).sort((a, b) => a[1].timestamp - b[1].timestamp);
  const removeCount = cache.size - MARKET_SEARCH_CACHE_MAX_ENTRIES;
  for (const [key] of sorted.slice(0, removeCount)) {
    cache.delete(key);
  }
}

/** A raised limit on a search that already has results fetches the next batch. */
export function isLoadMoreRequest(query: string, loadedCount: number, limit: number) {
  return query.length > 0 && loadedCount > 0 && limit > loadedCount;
}

/** Filters by source; search results are ranked by installs, the leaderboard keeps its order. */
export function filterMarketSkills(skills: SkillsShSkill[], sourceFilter: string, query: string) {
  const filtered = sourceFilter === "all"
    ? skills
    : skills.filter((skill) => skill.source === sourceFilter);
  if (query.trim().length > 0) {
    return [...filtered].sort((a, b) => b.installs - a.installs);
  }
  return filtered;
}

/**
 * One page of results plus the page numbers to show: all of them up to 7,
 * otherwise the first, the last and the neighbours of the current page.
 */
export function paginateMarketSkills(skills: SkillsShSkill[], page: number, pageSize: number) {
  const totalPages = Math.max(1, Math.ceil(skills.length / pageSize));
  const currentPage = Math.min(page, totalPages);
  const start = (currentPage - 1) * pageSize;
  const visiblePages = Array.from({ length: totalPages }, (_, index) => index + 1).filter((p) => {
    if (totalPages <= 7) return true;
    if (p === 1 || p === totalPages) return true;
    return Math.abs(p - currentPage) <= 1;
  });
  return {
    totalPages,
    currentPage,
    items: skills.slice(start, start + pageSize),
    visiblePages,
  };
}
