import { Download, FileText, Loader2, RotateCcw, Square, SquareCheck, Trash2, Upload } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../utils";
import { ToggleSwitch } from "./ToggleSwitch";
import { ProjectAgentDots } from "./ProjectAgentDots";
import { CreatorBadge } from "./CreatorBadge";
import { getSyncStatusMeta, type ProjectSkillItemProps } from "./projectSkillItem";
import { getTagColor } from "../lib/skillTags";
import { getAssignedAgents, isCenterUpdatable, isProjectUpdatable } from "../lib/projectSkillGroups";

/** A skill in the project grid view. */
export function ProjectSkillCard({
  skill,
  creator,
  allTags,
  targets,
  supportsSkillToggle,
  isMultiSelect,
  isSelected,
  isUpdatingCenter,
  isUpdatingProject,
  isToggling,
  pendingAgent,
  vendoredLock,
  onToggleSelect,
  onOpenDetail,
  onToggleAgent,
  onUpdateCenter,
  onUpdateProject,
  onToggleSkill,
  onDelete,
}: ProjectSkillItemProps) {
  const { t } = useTranslation();
  const canUpdateCenter = isCenterUpdatable(skill.status);
  const canUpdateProject = isProjectUpdatable(skill.status);
  const statusMeta = getSyncStatusMeta(t, skill.status);
  const assignedAgents = getAssignedAgents(skill.variants);

  return (
    <div
      className={cn(
        "app-panel group relative flex h-full cursor-pointer flex-col overflow-hidden shadow-card transition-all hover:-translate-y-px hover:border-border hover:shadow-card-hover",
        isMultiSelect && isSelected && "ring-1 ring-accent border-accent/40"
      )}
      onClick={() =>
        isMultiSelect ? onToggleSelect(skill.id) : onOpenDetail(skill)
      }
    >
      <div className="flex items-center gap-2.5 px-3.5 pt-3 pb-1.5">
        {/* Fixed slot: status dot, or the checkbox in multi-select */}
        <div className="flex h-4 w-4 shrink-0 items-center justify-center">
          {isMultiSelect ? (
            isSelected
              ? <SquareCheck className="h-3.5 w-3.5 text-accent" />
              : <Square className="h-3.5 w-3.5 text-faint" />
          ) : (
            <span
              className={cn(
                "h-2 w-2 rounded-full",
                skill.enabledCount === skill.totalCount
                  ? "bg-accent-light shadow-[0_0_0_3px_var(--color-accent-bg)]"
                  : skill.enabledCount > 0
                    ? "bg-amber-500 shadow-[0_0_0_3px_rgba(245,158,11,0.15)]"
                    : "bg-surface-active"
              )}
              title={`${skill.enabledCount}/${skill.totalCount}`}
            />
          )}
        </div>
        <h3
          className="flex-1 truncate text-[14px] font-semibold text-primary"
          title={skill.name}
        >
          {skill.name}
        </h3>
        {skill.files.length > 0 && (
          <span className="flex items-center gap-1 text-[12px] text-faint shrink-0">
            <FileText className="w-3 h-3" />
            {skill.files.length}
          </span>
        )}
      </div>

      <div className="px-3.5 pb-3">
        <p className="text-[13px] leading-[18px] text-muted truncate">
          {skill.description || "\u2014"}
        </p>
        {skill.tags.length > 0 && (
          <div className="mt-2 flex flex-wrap items-center gap-1">
            {skill.tags.map((tag) => (
              <span
                key={tag}
                className={cn(
                  "inline-flex items-center rounded-full px-2 py-0.5 text-[11px] font-medium",
                  getTagColor(tag, allTags)
                )}
              >
                {tag}
              </span>
            ))}
          </div>
        )}
      </div>

      <div className="mt-auto flex items-center justify-between gap-2 border-t border-border-faint px-3.5 py-2.5">
        <div className="flex min-w-0 items-center gap-1.5">
          <span className={cn("rounded-full px-2 py-0.5 text-[12px] font-medium", statusMeta.className)}>
            {statusMeta.label}
          </span>
          {skill.enabledCount === 0 && (
            <span className="rounded-full bg-red-500/10 px-2 py-0.5 text-[12px] font-medium text-red-600 dark:text-red-300">
              {t("project.disabled")}
            </span>
          )}
          <CreatorBadge creator={creator} hideLocal />
        </div>
        {!isMultiSelect && (
          <div className="flex items-center gap-1.5 shrink-0">
            <ProjectAgentDots
              assignedAgents={assignedAgents}
              targets={targets}
              limit={4}
              size="sm"
              onToggle={(agentKey, enabled) => onToggleAgent(skill, agentKey, enabled)}
              locked={vendoredLock}
              pendingKey={pendingAgent}
            />
            {canUpdateCenter && (
              <button
                onClick={(e) => { e.stopPropagation(); onUpdateCenter(skill); }}
                disabled={isUpdatingCenter || isUpdatingProject}
                className="rounded px-2 py-1 text-[13px] font-medium text-muted transition-colors outline-none hover:bg-surface-hover hover:text-secondary disabled:opacity-50"
                title={t("project.updateCenter")}
              >
                {isUpdatingCenter ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Upload className="h-3.5 w-3.5" />
                )}
              </button>
            )}
            {canUpdateProject && (
              <button
                onClick={(e) => { e.stopPropagation(); onUpdateProject(skill); }}
                disabled={isUpdatingCenter || isUpdatingProject}
                className="rounded px-2 py-1 text-[13px] font-medium text-muted transition-colors outline-none hover:bg-surface-hover hover:text-secondary disabled:opacity-50"
                title={
                  skill.status === "project_newer"
                    ? t("project.resetFromCenter")
                    : t("project.updateProject")
                }
              >
                {isUpdatingProject ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : skill.status === "project_newer" ? (
                  <RotateCcw className="h-3.5 w-3.5" />
                ) : (
                  <Download className="h-3.5 w-3.5" />
                )}
              </button>
            )}
            {supportsSkillToggle ? (
              <ToggleSwitch
                checked={skill.enabledCount === skill.totalCount}
                loading={isToggling}
                onChange={() => onToggleSkill(skill)}
                title={
                  skill.enabledCount === skill.totalCount
                    ? t("project.enabled")
                    : t("project.enableSkill")
                }
              />
            ) : null}
            <button
              onClick={(e) => { e.stopPropagation(); onDelete(skill); }}
              className="rounded px-2 py-1 text-muted transition-colors outline-none hover:bg-red-500/10 hover:text-red-500"
              title={t("project.deleteSkill")}
            >
              <Trash2 className="h-3.5 w-3.5" />
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
