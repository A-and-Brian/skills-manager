import type { ProjectSkill } from "./tauri";

/** The folder a copy-mode project vendors its skills into. */
export const VENDORED_SKILLS_DIR = ".agents/skills";

/** One logical skill in a project: every agent's copy of the same relative path. */
export interface ProjectSkillGroup {
  id: string;
  name: string;
  dir_name: string;
  relative_path: string;
  description: string | null;
  files: string[];
  variants: ProjectSkill[];
  /**
   * The copies an action has to touch. Links into `.agents/skills` follow
   * their vendored copy, so they are left out.
   */
  effectiveVariants: ProjectSkill[];
  /** The vendored copy other agents' links read, whatever the deploy mode. */
  vendoredVariant: ProjectSkill | null;
  enabledCount: number;
  totalCount: number;
  primaryVariant: ProjectSkill;
  status: ProjectSkill["sync_status"];
  tags: string[];
  centerSkillIds: string[];
  agentsOverridden: boolean;
}

const STATUS_PRIORITY: ProjectSkill["sync_status"][] = [
  "diverged",
  "project_newer",
  "center_newer",
  "project_only",
  "in_sync",
];

function getGroupStatus(variants: ProjectSkill[]): ProjectSkill["sync_status"] {
  return STATUS_PRIORITY.find((status) => variants.some((variant) => variant.sync_status === status))
    ?? "project_only";
}

const byName = (a: string, b: string) => a.localeCompare(b);

/** Whether an agent's project folder is `.agents/skills` itself; `.` segments do not count. */
export function isVendoredSkillsDir(relativeSkillsDir: string): boolean {
  return relativeSkillsDir.split(/[\\/]/).filter((part) => part && part !== ".").join("/")
    === VENDORED_SKILLS_DIR;
}

/**
 * Group a project's scanned copies by relative path. The primary variant is
 * the first one that is not a link into `.agents/skills`, and those links are
 * excluded from the effective variants; a group made only of such links still
 * shows, with its first link standing in for it. Decided from what is on disk,
 * not the deploy mode, so vendored skills stay vendored after switching back
 * to linking.
 */
export function groupProjectSkills(skills: ProjectSkill[]): ProjectSkillGroup[] {
  const buckets = new Map<string, ProjectSkill[]>();
  for (const skill of skills) {
    const key = skill.relative_path.toLowerCase();
    buckets.set(key, [...(buckets.get(key) ?? []), skill]);
  }

  return Array.from(buckets, ([id, found]) => {
    const variants = [...found].sort((a, b) => byName(a.agent_display_name, b.agent_display_name));
    const owned = variants.filter((variant) => !variant.alias_of);
    const effectiveVariants = owned.length > 0 ? owned : variants;
    const first = found[0];
    return {
      id,
      name: first.name,
      dir_name: first.dir_name,
      relative_path: first.relative_path,
      description: found.find((variant) => variant.description)?.description ?? first.description,
      files: Array.from(new Set(found.flatMap((variant) => variant.files))).sort(),
      variants,
      effectiveVariants,
      vendoredVariant: variants.find((variant) => variant.vendored) ?? null,
      enabledCount: found.filter((variant) => variant.enabled).length,
      totalCount: found.length,
      primaryVariant: effectiveVariants[0],
      status: getGroupStatus(found),
      tags: Array.from(new Set(found.flatMap((variant) => variant.tags))).sort(byName),
      centerSkillIds: Array.from(
        new Set(found.flatMap((variant) => (variant.center_skill_id ? [variant.center_skill_id] : [])))
      ).sort(byName),
      agentsOverridden: found.some((variant) => variant.agents_overridden),
    };
  }).sort((a, b) => byName(a.name.toLowerCase(), b.name.toLowerCase()));
}
