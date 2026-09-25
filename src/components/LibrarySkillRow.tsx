import { Loader2, RefreshCw, RotateCcw, Square, SquareCheck, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../utils";
import { SortableItem } from "./SortableItem";
import { CardActionMenu } from "./CardActionMenu";
import { ToggleSwitch } from "./ToggleSwitch";
import { CreatorBadge } from "./CreatorBadge";
import { SyncDots } from "./SyncDots";
import {
  refreshLabel,
  renderDragHandle,
  sourceIcon,
  sourceTypeLabel,
  statusBadge,
  type LibrarySkillItemProps,
} from "./librarySkillItem";
import { getTagColor } from "../lib/skillTags";
import { canRefreshSkill } from "../lib/librarySkillQuery";
import { skillCreator } from "../lib/skillCreator";

/** A skill in the library's list view. */
export function LibrarySkillRow({
  skill,
  displayName,
  enabledInPreset,
  hasViewedPreset,
  viewedPresetName,
  tools,
  allTags,
  isMultiSelect,
  selected,
  canDrag,
  deleting,
  hasConflict,
  checking,
  updating,
  menuOpen,
  pendingTool,
  onToggleSelect,
  onOpenDetail,
  onOpenBackup,
  onMenuSkillChange,
  onCheckUpdate,
  onRefresh,
  onRelinkSource,
  onDetachSource,
  onDelete,
  onTogglePreset,
  onToggleTarget,
}: LibrarySkillItemProps) {
  const { t } = useTranslation();
  const badge = statusBadge(skill, t);
  const hasUpdate =
    skill.update_status === "update_available" && canRefreshSkill(skill);
  const isMissingLocalSource =
    skill.update_status === "source_missing"
    && (skill.source_type === "local" || skill.source_type === "import");
  const creator = skillCreator(skill);

  return (
    <SortableItem
      id={skill.id}
      disabled={!canDrag}
      className={menuOpen ? "relative z-30" : undefined}
    >
    {(handleProps) => (
    <div
      className={cn(
        "app-panel group relative flex cursor-pointer items-center gap-3.5 rounded-xl border-transparent px-3.5 py-3 transition-all hover:border-border hover:bg-surface-hover",
        isMultiSelect && selected && "ring-1 ring-accent border-accent/40"
      )}
      onClick={() =>
        isMultiSelect ? onToggleSelect(skill.id) : onOpenDetail(skill.id)
      }
    >
      {deleting && (
        <div className="absolute inset-0 z-20 flex items-center justify-center rounded-xl bg-surface/70 backdrop-blur-[1px]">
          <Loader2 className="h-5 w-5 animate-spin text-muted" />
        </div>
      )}
      {/* Same fixed slot as the grid card: status dot / drag handle / checkbox */}
      <div className="relative flex h-4 w-4 shrink-0 items-center justify-center">
        {isMultiSelect ? (
          selected
            ? <SquareCheck className="h-3.5 w-3.5 text-accent" />
            : <Square className="h-3.5 w-3.5 text-faint" />
        ) : (
          <>
            <span
              className={cn(
                "h-2 w-2 rounded-full transition-opacity",
                canDrag && "group-hover:opacity-0",
                enabledInPreset
                  ? "bg-accent-light shadow-[0_0_0_3px_var(--color-accent-bg)]"
                  : "bg-surface-active"
              )}
              title={enabledInPreset ? t("mySkills.enabledButton") : t("mySkills.notInPreset")}
            />
            {canDrag && renderDragHandle(handleProps, t("mySkills.dragToReorder"))}
          </>
        )}
      </div>

      <h3
        className="w-[180px] shrink-0 truncate text-[14px] font-semibold text-secondary group-hover:text-primary"
        title={displayName}
      >
        {displayName}
      </h3>

      <p className="min-w-0 flex-1 truncate text-[13px] text-muted">
        {skill.description || "—"}
      </p>

      <div className="flex shrink-0 items-center gap-1.5">
        {skill.tags.map((tag) => (
          <span
            key={tag}
            className={cn(
              "inline-flex items-center rounded-full px-1.5 py-0.5 text-[11px] font-medium",
              getTagColor(tag, allTags)
            )}
          >
            {tag}
          </span>
        ))}
      </div>

      <div className="flex shrink-0 items-center gap-2.5">
        {hasConflict && (
          <button
            onClick={(e) => { e.stopPropagation(); onOpenBackup(); }}
            className="rounded-full bg-amber-500/12 px-2 py-0.5 text-[12px] font-medium text-amber-600 transition-colors hover:bg-amber-500/20 dark:text-amber-400"
            title={t("mySkills.needsAttentionHint")}
          >
            {t("mySkills.needsAttention")}
          </button>
        )}
        {hasUpdate && !isMultiSelect ? (
          <button
            onClick={(e) => { e.stopPropagation(); onRefresh(skill); }}
            disabled={updating}
            className="inline-flex shrink-0 items-center gap-1 rounded-full bg-amber-500/12 px-2 py-0.5 text-[11px] font-medium text-amber-600 outline-none transition-colors hover:bg-amber-500/20 disabled:opacity-50 dark:text-amber-400"
            title={refreshLabel(skill, t)}
          >
            <RotateCcw className={cn("h-2.5 w-2.5", updating && "animate-spin")} />
            {t("mySkills.updateActions.update")}
          </button>
        ) : badge && (
          <span
            className={cn(
              "rounded-full px-2 py-0.5 text-[12px] font-medium",
              badge.className
            )}
          >
            {badge.label}
          </span>
        )}
        <SyncDots
          skill={skill}
          tools={tools}
          limit={6}
          size="sm"
          onToggle={
            isMultiSelect
              ? undefined
              : (tool, enabled) => onToggleTarget(skill, tool, enabled)
          }
          pendingKey={pendingTool}
        />
        <CreatorBadge creator={creator} size="md" hideLocal className="max-w-[160px]" />
        <span className="inline-flex items-center gap-1 text-[13px] text-muted">
          {sourceIcon(skill.source_type)}
          {sourceTypeLabel(skill)}
        </span>
        {enabledInPreset && (
          <span className="text-[13px] font-medium text-amber-600 dark:text-amber-400/80">
            {viewedPresetName}
          </span>
        )}
      </div>

      {isMissingLocalSource && !isMultiSelect && (
        <div className="flex shrink-0 items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100">
          <button
            onClick={(e) => { e.stopPropagation(); onRelinkSource(skill); }}
            disabled={updating}
            className="rounded px-2 py-0.5 text-[13px] font-medium text-secondary transition-colors hover:bg-surface-hover disabled:opacity-50"
          >
            {t("mySkills.updateActions.relink")}
          </button>
          <button
            onClick={(e) => { e.stopPropagation(); onDetachSource(skill); }}
            disabled={updating}
            className="rounded px-2 py-0.5 text-[13px] font-medium text-muted transition-colors hover:bg-surface-hover hover:text-secondary disabled:opacity-50"
          >
            {t("mySkills.updateActions.detachSource")}
          </button>
        </div>
      )}

      {!isMultiSelect && (
        <div className="flex shrink-0 items-center gap-2">
          <CardActionMenu
            label={t("mySkills.moreActions")}
            onOpenChange={(open) => onMenuSkillChange(open ? skill.id : null)}
            className={cn(
              "transition-opacity",
              menuOpen
                ? "opacity-100"
                : "opacity-0 group-hover:opacity-100"
            )}
            actions={[
              {
                key: "check",
                label: t("mySkills.updateActions.check"),
                icon: <RefreshCw className={cn("h-3.5 w-3.5", checking && "animate-spin")} />,
                disabled: checking,
                onSelect: () => onCheckUpdate(skill),
              },
              ...(canRefreshSkill(skill)
                ? [{
                    key: "refresh",
                    label: refreshLabel(skill, t),
                    icon: <RotateCcw className={cn("h-3.5 w-3.5", updating && "animate-spin")} />,
                    disabled: updating,
                    onSelect: () => onRefresh(skill),
                  }]
                : []),
              {
                key: "delete",
                label: t("common.delete"),
                icon: <Trash2 className="h-3.5 w-3.5" />,
                danger: true,
                onSelect: () => onDelete(skill),
              },
            ]}
          />
          <ToggleSwitch
            checked={enabledInPreset}
            disabled={!hasViewedPreset}
            onChange={() => onTogglePreset(skill)}
            title={enabledInPreset ? t("mySkills.enabledButton") : t("mySkills.enable")}
          />
        </div>
      )}
    </div>
    )}
    </SortableItem>
  );
}
