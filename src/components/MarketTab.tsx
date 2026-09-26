import { useMemo, useRef } from "react";
import {
  Check,
  ChevronLeft,
  ChevronRight,
  Clock,
  DownloadCloud,
  ExternalLink,
  Loader2,
  Plus,
  Search,
  Star,
  TrendingUp,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { cn } from "../utils";
import type { SkillsShSkill } from "../lib/tauri";
import { filterMarketSkills, paginateMarketSkills } from "../lib/marketSearch";
import { MARKET_SEARCH_STEP, type MarketSearch } from "../hooks/useMarketSearch";
import type { SourceOverflow } from "../hooks/useSourceOverflow";
import { StatusBanner } from "./StatusBanner";
import { SourceFilterPills } from "./SourceFilterPills";

const MARKET_PAGE_SIZE = 24;

interface MarketTabProps {
  market: MarketSearch;
  sourceOverflow: SourceOverflow;
  /** `source/skill_id` refs of the skills.sh skills already installed. */
  installedSourceRefs: Set<string>;
  /** Id of the skill being installed, if any. */
  installing: string | null;
  onInstall: (skill: SkillsShSkill) => void;
  onCancelInstall: (cancelKey: string) => void;
}

/**
 * The install page's market tab: browse or search skills.sh, filter by
 * source, page through results and install.
 */
export function MarketTab({
  market,
  sourceOverflow,
  installedSourceRefs,
  installing,
  onInstall,
  onCancelInstall,
}: MarketTabProps) {
  const { t } = useTranslation();
  const {
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
  } = market;
  const marketListRef = useRef<HTMLDivElement | null>(null);

  const scrollMarketListToTop = () => {
    marketListRef.current?.scrollIntoView({ behavior: "smooth", block: "start" });
  };

  const changeMarketPage = (page: number) => {
    setMarketPage(page);
    scrollMarketListToTop();
  };

  const filteredMarketSkills = useMemo(
    () => filterMarketSkills(marketSkills, marketSourceFilter, debouncedMarketQuery),
    [marketSkills, marketSourceFilter, debouncedMarketQuery]
  );

  const {
    totalPages: totalMarketPages,
    currentPage: currentMarketPage,
    items: paginatedMarketSkills,
    visiblePages: visibleMarketPages,
  } = paginateMarketSkills(filteredMarketSkills, marketPage, MARKET_PAGE_SIZE);
  const hasMarketQuery = debouncedMarketQuery.trim().length > 0;
  const canLoadMoreSearch = hasMarketQuery && marketSkills.length >= marketSearchLimit;
  const isLoadingMoreSearch = hasMarketQuery && marketLoadingMore;

  return (
    <div className="animate-in fade-in duration-300">
      <div className="app-panel mb-3 p-3.5">
        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-2">
            <div className="flex flex-col gap-1.5 lg:flex-row lg:items-center">
              {!hasMarketQuery ? (
                <div className="app-segmented shrink-0 bg-background">
                  {[
                    { id: "alltime" as const, label: t("install.all"), icon: Clock },
                    { id: "trending" as const, label: t("install.trending"), icon: TrendingUp },
                    { id: "hot" as const, label: t("install.hot"), icon: Star },
                  ].map((tab) => {
                    const Icon = tab.icon;
                    const isActive = marketTab === tab.id;
                    return (
                      <button
                        key={tab.id}
                        onClick={() => setMarketTab(tab.id)}
                        className={cn(
                          "app-segmented-button flex items-center gap-1.5",
                          isActive && "app-segmented-button-active"
                        )}
                      >
                        <Icon className="h-3 w-3" />
                        {tab.label}
                      </button>
                    );
                  })}
                </div>
              ) : null}

              <div className="relative flex-1 lg:max-w-[640px]">
                <Search className="pointer-events-none absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted" />
                <input
                  type="text"
                  value={marketQuery}
                  onChange={(event) => {
                    setMarketQuery(event.target.value);
                    setMarketSearchLimit(MARKET_SEARCH_STEP);
                  }}
                  placeholder={t("install.searchMarket")}
                  className="app-input w-full bg-background pl-9"
                  autoCapitalize="none"
                  autoCorrect="off"
                  spellCheck={false}
                />
              </div>
            </div>
          </div>

          {sourceOptions.length > 0 && (
            <SourceFilterPills
              sourceOptions={sourceOptions}
              sourceFilter={marketSourceFilter}
              onSourceFilterChange={setMarketSourceFilter}
              sourceOverflow={sourceOverflow}
            />
          )}
        </div>
      </div>

      {marketError ? (
        <div className="mb-4">
          <StatusBanner
            compact
            title={t("common.requestFailed")}
            description={marketError}
            actionLabel={t("common.retry")}
            onAction={() => setMarketReloadKey((value) => value + 1)}
            tone="danger"
          />
        </div>
      ) : null}

      {marketLoading && !marketLoadingMore ? (
        <div className="flex items-center justify-center py-16">
          <Loader2 className="h-5 w-5 animate-spin text-muted" />
        </div>
      ) : (
        <div className="pb-8">
          <div ref={marketListRef} className="scroll-mt-4" />

          {filteredMarketSkills.length === 0 ? (
            <div className="app-panel flex flex-col items-center justify-center rounded-2xl px-6 py-14 text-center">
              <div className="flex h-12 w-12 items-center justify-center rounded-2xl border border-border bg-background text-muted">
                <Search className="h-5 w-5" />
              </div>
              <h3 className="mt-4 text-[14px] font-semibold text-secondary">
                {t("install.noResults.title")}
              </h3>
              <p className="mt-1 max-w-md text-[13px] text-muted">
                {t("install.noResults.description")}
              </p>
            </div>
          ) : (
            <>
              <div className="grid grid-cols-2 gap-2.5 lg:grid-cols-3">
                {paginatedMarketSkills.map((skill) => {
                  const displayName = skill.name || skill.skill_id;
                  const showSkillId = skill.skill_id.trim() !== displayName.trim();
                  const owner = skill.source.split("/")[0];
                  const avatarUrl = `https://github.com/${owner}.png?size=32`;
                  const sourceRef = `${skill.source}/${skill.skill_id}`;
                  const isInstalled = installedSourceRefs.has(sourceRef);

                  return (
                  <div
                    key={skill.id}
                    className="app-panel flex flex-col gap-2 p-3 transition-colors hover:border-border"
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="flex min-w-0 flex-1 items-center gap-2">
                        <img
                          src={avatarUrl}
                          alt={owner}
                          className="h-6 w-6 shrink-0 rounded-full border border-border-subtle"
                          loading="lazy"
                        />
                        <div className="min-w-0">
                          <h3 className="truncate text-[13px] font-semibold text-secondary">
                            {displayName}
                          </h3>
                          {showSkillId ? (
                            <p className="truncate text-[13px] leading-4 text-muted">{skill.skill_id}</p>
                          ) : null}
                        </div>
                      </div>

                      <div className="flex shrink-0 items-center gap-1">
                        <button
                          onClick={() => openUrl(`https://skills.sh/${skill.source}/${skill.skill_id}`)}
                          className="rounded-[5px] p-1 text-muted transition-colors hover:bg-surface-hover hover:text-secondary"
                          title={t("install.viewOnWeb")}
                        >
                          <ExternalLink className="h-3.5 w-3.5" />
                        </button>
                        {isInstalled ? (
                          <span
                            className="rounded-[5px] border border-emerald-500/20 bg-emerald-500/10 p-1 text-emerald-400"
                            title={t("install.installed")}
                          >
                            <Check className="h-3.5 w-3.5" />
                          </span>
                        ) : installing === skill.id ? (
                          <button
                            onClick={() => onCancelInstall(`${skill.source}/${skill.skill_id}`)}
                            className="inline-flex items-center gap-1 rounded-[5px] border border-red-500/30 bg-red-500/10 px-1.5 py-1 text-red-400 transition-colors hover:bg-red-500/20"
                            title={t("install.cancel")}
                            aria-label={t("install.cancel")}
                          >
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            <span className="text-[11px] leading-none font-medium">
                              {t("install.cancel")}
                            </span>
                          </button>
                        ) : (
                          <button
                            onClick={() => onInstall(skill)}
                            disabled={installing !== null}
                            className="rounded-[5px] border border-accent-border bg-accent-dark p-1 text-white transition-colors hover:bg-accent disabled:opacity-50"
                            title={t("install.oneClickInstall")}
                          >
                            <Plus className="h-3.5 w-3.5" />
                          </button>
                        )}
                      </div>
                    </div>

                    <div className="flex flex-wrap items-center gap-1">
                      <button
                        type="button"
                        onClick={() => setMarketSourceFilter(skill.source)}
                        disabled={marketSourceFilter === skill.source}
                        title={t("install.onlyThisContributor")}
                        className={cn(
                          "rounded-[5px] bg-accent-bg px-1.5 py-0.5 text-[13px] leading-4 font-medium text-accent-light transition-colors",
                          marketSourceFilter === skill.source
                            ? "cursor-default opacity-90"
                            : "hover:bg-accent-bg/80"
                        )}
                      >
                        @{skill.source}
                      </button>
                      {marketTab === "alltime" && skill.installs > 0 && (
                        <span className="inline-flex items-center gap-1 rounded-[5px] border border-border-subtle bg-background px-1.5 py-0.5 text-[13px] leading-4 text-muted">
                          <DownloadCloud className="h-3 w-3" />
                          {skill.installs >= 1_000_000
                            ? `${(skill.installs / 1_000_000).toFixed(1)}M`
                            : skill.installs >= 1_000
                              ? `${(skill.installs / 1_000).toFixed(1)}K`
                              : skill.installs}
                        </span>
                      )}
                      {isInstalled ? (
                        <span className="inline-flex items-center gap-1 rounded-[5px] border border-emerald-500/20 bg-emerald-500/10 px-1.5 py-0.5 text-[13px] leading-4 font-medium text-emerald-400">
                          <Check className="h-3 w-3" />
                          {t("install.installed")}
                        </span>
                      ) : null}
                    </div>
                  </div>
                  );
                })}
              </div>

              {totalMarketPages > 1 ? (
                <div className="mt-5 flex flex-wrap items-center justify-center gap-1.5">
                  <button
                    onClick={() => changeMarketPage(Math.max(1, currentMarketPage - 1))}
                    disabled={currentMarketPage === 1}
                    className="inline-flex items-center gap-1 rounded-[6px] border border-border-subtle bg-surface px-3 py-1.5 text-[13px] font-medium text-secondary transition-colors hover:bg-surface-hover disabled:opacity-50"
                  >
                    <ChevronLeft className="h-3.5 w-3.5" />
                    {t("install.pagination.previous")}
                  </button>

                  {visibleMarketPages.map((page, index) => {
                    const previousPage = visibleMarketPages[index - 1];
                    const showGap = previousPage && page - previousPage > 1;

                    return (
                      <div key={page} className="flex items-center gap-1.5">
                        {showGap ? <span className="px-1 text-[13px] text-faint">...</span> : null}
                        <button
                          onClick={() => changeMarketPage(page)}
                          className={cn(
                            "min-w-8 rounded-[6px] border px-2.5 py-1.5 text-[13px] font-semibold transition-colors",
                            page === currentMarketPage
                              ? "border-accent-border bg-accent-dark text-white"
                              : "border-border-subtle bg-surface text-secondary hover:bg-surface-hover"
                          )}
                        >
                          {page}
                        </button>
                      </div>
                    );
                  })}

                  <button
                    onClick={() => changeMarketPage(Math.min(totalMarketPages, currentMarketPage + 1))}
                    disabled={currentMarketPage === totalMarketPages}
                    className="inline-flex items-center gap-1 rounded-[6px] border border-border-subtle bg-surface px-3 py-1.5 text-[13px] font-medium text-secondary transition-colors hover:bg-surface-hover disabled:opacity-50"
                  >
                    {t("install.pagination.next")}
                    <ChevronRight className="h-3.5 w-3.5" />
                  </button>
                </div>
              ) : null}

              {hasMarketQuery ? (
                <div className="mt-4 flex justify-center">
                  <button
                    type="button"
                    onClick={() => setMarketSearchLimit((value) => value + MARKET_SEARCH_STEP)}
                    disabled={!canLoadMoreSearch || marketLoading}
                    className="inline-flex items-center gap-2 rounded-[6px] border border-border-subtle bg-surface px-3.5 py-2 text-[13px] font-medium text-secondary transition-colors hover:bg-surface-hover disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {marketLoading ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <Search className="h-3.5 w-3.5" />
                    )}
                    {isLoadingMoreSearch
                      ? t("install.loadingMore")
                      : t("install.loadMoreSearch")}
                  </button>
                </div>
              ) : null}
            </>
          )}
        </div>
      )}
    </div>
  );
}
