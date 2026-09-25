import type { ManagedSkill } from "./tauri";
import { UNTAGGED_FILTER } from "./skillTags";
import { matchesTagFilter } from "./tagFilter";
import { creatorKey, creatorLabel, creatorName, LOCAL_CREATOR, skillCreator, type SkillCreator } from "./skillCreator";

/**
 * Pure filter / sort / group logic for the Library view. Kept out of the React
 * component so it can be unit-tested against plain skill records.
 */

export type LibraryGroupBy = "none" | "tag" | "source" | "agent" | "creator";
export type LibrarySortBy = "name" | "updated" | "added" | "update_status";
export type LibraryUpdateFilter = "update_available" | "error" | "up_to_date" | "local";

export const LIBRARY_GROUP_BY_OPTIONS: readonly LibraryGroupBy[] = ["none", "tag", "source", "agent", "creator"];
export const LIBRARY_SORT_BY_OPTIONS: readonly LibrarySortBy[] = ["name", "updated", "added", "update_status"];
export const LIBRARY_UPDATE_FILTERS: readonly LibraryUpdateFilter[] = ["update_available", "error", "up_to_date", "local"];

/** Agent-filter value and group key for skills deployed nowhere. */
export const NOT_DEPLOYED = "__not_deployed__";
/** Tag group key for skills without tags (filtering uses UNTAGGED_FILTER from skillTags). */
export const NO_TAG_GROUP = "__untagged__";

export interface LibraryQuery {
  /** Already lower-cased search text; empty string matches everything. */
  search: string;
  sources: ReadonlySet<string>;
  /** Tag names, or UNTAGGED_FILTER. */
  tags: ReadonlySet<string>;
  /** Agent keys, or NOT_DEPLOYED. */
  agents: ReadonlySet<string>;
  /** Creator keys from `creatorKey`, or LOCAL_CREATOR. */
  creators: ReadonlySet<string>;
  updates: ReadonlySet<LibraryUpdateFilter>;
  sortBy: LibrarySortBy;
  groupBy: LibraryGroupBy;
  /** Present when a preset is viewed: enabled skills lead, in preset order. */
  preset?: { id: string; order: readonly string[]; mode: "all" | "enabled" | "available" };
}

export interface SkillGroup {
  /** Raw group value (tag name, source_type, agent key, creator key, or a sentinel). */
  key: string;
  skills: ManagedSkill[];
}

/** Which update pill a skill answers to. `null` means none (unknown / checking). */
export function updateFilterOf(skill: ManagedSkill): LibraryUpdateFilter | null {
  // A local/import skill whose source changed or went missing is checked like
  // any other, so it must answer to those pills before its own bucket.
  switch (skill.update_status) {
    case "update_available":
      return "update_available";
    case "error":
    case "source_missing":
      return "error";
  }
  if (skill.source_type === "local" || skill.source_type === "import") return "local";
  return skill.update_status === "up_to_date" ? "up_to_date" : null;
}

function agentKeysOf(skill: ManagedSkill): string[] {
  const keys = [...new Set(skill.targets.map((target) => target.tool))];
  return keys.length > 0 ? keys : [NOT_DEPLOYED];
}

export interface CreatorOption {
  key: string;
  creator: SkillCreator;
  count: number;
}

/** Every creator in the list, most skills first, with "Local" last. */
export function libraryCreators(skills: readonly ManagedSkill[]): CreatorOption[] {
  const options = new Map<string, CreatorOption>();
  for (const skill of skills) {
    const creator = skillCreator(skill);
    const key = creatorKey(creator);
    const option = options.get(key);
    if (option) option.count += 1;
    else options.set(key, { key, creator, count: 1 });
  }
  const isLocal = (option: CreatorOption) => (option.key === LOCAL_CREATOR ? 1 : 0);
  return [...options.values()].sort(
    (a, b) =>
      isLocal(a) - isLocal(b)
      || b.count - a.count
      || creatorName(a.creator).localeCompare(creatorName(b.creator))
  );
}

export function filterLibrarySkills(
  skills: readonly ManagedSkill[],
  q: LibraryQuery,
  displayNameOf: (skill: ManagedSkill) => string
): ManagedSkill[] {
  return skills.filter((skill) => {
    if (q.search) {
      const haystack = [skill.name, displayNameOf(skill), skill.description ?? "", creatorLabel(skillCreator(skill))];
      if (!haystack.some((text) => text.toLowerCase().includes(q.search))) return false;
    }
    if (q.sources.size > 0 && !q.sources.has(skill.source_type)) return false;
    if (!matchesTagFilter(skill.tags, q.tags)) return false;
    if (q.agents.size > 0 && !agentKeysOf(skill).some((key) => q.agents.has(key))) return false;
    if (q.creators.size > 0 && !q.creators.has(creatorKey(skillCreator(skill)))) return false;
    if (q.updates.size > 0) {
      const bucket = updateFilterOf(skill);
      if (!bucket || !q.updates.has(bucket)) return false;
    }
    if (q.preset && q.preset.mode !== "all") {
      const enabled = skill.preset_ids.includes(q.preset.id);
      return q.preset.mode === "enabled" ? enabled : !enabled;
    }
    return true;
  });
}

export interface LibraryFilterCounts {
  sources: Map<string, number>;
  /** Tag names, plus the untagged sentinel. */
  tags: Map<string, number>;
  /** Agent keys, plus NOT_DEPLOYED. */
  agents: Map<string, number>;
  creators: Map<string, number>;
  updates: Map<LibraryUpdateFilter, number>;
}

/**
 * How many skills each filter option would match. Every category is counted
 * with search, preset mode and the other categories applied but its own
 * selection ignored, so a number reads as "what ticking this adds".
 */
export function libraryFilterCounts(
  skills: readonly ManagedSkill[],
  q: LibraryQuery,
  displayNameOf: (skill: ManagedSkill) => string
): LibraryFilterCounts {
  const tally = <K>(without: Partial<LibraryQuery>, keysOf: (skill: ManagedSkill) => readonly (K | null)[]) => {
    const counts = new Map<K, number>();
    for (const skill of filterLibrarySkills(skills, { ...q, ...without }, displayNameOf)) {
      for (const key of keysOf(skill)) {
        if (key !== null) counts.set(key, (counts.get(key) ?? 0) + 1);
      }
    }
    return counts;
  };
  return {
    sources: tally({ sources: new Set() }, (skill) => [skill.source_type]),
    tags: tally({ tags: new Set() }, (skill) => (skill.tags.length > 0 ? skill.tags : [UNTAGGED_FILTER])),
    agents: tally({ agents: new Set() }, agentKeysOf),
    creators: tally({ creators: new Set() }, (skill) => [creatorKey(skillCreator(skill))]),
    updates: tally({ updates: new Set() }, (skill) => [updateFilterOf(skill)]),
  };
}

const UPDATE_STATUS_RANK: Record<string, number> = {
  update_available: 0,
  error: 1,
  source_missing: 1,
  checking: 2,
  unknown: 3,
  up_to_date: 4,
};

function compareBy(sortBy: LibrarySortBy): (a: ManagedSkill, b: ManagedSkill) => number {
  const byName = (a: ManagedSkill, b: ManagedSkill) => a.name.localeCompare(b.name);
  switch (sortBy) {
    case "updated":
      return (a, b) => b.updated_at - a.updated_at || byName(a, b);
    case "added":
      return (a, b) => b.created_at - a.created_at || byName(a, b);
    case "update_status":
      return (a, b) =>
        (UPDATE_STATUS_RANK[a.update_status] ?? 3) - (UPDATE_STATUS_RANK[b.update_status] ?? 3)
        || byName(a, b);
    default:
      return byName;
  }
}

/**
 * Sort a flat list. With a preset viewed, enabled skills come first and keep
 * the preset's saved order; everything else falls back to `sortBy`.
 */
export function sortLibrarySkills(skills: readonly ManagedSkill[], q: LibraryQuery): ManagedSkill[] {
  const compare = compareBy(q.sortBy);
  const preset = q.preset;
  if (!preset) return [...skills].sort(compare);
  return [...skills].sort((a, b) => {
    const aEnabled = a.preset_ids.includes(preset.id) ? 0 : 1;
    const bEnabled = b.preset_ids.includes(preset.id) ? 0 : 1;
    if (aEnabled !== bEnabled) return aEnabled - bEnabled;
    const aOrder = preset.order.indexOf(a.id);
    const bOrder = preset.order.indexOf(b.id);
    if (aOrder !== -1 && bOrder !== -1) return aOrder - bOrder;
    if (aOrder !== -1) return -1;
    if (bOrder !== -1) return 1;
    return compare(a, b);
  });
}

/**
 * Split an already-sorted list into groups. A skill with several tags or
 * agents appears under each. Group order follows first appearance, except the
 * empty bucket (untagged / not deployed / local) which always comes last. Tag
 * and creator groups are alphabetical.
 */
export function groupLibrarySkills(skills: readonly ManagedSkill[], groupBy: LibraryGroupBy): SkillGroup[] {
  if (groupBy === "none") return [{ key: "", skills: [...skills] }];
  const keysOf = (skill: ManagedSkill): string[] => {
    if (groupBy === "source") return [skill.source_type];
    if (groupBy === "agent") return agentKeysOf(skill);
    if (groupBy === "creator") return [creatorKey(skillCreator(skill))];
    return skill.tags.length > 0 ? skill.tags : [NO_TAG_GROUP];
  };
  const buckets = new Map<string, ManagedSkill[]>();
  for (const skill of skills) {
    for (const key of keysOf(skill)) {
      const bucket = buckets.get(key);
      if (bucket) bucket.push(skill);
      else buckets.set(key, [skill]);
    }
  }
  const groups = [...buckets].map(([key, list]) => ({ key, skills: list }));
  const isEmptyBucket = (key: string) => key === NO_TAG_GROUP || key === NOT_DEPLOYED || key === LOCAL_CREATOR;
  const nameOf = (group: SkillGroup) => creatorName(skillCreator(group.skills[0]));
  return groups.sort((a, b) => {
    const aEmpty = isEmptyBucket(a.key) ? 1 : 0;
    const bEmpty = isEmptyBucket(b.key) ? 1 : 0;
    if (aEmpty !== bEmpty) return aEmpty - bEmpty;
    if (groupBy === "tag") return a.key.localeCompare(b.key);
    return groupBy === "creator" ? nameOf(a).localeCompare(nameOf(b)) : 0;
  });
}
