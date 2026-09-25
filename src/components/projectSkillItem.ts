import type { SkillCreator } from "../lib/skillCreator";
import type { ProjectAgentTarget, ProjectSkill } from "../lib/tauri";
import type { ProjectSkillGroup } from "../lib/projectSkillGroups";

/** Shared by the project's grid card and list row. */
export interface ProjectSkillItemProps {
  skill: ProjectSkillGroup;
  creator: SkillCreator;
  allTags: string[];
  targets: ProjectAgentTarget[];
  supportsSkillToggle: boolean;
  isMultiSelect: boolean;
  isSelected: boolean;
  isUpdatingCenter: boolean;
  isUpdatingProject: boolean;
  isToggling: boolean;
  /** Agent whose toggle is in flight for this skill. */
  pendingAgent: string | null;
  /** The agent that must keep the vendored copy, if any. */
  vendoredLock: { key: string; reason: string } | null;
  onToggleSelect: (skillKey: string) => void;
  onOpenDetail: (skill: ProjectSkillGroup) => void;
  onToggleAgent: (skill: ProjectSkillGroup, agentKey: string, enabled: boolean) => void;
  onUpdateCenter: (skill: ProjectSkillGroup) => void;
  onUpdateProject: (skill: ProjectSkillGroup) => void;
  onToggleSkill: (skill: ProjectSkillGroup) => void;
  onDelete: (skill: ProjectSkillGroup) => void;
}

export function getSyncStatusMeta(t: (key: string) => string, status: ProjectSkill["sync_status"]) {
  switch (status) {
    case "in_sync":
      return {
        label: t("project.syncStatus.inSync"),
        className: "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
      };
    case "project_newer":
      return {
        label: t("project.syncStatus.projectNewer"),
        className: "bg-amber-500/10 text-amber-700 dark:text-amber-300",
      };
    case "center_newer":
      return {
        label: t("project.syncStatus.centerNewer"),
        className: "bg-sky-500/10 text-sky-700 dark:text-sky-300",
      };
    case "diverged":
      return {
        label: t("project.syncStatus.diverged"),
        className: "bg-violet-500/10 text-violet-700 dark:text-violet-300",
      };
    default:
      return {
        label: t("project.syncStatus.projectOnly"),
        className: "bg-surface-hover text-muted",
      };
  }
}
