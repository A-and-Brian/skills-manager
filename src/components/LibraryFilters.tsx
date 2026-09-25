import { useEffect, useRef, useState, type MouseEvent as ReactMouseEvent, type ReactNode } from "react";
import { ListFilter, Search, Square, SquareCheck, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../utils";

export interface FilterOption {
  key: string;
  /** Plain text, used for the option search and the chip summary. */
  label: string;
  /** Skills this option would match (see `libraryFilterCounts`). */
  count: number;
  icon?: ReactNode;
  /** Rendered instead of icon + label, e.g. a creator avatar with its name. */
  badge?: ReactNode;
  title?: string;
  onContextMenu?: (e: ReactMouseEvent) => void;
}

export interface FilterCategory {
  key: string;
  label: string;
  options: FilterOption[];
  selected: ReadonlySet<string>;
  onToggle: (key: string) => void;
  onClear: () => void;
  /** Not offered in the popover (nothing to pick), but an active selection still gets its chip. */
  hidden?: boolean;
}

const PANEL_WIDTH = 520;
const WINDOW_MARGIN = 16;

interface PopoverProps {
  categories: FilterCategory[];
  onClearAll: () => void;
  /** Keep the panel open while a layer above it (tag menu, dialog) owns clicks and Escape. */
  holdOpen?: boolean;
}

/** Toolbar "Filter" button with a category / checkbox-list panel. */
export function LibraryFilterPopover({ categories, onClearAll, holdOpen = false }: PopoverProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const [optionSearch, setOptionSearch] = useState("");
  const [shift, setShift] = useState(0);
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  const visible = categories.filter((category) => !category.hidden);
  const active = visible.find((category) => category.key === activeKey) ?? visible[0];
  const selectedTotal = categories.reduce((sum, category) => sum + category.selected.size, 0);
  const needle = optionSearch.trim().toLowerCase();
  const shown = active?.options.filter((option) => option.label.toLowerCase().includes(needle)) ?? [];

  const toggleOpen = () => {
    if (open) {
      setOpen(false);
      setOptionSearch("");
      return;
    }
    // Slide the panel left when it would run past the window's right edge.
    const left = containerRef.current?.getBoundingClientRect().left ?? 0;
    const width = Math.min(PANEL_WIDTH, window.innerWidth - 2 * WINDOW_MARGIN);
    setShift(Math.min(0, window.innerWidth - WINDOW_MARGIN - width - left));
    setOpen(true);
  };

  useEffect(() => {
    if (!open || holdOpen) return;
    const close = () => {
      setOpen(false);
      setOptionSearch("");
    };
    const handlePointer = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) close();
    };
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      // Consumed here so multi-select mode behind the panel stays on.
      e.stopPropagation();
      close();
      triggerRef.current?.focus();
    };
    document.addEventListener("mousedown", handlePointer);
    document.addEventListener("keydown", handleEscape);
    return () => {
      document.removeEventListener("mousedown", handlePointer);
      document.removeEventListener("keydown", handleEscape);
    };
  }, [open, holdOpen]);

  return (
    <div ref={containerRef} className="relative shrink-0">
      <button
        ref={triggerRef}
        type="button"
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={toggleOpen}
        className={cn(
          "app-toolbar-button app-toolbar-button-secondary focus-visible:ring-2 focus-visible:ring-border",
          selectedTotal > 0 && "border-accent-border text-accent-light hover:text-accent-light",
          open && "bg-surface-hover"
        )}
      >
        <ListFilter className="h-3.5 w-3.5" />
        {selectedTotal > 0
          ? t("mySkills.filterPopover.triggerCount", { count: selectedTotal })
          : t("mySkills.filterPopover.trigger")}
      </button>

      {open && active && (
        <div
          role="dialog"
          aria-label={t("mySkills.filterPopover.title")}
          style={{ left: shift }}
          className="absolute top-full z-40 mt-1 flex w-[min(520px,calc(100vw-2rem))] overflow-hidden rounded-lg border border-border bg-surface shadow-lg"
        >
          <div className="w-40 shrink-0 space-y-0.5 border-r border-border-subtle p-1">
            {visible.map((category) => (
              <button
                key={category.key}
                type="button"
                aria-pressed={category === active}
                onClick={() => {
                  setActiveKey(category.key);
                  setOptionSearch("");
                }}
                className={cn(
                  "flex w-full items-center justify-between gap-2 rounded-md px-2 py-1.5 text-left text-[13px] outline-none transition-colors focus-visible:ring-2 focus-visible:ring-border",
                  category === active
                    ? "bg-surface-active text-secondary"
                    : "text-muted hover:bg-surface-hover hover:text-secondary"
                )}
              >
                <span className="truncate">{category.label}</span>
                {category.selected.size > 0 && (
                  <span className="shrink-0 rounded-full bg-accent px-1.5 text-[11px] font-medium tabular-nums text-white">
                    {category.selected.size}
                  </span>
                )}
              </button>
            ))}
          </div>

          <div className="min-w-0 flex-1 p-2">
            <div className="flex h-6 items-center justify-between gap-2 px-1 text-[12px]">
              <span className="font-medium text-secondary">{active.label}</span>
              <span className="flex items-center gap-3 text-muted">
                {active.selected.size > 0 && (
                  <button type="button" onClick={active.onClear} className="outline-none hover:text-secondary focus-visible:underline">
                    {t("mySkills.filterPopover.clear")}
                  </button>
                )}
                {selectedTotal > 0 && (
                  <button type="button" onClick={onClearAll} className="outline-none hover:text-secondary focus-visible:underline">
                    {t("mySkills.filterPopover.clearAll")}
                  </button>
                )}
              </span>
            </div>

            <div className="relative mt-1">
              <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted" />
              <input
                type="text"
                autoFocus
                value={optionSearch}
                onChange={(e) => setOptionSearch(e.target.value)}
                placeholder={t("mySkills.filterPopover.search")}
                className="app-input h-8 w-full pl-8 text-[12px]"
                autoCapitalize="none"
                autoCorrect="off"
                spellCheck={false}
              />
            </div>

            <div className="mt-1 max-h-72 overflow-y-auto">
              {shown.length === 0 ? (
                <p className="px-2 py-3 text-[12px] text-faint">{t("mySkills.filterPopover.noOptions")}</p>
              ) : (
                shown.map((option) => {
                  const checked = active.selected.has(option.key);
                  return (
                    <button
                      key={option.key}
                      type="button"
                      role="checkbox"
                      aria-checked={checked}
                      onClick={() => active.onToggle(option.key)}
                      onContextMenu={option.onContextMenu}
                      title={option.title}
                      className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] text-secondary outline-none transition-colors hover:bg-surface-hover focus-visible:bg-surface-hover"
                    >
                      {checked
                        ? <SquareCheck className="h-3.5 w-3.5 shrink-0 text-accent" />
                        : <Square className="h-3.5 w-3.5 shrink-0 text-faint" />}
                      {option.badge ?? (
                        <>
                          {option.icon}
                          <span className="min-w-0 truncate">{option.label}</span>
                        </>
                      )}
                      <span
                        className={cn(
                          "ml-auto shrink-0 pl-2 text-[11px] tabular-nums",
                          option.count === 0 ? "text-faint opacity-60" : "text-muted"
                        )}
                      >
                        {option.count}
                      </span>
                    </button>
                  );
                })
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

interface ChipsProps {
  categories: FilterCategory[];
  onClearAll: () => void;
  className?: string;
}

/** One chip per category with a selection; renders nothing when no filter is on. */
export function LibraryFilterChips({ categories, onClearAll, className }: ChipsProps) {
  const { t } = useTranslation();
  const active = categories.filter((category) => category.selected.size > 0);
  if (active.length === 0) return null;

  return (
    <div className={cn("flex flex-wrap items-center gap-1.5", className)}>
      {active.map((category) => {
        const labels = [...category.selected].map(
          (key) => category.options.find((option) => option.key === key)?.label ?? key
        );
        const summary = labels.slice(0, 2).join(", ") + (labels.length > 2 ? ` +${labels.length - 2}` : "");
        const removeLabel = t("mySkills.filterPopover.remove", { category: category.label });
        return (
          <span
            key={category.key}
            className="inline-flex max-w-full items-center gap-1 rounded-full border border-border-subtle bg-surface py-0.5 pl-2.5 pr-1 text-[12px] text-muted"
          >
            <span className="truncate">
              {category.label}: <span className="font-medium text-secondary">{summary}</span>
            </span>
            <button
              type="button"
              onClick={category.onClear}
              aria-label={removeLabel}
              title={removeLabel}
              className="shrink-0 rounded-full p-0.5 outline-none transition-colors hover:bg-surface-hover hover:text-secondary focus-visible:ring-2 focus-visible:ring-border"
            >
              <X className="h-3 w-3" />
            </button>
          </span>
        );
      })}
      <button
        type="button"
        onClick={onClearAll}
        className="rounded px-1.5 text-[12px] font-medium text-muted underline-offset-2 outline-none transition-colors hover:text-secondary hover:underline focus-visible:underline"
      >
        {t("mySkills.filterPopover.clear")}
      </button>
    </div>
  );
}
