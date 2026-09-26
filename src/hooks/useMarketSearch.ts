import { useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import * as api from "../lib/tauri";
import type { SkillsShSkill } from "../lib/tauri";
import {
  MARKET_SEARCH_CACHE_TTL_MS,
  isLoadMoreRequest,
  marketSearchCacheKey,
  pruneMarketSearchCache,
  type MarketSearchCacheEntry,
} from "../lib/marketSearch";

export const MARKET_SEARCH_STEP = 60;
const MARKET_SEARCH_DEBOUNCE_MS = 450;

/**
 * skills.sh leaderboard and search results. Loads only while `active`,
 * debounces the query, caches search pages, and drops results from requests
 * that were superseded before they landed.
 */
export function useMarketSearch(active: boolean) {
  const { t } = useTranslation();
  const [marketTab, setMarketTab] = useState<"hot" | "trending" | "alltime">("alltime");
  const [marketQuery, setMarketQuery] = useState("");
  const [marketSourceFilter, setMarketSourceFilter] = useState("all");
  const [marketSkills, setMarketSkills] = useState<SkillsShSkill[]>([]);
  const [marketPage, setMarketPage] = useState(1);
  const [marketSearchLimit, setMarketSearchLimit] = useState(MARKET_SEARCH_STEP);
  const [marketLoading, setMarketLoading] = useState(false);
  const [marketLoadingMore, setMarketLoadingMore] = useState(false);
  const [marketError, setMarketError] = useState<string | null>(null);
  const [marketReloadKey, setMarketReloadKey] = useState(0);
  const marketSearchCacheRef = useRef<Map<string, MarketSearchCacheEntry>>(new Map());
  const marketSkillsLengthRef = useRef(0);
  const [debouncedMarketQuery, setDebouncedMarketQuery] = useState("");
  const deferredMarketQuery = useDeferredValue(marketQuery);

  useEffect(() => {
    const timer = setTimeout(() => {
      setDebouncedMarketQuery(deferredMarketQuery);
    }, MARKET_SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [deferredMarketQuery]);

  useEffect(() => {
    marketSkillsLengthRef.current = marketSkills.length;
  }, [marketSkills.length]);

  useEffect(() => {
    if (!active) return;

    const query = debouncedMarketQuery.trim();
    const loadingMore = isLoadMoreRequest(query, marketSkillsLengthRef.current, marketSearchLimit);

    if (query.length > 0 && !loadingMore) {
      const cacheKey = marketSearchCacheKey(query, marketSearchLimit);
      const cached = marketSearchCacheRef.current.get(cacheKey);
      if (cached && Date.now() - cached.timestamp < MARKET_SEARCH_CACHE_TTL_MS) {
        setMarketSkills(cached.data);
        setMarketLoading(false);
        setMarketLoadingMore(false);
        setMarketPage(1);
        setMarketError(null);
        return;
      }
    }

    setMarketLoadingMore(loadingMore);
    setMarketLoading(true);
    if (!loadingMore) {
      setMarketPage(1);
    }
    setMarketError(null);

    let stale = false;
    const request = query
      ? api.searchSkillssh(query, marketSearchLimit)
      : api.fetchLeaderboard(marketTab);

    request
      .then((result) => {
        if (stale) return;
        setMarketSkills(result);
        if (query.length > 0 && !loadingMore) {
          const cacheKey = marketSearchCacheKey(query, marketSearchLimit);
          marketSearchCacheRef.current.set(cacheKey, { timestamp: Date.now(), data: result });
          pruneMarketSearchCache(marketSearchCacheRef.current, Date.now());
        }
        if (!loadingMore) {
          setMarketSourceFilter("all");
        }
      })
      .catch((e) => {
        if (stale) return;
        console.error(e);
        const message = e?.toString?.() || t("common.error");
        setMarketError(message);
        toast.error(message);
      })
      .finally(() => {
        if (stale) return;
        setMarketLoading(false);
        setMarketLoadingMore(false);
      });

    return () => { stale = true; };
  }, [active, debouncedMarketQuery, marketReloadKey, marketSearchLimit, marketTab, t]);

  const sourceOptions = useMemo(
    () => Array.from(new Set(marketSkills.map((skill) => skill.source))),
    [marketSkills]
  );

  return {
    marketTab,
    setMarketTab,
    marketQuery,
    setMarketQuery,
    marketSourceFilter,
    setMarketSourceFilter,
    marketSkills,
    marketPage,
    setMarketPage,
    marketSearchLimit,
    setMarketSearchLimit,
    marketLoading,
    marketLoadingMore,
    marketError,
    setMarketReloadKey,
    debouncedMarketQuery,
    sourceOptions,
  };
}

export type MarketSearch = ReturnType<typeof useMarketSearch>;
