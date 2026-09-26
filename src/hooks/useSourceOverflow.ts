import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

/**
 * The market's source filter row: how many source pills fit on one line
 * (re-measured on resize), and the "more" menu holding the rest, with its
 * search, click-outside close and keyboard focus.
 */
export function useSourceOverflow(sourceOptions: string[]) {
  const [sourceOverflowOpen, setSourceOverflowOpen] = useState(false);
  const [sourceOverflowSide, setSourceOverflowSide] = useState<"left" | "right">("left");
  const [sourceSearch, setSourceSearch] = useState("");
  const [sourceFocusedIndex, setSourceFocusedIndex] = useState(-1);
  const sourceListRef = useRef<HTMLDivElement | null>(null);
  const [visibleSourceCount, setVisibleSourceCount] = useState<number>(Infinity);
  const sourceOverflowBtnRef = useRef<HTMLButtonElement | null>(null);
  const sourceOverflowPanelRef = useRef<HTMLDivElement | null>(null);
  const filterContainerRef = useRef<HTMLDivElement | null>(null);
  const allBtnMeasureRef = useRef<HTMLButtonElement | null>(null);
  const moreBtnMeasureRef = useRef<HTMLButtonElement | null>(null);
  const sourceMeasureRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const resetSourceOverflowState = useCallback(() => {
    setSourceOverflowOpen(false);
    setSourceSearch("");
    setSourceFocusedIndex(-1);
  }, []);

  useEffect(() => {
    if (!sourceOverflowOpen) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (
        sourceOverflowBtnRef.current?.contains(e.target as Node) ||
        sourceOverflowPanelRef.current?.contains(e.target as Node)
      ) return;
      resetSourceOverflowState();
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [resetSourceOverflowState, sourceOverflowOpen]);

  // Measure how many source pills can fit in one row; reserve room for All + More.
  const computeVisibleCount = useCallback(() => {
    const container = filterContainerRef.current;
    const allBtn = allBtnMeasureRef.current;
    const moreBtn = moreBtnMeasureRef.current;
    if (!container || !allBtn || !moreBtn) {
      setVisibleSourceCount(Infinity);
      return;
    }

    const containerWidth = container.clientWidth;
    if (containerWidth <= 0) {
      setVisibleSourceCount(Infinity);
      return;
    }

    const styles = window.getComputedStyle(container);
    const gap = parseFloat(styles.columnGap || styles.gap || "6") || 6;
    const available = containerWidth - allBtn.offsetWidth - gap - moreBtn.offsetWidth - gap;

    if (available <= 0) {
      setVisibleSourceCount(0);
      return;
    }

    let used = 0;
    let count = 0;
    for (let i = 0; i < sourceOptions.length; i += 1) {
      const el = sourceMeasureRefs.current[i];
      const w = el?.offsetWidth ?? 0;
      if (w <= 0) continue;
      const nextUsed = used + (count > 0 ? gap : 0) + w;
      if (nextUsed <= available) {
        used = nextUsed;
        count += 1;
      } else {
        break;
      }
    }
    setVisibleSourceCount(count);
  }, [sourceOptions]);

  // The set-state-in-effect disables in this hook mark code moved verbatim
  // from InstallSkills, where this lint did not reach it.
  useLayoutEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    computeVisibleCount();
  }, [computeVisibleCount]);

  useEffect(() => {
    const container = filterContainerRef.current;
    if (!container) return;
    const observer = new ResizeObserver(computeVisibleCount);
    observer.observe(container);
    return () => observer.disconnect();
  }, [computeVisibleCount]);

  const overflowSources = sourceOptions.slice(visibleSourceCount);
  const filteredOverflowSources = sourceSearch
    ? overflowSources.filter((s) => s.toLowerCase().includes(sourceSearch.toLowerCase()))
    : overflowSources;

  useEffect(() => {
    if (sourceOverflowOpen && visibleSourceCount >= sourceOptions.length) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      resetSourceOverflowState();
    }
  }, [resetSourceOverflowState, sourceOptions.length, sourceOverflowOpen, visibleSourceCount]);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setSourceFocusedIndex((idx) => {
      if (filteredOverflowSources.length === 0) return -1;
      if (idx < 0) return idx;
      return Math.min(idx, filteredOverflowSources.length - 1);
    });
  }, [filteredOverflowSources.length]);

  // Scroll the focused overflow item into view whenever the index changes
  useEffect(() => {
    if (sourceFocusedIndex < 0) return;
    sourceListRef.current
      ?.children[sourceFocusedIndex]
      ?.scrollIntoView({ block: "nearest" });
  }, [sourceFocusedIndex]);

  return {
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
  };
}

export type SourceOverflow = ReturnType<typeof useSourceOverflow>;
