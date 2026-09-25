import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import {
  DownloadCloud,
  UploadCloud,
  Github,
  Box,
  Star,
  TrendingUp,
  Clock,
  Plus,
  Loader2,
  ExternalLink,
  Check,
  ChevronLeft,
  ChevronRight,
  Search,
  MoreHorizontal,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { cn } from "../utils";
import { useApp } from "../context/AppContext";
import * as api from "../lib/tauri";
import type { SkillsShSkill, BatchImportResult } from "../lib/tauri";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useNavigate, useSearch } from "@tanstack/react-router";
import type { InstallTab } from "./installSearch";
import { listenOnActiveHost } from "../lib/hostEvents";
import { pickPath } from "../lib/pickPath";
import { filterMarketSkills, paginateMarketSkills } from "../lib/marketSearch";
import { MARKET_SEARCH_STEP, useMarketSearch } from "../hooks/useMarketSearch";
import { useSourceOverflow } from "../hooks/useSourceOverflow";
import { useGitPreview } from "../hooks/useGitPreview";
import { useLocalScan } from "../hooks/useLocalScan";
import { findInstalledByGitUrl as findInstalledSkillByGitUrl } from "../lib/gitUrl";
import { StatusBanner } from "../components/StatusBanner";
import { GitInstallTab } from "../components/GitInstallTab";
import { LocalInstallTab } from "../components/LocalInstallTab";
import { GitPreviewDialog } from "../components/GitPreviewDialog";
import { getErrorMessage, getErrorKind } from "../lib/error";

const MARKET_PAGE_SIZE = 24;

export function InstallSkills() {
  const { t } = useTranslation();
  const { refreshPresets, refreshManagedSkills, managedSkills, openSkillDetailById } = useApp();
  const navigate = useNavigate();
  const { tab: tabParam } = useSearch({ from: "/install" });
  const [activeTab, setActiveTab] = useState<InstallTab>("market");
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
  } = useMarketSearch(activeTab === "market");
  const [installing, setInstalling] = useState<string | null>(null);
  const {
    gitUrl,
    setGitUrl,
    gitLoading,
    gitCancelKey,
    gitPreview,
    gitSelections,
    setGitSelections,
    gitConfirmLoading,
    handleGitPreview,
    handleGitPreviewClose,
    handleGitConfirm,
  } = useGitPreview();
  const {
    scanResult,
    scanLoading,
    localError,
    setLocalError,
    runScan,
    runScanSilent,
  } = useLocalScan(activeTab === "local");
  const [importingPaths, setImportingPaths] = useState<Set<string>>(new Set());
  const [importingAll, setImportingAll] = useState(false);
  const [renameEditing, setRenameEditing] = useState<Record<string, string>>({});
  const marketListRef = useRef<HTMLDivElement | null>(null);
  const {
    sourceOverflowOpen,
    setSourceOverflowOpen,
    sourceOverflowSide,
    setSourceOverflowSide,
    sourceSearch,
    setSourceSearch,
    sourceFocusedIndex,
    setSourceFocusedIndex,
    visibleSourceCount,
    filteredOverflowSources,
    resetSourceOverflowState,
    filterContainerRef,
    allBtnMeasureRef,
    moreBtnMeasureRef,
    sourceMeasureRefs,
    sourceOverflowBtnRef,
    sourceOverflowPanelRef,
    sourceListRef,
  } = useSourceOverflow(sourceOptions);

  const managedSkillsRef = useRef(managedSkills);
  managedSkillsRef.current = managedSkills;

  const goToSkill = useCallback((skillName: string) => {
    // Use ref to get the latest managedSkills after refresh
    const skills = managedSkillsRef.current;
    const skill = skills.find(
      (s) => s.name === skillName || s.source_ref === skillName
    );
    if (skill) {
      openSkillDetailById(skill.id);
    }
    navigate({ to: "/my-skills" });
  }, [navigate, openSkillDetailById]);

  const installedSourceRefs = useMemo(() => {
    const set = new Set<string>();
    for (const skill of managedSkills) {
      if (skill.source_type === "skillssh" && skill.source_ref) {
        set.add(skill.source_ref);
      }
    }
    return set;
  }, [managedSkills]);

  const findInstalledByGitUrl = useCallback(
    (url: string) => findInstalledSkillByGitUrl(managedSkills, url),
    [managedSkills]
  );

  useEffect(() => {
    if (tabParam) {
      setActiveTab(tabParam);
    }
  }, [tabParam]);

  const switchTab = (tab: InstallTab) => {
    setActiveTab(tab);
    navigate({ to: "/install", search: { tab } });
  };

  const warnRejected = (results: PromiseSettledResult<unknown>[], label: string) => {
    for (const r of results) {
      if (r.status === "rejected") console.warn(`${label} failed:`, r.reason);
    }
  };

  const installLocalSource = async (sourcePath: string) => {
    const name = sourcePath.split("/").pop() || sourcePath;
    const toastId = toast.loading(t("install.toast.installing", { name }));
    try {
      await api.installLocal(sourcePath);
    } catch (e) {
      const message = getErrorMessage(e, t("common.error"));
      setLocalError(message);
      toast.error(message, { id: toastId });
      return;
    }
    // Install succeeded — post-install refresh is best-effort and must not
    // surface as an install failure.
    const results = await Promise.allSettled([
      refreshPresets(),
      refreshManagedSkills(),
      runScanSilent(),
    ]);
    warnRejected(results, "post-install refresh");
    toast.success(t("install.toast.success", { name }), {
      id: toastId,
      action: {
        label: t("install.toast.view"),
        onClick: () => goToSkill(name),
      },
    });
  };

  const handleLocalFolderInstall = async () => {
    try {
      const selected = await pickPath({ directory: true });
      if (!selected) return;
      installLocalSource(selected);
    } catch (error: unknown) {
      const message = getErrorMessage(error, t("common.error"));
      setLocalError(message);
      toast.error(message);
    }
  };

  const handleLocalFileInstall = async () => {
    try {
      const selected = await pickPath({ files: ["zip", "skill"], filterName: "Skills" });
      if (!selected) return;
      installLocalSource(selected);
    } catch (error: unknown) {
      const message = getErrorMessage(error, t("common.error"));
      setLocalError(message);
      toast.error(message);
    }
  };

  const handleBatchImportFolder = async () => {
    let unlisten: (() => void) | null = null;
    try {
      const selected = await pickPath({ directory: true });
      if (!selected) return;

      const toastId = toast.loading(t("install.local.batchImporting"));

      unlisten = await listenOnActiveHost<{ current: number; total: number; name: string }>(
        "batch-import-progress",
        (event) => {
          const { current, total, name } = event.payload;
          toast.loading(
            t("install.local.batchProgress", { current, total, name }),
            { id: toastId }
          );
        }
      );

      const result: BatchImportResult = await api.batchImportFolder(selected);

      if (result.errors.length > 0) {
        const previewErrors = result.errors.slice(0, 3).join("; ");
        const remaining = result.errors.length - 3;
        const detail = remaining > 0 ? `${previewErrors}; +${remaining} more` : previewErrors;
        toast.error(
          `${t("install.local.batchErrors", { count: result.errors.length })}: ${detail}`,
          { id: toastId }
        );
      } else if (result.imported === 0) {
        toast.info(
          t("install.local.batchAllSkipped", { skipped: result.skipped }),
          { id: toastId }
        );
      } else {
        toast.success(
          t("install.local.batchSuccess", {
            imported: result.imported,
            skipped: result.skipped,
          }),
          { id: toastId }
        );
      }

      await Promise.all([refreshPresets(), refreshManagedSkills()]);
      runScan();
    } catch (error: unknown) {
      const message = getErrorMessage(error, t("common.error"));
      setLocalError(message);
      toast.error(message);
    } finally {
      unlisten?.();
    }
  };

  const handleInstallSkillssh = async (skill: SkillsShSkill) => {
    const displayName = skill.name || skill.skill_id;
    const cancelKey = `${skill.source}/${skill.skill_id}`;
    setInstalling(skill.id);

    const toastId = toast.loading(t("install.toast.cloning"));
    let unlisten: (() => void) | null = null;

    try {
      unlisten = await listenOnActiveHost<{ skill_id: string; phase: string; detail?: string }>(
        "install-progress",
        (event) => {
          if (event.payload.skill_id !== cancelKey) return;
          if (event.payload.phase === "cloning") {
            const detail = event.payload.detail?.trim();
            const msg = detail
              ? `${t("install.toast.cloning")}\n${detail}`
              : t("install.toast.cloning");
            toast.loading(msg, { id: toastId });
          } else if (event.payload.phase === "installing") {
            toast.loading(t("install.toast.installing", { name: displayName }), { id: toastId });
          }
        }
      );
      await api.installFromSkillssh(skill.source, skill.skill_id);
      await Promise.all([refreshPresets(), refreshManagedSkills()]);
      toast.success(t("install.toast.success", { name: displayName }), {
        id: toastId,
        action: {
          label: t("install.toast.view"),
          onClick: () => goToSkill(displayName),
        },
      });
    } catch (error: unknown) {
      if (getErrorKind(error) === "cancelled") {
        toast.info(t("install.toast.cancelled"), { id: toastId });
      } else {
        toast.error(getErrorMessage(error, t("common.error")), { id: toastId });
      }
    } finally {
      setInstalling(null);
      unlisten?.();
    }
  };

  const handleCancelInstall = (cancelKey: string) => {
    api.cancelInstall(cancelKey).catch(() => {
      // Ignore race: install may have completed before cancel request arrives.
    });
  };

  const handleImportDiscovered = async (sourcePath: string, name: string) => {
    setImportingPaths((prev) => new Set(prev).add(sourcePath));
    try {
      try {
        await api.importExistingSkill(sourcePath, name);
      } catch (error: unknown) {
        toast.error(getErrorMessage(error, t("common.error")));
        return;
      }
      toast.success(t("install.scan.importedOne", { name }));
      const results = await Promise.allSettled([
        refreshPresets(),
        refreshManagedSkills(),
        runScanSilent(),
      ]);
      warnRejected(results, "post-import refresh");
    } finally {
      setImportingPaths((prev) => {
        const next = new Set(prev);
        next.delete(sourcePath);
        return next;
      });
    }
  };

  const handleImportAllDiscovered = async () => {
    setImportingAll(true);
    try {
      try {
        await api.importAllDiscovered();
      } catch (error: unknown) {
        toast.error(getErrorMessage(error, t("common.error")));
        return;
      }
      toast.success(t("install.scan.importedAll"));
      const results = await Promise.allSettled([
        refreshPresets(),
        refreshManagedSkills(),
        runScanSilent(),
      ]);
      warnRejected(results, "post-import refresh");
    } finally {
      setImportingAll(false);
    }
  };

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
    <div className="app-page gap-4">
      <div className="app-page-header border-b-0 pb-0">
        <h1 className="app-page-title mb-4">{t("install.title")}</h1>
        <div className="flex gap-1 border-b border-border-subtle">
          {[
            { id: "market" as const, label: t("install.browseMarket"), icon: Box },
            { id: "local" as const, label: t("install.localInstall"), icon: UploadCloud },
            { id: "git" as const, label: t("install.gitInstall"), icon: Github },
          ].map((tab) => {
            const Icon = tab.icon;
            const isActive = activeTab === tab.id;
            return (
              <button
                key={tab.id}
                onClick={() => switchTab(tab.id)}
                className={cn(
                  "mr-4 flex items-center gap-1.5 border-b-2 px-1 pb-1.5 text-[13px] font-medium transition-colors outline-none",
                  isActive
                    ? "border-accent text-accent"
                    : "border-transparent text-muted hover:text-tertiary"
                )}
              >
                <Icon className="h-3.5 w-3.5" />
                {tab.label}
              </button>
            );
          })}
        </div>
      </div>

      {activeTab === "market" && (
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
                <div className="border-t border-border-subtle pt-2">
                  <div className="flex items-center gap-3">
                    <span className="shrink-0 text-[13px] font-medium text-tertiary">
                      {t("install.filters.source")}
                    </span>
                    <div ref={filterContainerRef} className="relative min-w-0 flex-1">
                      {/* Hidden measurement layer — never visible, keeps all pills in DOM for width queries */}
                      <div className="pointer-events-none invisible absolute left-0 top-0 flex h-0 items-center gap-1.5 overflow-hidden" aria-hidden="true">
                        <button
                          ref={allBtnMeasureRef}
                          tabIndex={-1}
                          className="rounded-full border px-2.5 py-1 text-[13px] font-medium whitespace-nowrap"
                        >
                          {t("install.filters.allSources")}
                        </button>
                        {sourceOptions.map((source, i) => (
                          <button
                            key={source}
                            ref={(el) => { sourceMeasureRefs.current[i] = el; }}
                            tabIndex={-1}
                            className="rounded-full border px-2.5 py-1 text-[13px] font-medium whitespace-nowrap"
                          >
                            @{source}
                          </button>
                        ))}
                        <button
                          ref={moreBtnMeasureRef}
                          tabIndex={-1}
                          className="flex items-center rounded-full border px-2 py-1"
                        >
                          <MoreHorizontal className="h-3.5 w-3.5" />
                        </button>
                      </div>
                      {/* Visible row */}
                      <div className="flex items-center gap-1.5">
                      <button
                        type="button"
                        onClick={() => setMarketSourceFilter("all")}
                        className={cn(
                          "rounded-full border px-2.5 py-1 text-[13px] font-medium whitespace-nowrap transition-colors",
                          marketSourceFilter === "all"
                            ? "border-accent-border bg-accent-bg text-accent-light"
                            : "border-border-subtle bg-background text-muted hover:text-secondary"
                        )}
                      >
                        {t("install.filters.allSources")}
                      </button>
                      {sourceOptions.slice(0, visibleSourceCount).map((source) => (
                        <button
                          key={source}
                          type="button"
                          onClick={() => setMarketSourceFilter(source)}
                          className={cn(
                            "rounded-full border px-2.5 py-1 text-[13px] font-medium whitespace-nowrap transition-colors",
                            marketSourceFilter === source
                              ? "border-accent-border bg-accent-bg text-accent-light"
                              : "border-border-subtle bg-background text-muted hover:text-secondary"
                          )}
                        >
                          @{source}
                        </button>
                      ))}
                      {visibleSourceCount < sourceOptions.length && (
                        <div className="relative">
                          <button
                            ref={sourceOverflowBtnRef}
                            type="button"
                            onClick={() => {
                              if (sourceOverflowBtnRef.current) {
                                const rect = sourceOverflowBtnRef.current.getBoundingClientRect();
                                setSourceOverflowSide(rect.left + 192 > window.innerWidth ? "right" : "left");
                              }
                              setSourceOverflowOpen((v) => {
                                if (v) {
                                  setSourceSearch("");
                                  setSourceFocusedIndex(-1);
                                }
                                return !v;
                              });
                            }}
                            className={cn(
                              "flex items-center rounded-full border px-2 py-1 text-[13px] font-medium transition-colors",
                              sourceOverflowOpen
                                ? "border-accent-border bg-accent-bg text-accent-light"
                                : "border-border-subtle bg-background text-muted hover:text-secondary"
                            )}
                            title={`${sourceOptions.length - visibleSourceCount} more`}
                            aria-expanded={sourceOverflowOpen}
                            aria-haspopup="listbox"
                          >
                            <MoreHorizontal className="h-3.5 w-3.5" />
                          </button>
                          {sourceOverflowOpen && (
                            <div
                              ref={sourceOverflowPanelRef}
                              role="listbox"
                              className={cn(
                                "absolute top-full z-50 mt-1.5 w-48 overflow-hidden rounded-xl border border-border bg-surface shadow-lg",
                                sourceOverflowSide === "left" ? "left-0" : "right-0"
                              )}
                            >
                              <div className="border-b border-border-subtle px-2 py-1.5">
                                <div className="relative">
                                  <Search className="pointer-events-none absolute left-2 top-1/2 h-3 w-3 -translate-y-1/2 text-muted" />
                                  <input
                                    type="text"
                                    value={sourceSearch}
                                    onChange={(e) => {
                                      setSourceSearch(e.target.value);
                                      setSourceFocusedIndex(-1);
                                    }}
                                    onKeyDown={(e) => {
                                      if (e.key === "ArrowDown") {
                                        e.preventDefault();
                                        if (filteredOverflowSources.length === 0) return;
                                        setSourceFocusedIndex((i) =>
                                          Math.min(i + 1, filteredOverflowSources.length - 1)
                                        );
                                      } else if (e.key === "ArrowUp") {
                                        e.preventDefault();
                                        if (filteredOverflowSources.length === 0) return;
                                        setSourceFocusedIndex((i) =>
                                          i <= 0 ? 0 : i - 1
                                        );
                                      } else if (e.key === "Enter" && sourceFocusedIndex >= 0) {
                                        const target = filteredOverflowSources[sourceFocusedIndex];
                                        if (target) {
                                          setMarketSourceFilter(target);
                                          resetSourceOverflowState();
                                        }
                                      } else if (e.key === "Escape") {
                                        resetSourceOverflowState();
                                      }
                                    }}
                                    placeholder={t("common.search")}
                                    className="app-input w-full bg-background py-1 pl-6 pr-2 text-[12px]"
                                    autoFocus
                                    autoCapitalize="none"
                                    autoCorrect="off"
                                    spellCheck={false}
                                  />
                                </div>
                              </div>
                              <div ref={sourceListRef} className="max-h-48 overflow-y-auto scrollbar-hide py-1">
                                {filteredOverflowSources.map((source, idx) => (
                                  <button
                                    key={source}
                                    type="button"
                                    role="option"
                                    aria-selected={marketSourceFilter === source}
                                    onClick={() => {
                                      setMarketSourceFilter(source);
                                      resetSourceOverflowState();
                                    }}
                                    className={cn(
                                      "flex w-full items-center px-3 py-1.5 text-left text-[13px] transition-colors",
                                      idx === sourceFocusedIndex
                                        ? "bg-surface-hover text-primary"
                                        : marketSourceFilter === source
                                          ? "bg-accent-bg text-accent-light"
                                          : "text-secondary hover:bg-surface-hover"
                                    )}
                                  >
                                    @{source}
                                  </button>
                                ))}
                              </div>
                            </div>
                          )}
                        </div>
                      )}
                      </div>
                    </div>
                  </div>
                </div>
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
                                onClick={() => handleCancelInstall(`${skill.source}/${skill.skill_id}`)}
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
                                onClick={() => handleInstallSkillssh(skill)}
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
      )}

      {activeTab === "local" && (
        <LocalInstallTab
          scanResult={scanResult}
          scanLoading={scanLoading}
          localError={localError}
          importingPaths={importingPaths}
          importingAll={importingAll}
          renameEditing={renameEditing}
          setRenameEditing={setRenameEditing}
          onInstallFolder={handleLocalFolderInstall}
          onInstallArchive={handleLocalFileInstall}
          onBatchImport={handleBatchImportFolder}
          onRescan={runScan}
          onImportAll={handleImportAllDiscovered}
          onImportOne={handleImportDiscovered}
        />
      )}

      {activeTab === "git" && (
        <GitInstallTab
          gitUrl={gitUrl}
          gitLoading={gitLoading}
          gitCancelKey={gitCancelKey}
          findInstalledByGitUrl={findInstalledByGitUrl}
          onGitUrlChange={setGitUrl}
          onPreview={handleGitPreview}
          onCancelInstall={handleCancelInstall}
        />
      )}

      {/* Git preview / selection dialog */}
      {gitPreview && (
        <GitPreviewDialog
          selections={gitSelections}
          setSelections={setGitSelections}
          confirmLoading={gitConfirmLoading}
          onClose={handleGitPreviewClose}
          onConfirm={handleGitConfirm}
        />
      )}
    </div>
  );
}
