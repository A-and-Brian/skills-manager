import { describe, expect, it } from "vitest";
import type { SkillsShSkill } from "./tauri";
import {
  MARKET_SEARCH_CACHE_MAX_ENTRIES,
  MARKET_SEARCH_CACHE_TTL_MS,
  filterMarketSkills,
  isLoadMoreRequest,
  marketSearchCacheKey,
  paginateMarketSkills,
  pruneMarketSearchCache,
  type MarketSearchCacheEntry,
} from "./marketSearch";

function skill(id: string, overrides: Partial<SkillsShSkill> = {}): SkillsShSkill {
  return { id, skill_id: id, name: id, source: "acme/skills", installs: 0, ...overrides };
}

function skills(count: number) {
  return Array.from({ length: count }, (_, i) => skill(`s${i + 1}`));
}

const NOW = 1_000_000;

describe("pruneMarketSearchCache", () => {
  it("drops expired entries", () => {
    const cache = new Map<string, MarketSearchCacheEntry>([
      ["old", { timestamp: NOW - MARKET_SEARCH_CACHE_TTL_MS, data: [] }],
      ["fresh", { timestamp: NOW - 1, data: [] }],
    ]);
    pruneMarketSearchCache(cache, NOW);
    expect([...cache.keys()]).toEqual(["fresh"]);
  });

  it("evicts the oldest entries down to the cap", () => {
    const cache = new Map<string, MarketSearchCacheEntry>();
    for (let i = 0; i < MARKET_SEARCH_CACHE_MAX_ENTRIES + 2; i++) {
      // Insert newest first so eviction follows timestamps, not insertion order.
      cache.set(`q${i}`, { timestamp: NOW - i, data: [] });
    }
    pruneMarketSearchCache(cache, NOW);
    expect(cache.size).toBe(MARKET_SEARCH_CACHE_MAX_ENTRIES);
    expect(cache.has(`q${MARKET_SEARCH_CACHE_MAX_ENTRIES + 1}`)).toBe(false);
    expect(cache.has(`q${MARKET_SEARCH_CACHE_MAX_ENTRIES}`)).toBe(false);
    expect(cache.has("q0")).toBe(true);
  });
});

describe("marketSearchCacheKey", () => {
  it("ignores query case and includes the limit", () => {
    expect(marketSearchCacheKey("React", 60)).toBe("react|60");
  });
});

describe("isLoadMoreRequest", () => {
  it("loads more when a search with results asks for a higher limit", () => {
    expect(isLoadMoreRequest("react", 60, 120)).toBe(true);
  });

  it("is a fresh request without a query, without results or without a higher limit", () => {
    expect(isLoadMoreRequest("", 60, 120)).toBe(false);
    expect(isLoadMoreRequest("react", 0, 120)).toBe(false);
    expect(isLoadMoreRequest("react", 60, 60)).toBe(false);
  });
});

describe("filterMarketSkills", () => {
  const list = [
    skill("a", { installs: 5, source: "one" }),
    skill("b", { installs: 50, source: "two" }),
    skill("c", { installs: 20, source: "one" }),
  ];

  it("keeps the leaderboard order without a query", () => {
    expect(filterMarketSkills(list, "all", " ").map((s) => s.id)).toEqual(["a", "b", "c"]);
  });

  it("sorts search results by installs", () => {
    expect(filterMarketSkills(list, "all", "x").map((s) => s.id)).toEqual(["b", "c", "a"]);
  });

  it("filters by source", () => {
    expect(filterMarketSkills(list, "one", "x").map((s) => s.id)).toEqual(["c", "a"]);
  });
});

describe("paginateMarketSkills", () => {
  it("returns the requested page", () => {
    const page = paginateMarketSkills(skills(25), 2, 10);
    expect(page.items.map((s) => s.id)).toEqual(["s11", "s12", "s13", "s14", "s15", "s16", "s17", "s18", "s19", "s20"]);
    expect(page.totalPages).toBe(3);
  });

  it("clamps the page to the last one", () => {
    const page = paginateMarketSkills(skills(25), 9, 10);
    expect(page.currentPage).toBe(3);
    expect(page.items).toHaveLength(5);
  });

  it("has one empty page when there are no results", () => {
    const page = paginateMarketSkills([], 1, 10);
    expect(page.totalPages).toBe(1);
    expect(page.visiblePages).toEqual([1]);
  });

  it("shows every page number up to 7 pages", () => {
    expect(paginateMarketSkills(skills(70), 4, 10).visiblePages).toEqual([1, 2, 3, 4, 5, 6, 7]);
  });

  it("shows first, last and the current page's neighbours beyond 7 pages", () => {
    expect(paginateMarketSkills(skills(100), 5, 10).visiblePages).toEqual([1, 4, 5, 6, 10]);
    expect(paginateMarketSkills(skills(100), 1, 10).visiblePages).toEqual([1, 2, 10]);
  });
});
