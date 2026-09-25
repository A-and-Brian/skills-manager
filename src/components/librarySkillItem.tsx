import { Github, Globe, GripVertical, HardDrive } from "lucide-react";
import type { TFunction } from "i18next";
import type { ManagedSkill, ToolInfo } from "../lib/tauri";
import type { SortableHandleProps } from "./SortableItem";

/** Shared by the library's grid card and list row. */
export interface LibrarySkillItemProps {
  skill: ManagedSkill;
  displayName: string;
  enabledInPreset: boolean;
  hasViewedPreset: boolean;
  viewedPresetName: string;
  tools: ToolInfo[];
  allTags: string[];
  isMultiSelect: boolean;
  selected: boolean;
  canDrag: boolean;
  deleting: boolean;
  hasConflict: boolean;
  checking: boolean;
  updating: boolean;
  menuOpen: boolean;
  /** Agent whose deploy toggle is in flight for this skill. */
  pendingTool: string | null;
  onToggleSelect: (skillId: string) => void;
  onOpenDetail: (skillId: string) => void;
  onOpenBackup: () => void;
  onMenuSkillChange: (skillId: string | null) => void;
  onCheckUpdate: (skill: ManagedSkill) => void;
  onRefresh: (skill: ManagedSkill) => void;
  onRelinkSource: (skill: ManagedSkill) => void;
  onDetachSource: (skill: ManagedSkill) => void;
  onDelete: (skill: ManagedSkill) => void;
  onTogglePreset: (skill: ManagedSkill) => void;
  onToggleTarget: (skill: ManagedSkill, tool: string, enabled: boolean) => void;
}

export const sourceIcon = (type: string) => {
  switch (type) {
    case "git":
    case "skillssh":
      return <Github className="h-3 w-3" />;
    case "local":
    case "import":
      return <HardDrive className="h-3 w-3" />;
    default:
      return <Globe className="h-3 w-3" />;
  }
};

export const sourceTypeLabel = (skill: ManagedSkill) =>
  skill.source_type === "skillssh" ? "skills.sh" : skill.source_type;

export const refreshLabel = (skill: ManagedSkill, t: TFunction) =>
  skill.source_type === "local" || skill.source_type === "import"
    ? t("mySkills.updateActions.reimport")
    : t("mySkills.updateActions.update");

export const statusBadge = (skill: ManagedSkill, t: TFunction) => {
  if (skill.update_status === "update_available") {
    return {
      label: "Update",
      className: "bg-amber-500/12 text-amber-600 dark:text-amber-400",
    };
  }
  if (skill.update_status === "source_missing") {
    return {
      label: t("mySkills.updateStatus.sourceMissing"),
      className: "bg-red-500/10 text-red-600 dark:text-red-300",
    };
  }
  if (skill.update_status === "error") {
    return {
      label: t("mySkills.updateStatus.error"),
      className: "bg-red-500/10 text-red-600 dark:text-red-300",
    };
  }
  return null;
};

export const renderDragHandle = (handleProps: SortableHandleProps, title: string) => (
  <div
    {...handleProps}
    title={title}
    className="absolute inset-0 flex cursor-grab items-center justify-center rounded text-faint opacity-0 transition-opacity hover:text-muted group-hover:opacity-100 active:cursor-grabbing"
  >
    <GripVertical className="h-4 w-4" />
  </div>
);
