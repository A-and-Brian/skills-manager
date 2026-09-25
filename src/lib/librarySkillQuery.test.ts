import { describe, expect, it } from "vitest";
import type { ManagedSkill } from "./tauri";
import {
  canRefreshSkill,
  filterLibrarySkills,
  groupLibrarySkills,
  libraryCreators,
  libraryFilterCounts,
  NO_TAG_GROUP,
  NOT_DEPLOYED,
  skillDisplayNames,
  sortLibrarySkills,
  togglableSkills,
  updateFilterOf,
  type LibraryQuery,
} from "./librarySkillQuery";
import { LOCAL_CREATOR } from "./skillCreator";
import { UNTAGGED_FILTER } from "./skillTags";

function skill(overrides: Partial<ManagedSkill> & { id: string }): ManagedSkill {
  return {
    name: overrides.id,
    description: null,
    author: null,
    source_type: "git",
    source_ref: null,
    source_ref_resolved: null,
    source_subpath: null,
    source_branch: null,
    source_revision: null,
    remote_revision: null,
    update_status: "unknown",
    last_checked_at: null,
    last_check_error: null,
    central_path: `/lib/${overrides.id}`,
    enabled: true,
    created_at: 0,
    updated_at: 0,
    status: "ok",
    targets: [],
    preset_ids: [],
    tags: [],
    ...overrides,
  };
}

function target(tool: string) {
  return { id: `${tool}-t`, skill_id: "", tool, target_path: "", mode: "symlink", status: "synced", synced_at: null };
}

function query(overrides: Partial<LibraryQuery> = {}): LibraryQuery {
  return {
    search: "",
    sources: new Set(),
    tags: new Set(),
    agents: new Set(),
    creators: new Set(),
    updates: new Set(),
    sortBy: "name",
    groupBy: "none",
    ...overrides,
  };
}

const displayName = (s: ManagedSkill) => s.name;

const docx = skill({ id: "docx", tags: ["docs", "office"], targets: [target("claude"), target("codex")], update_status: "up_to_date", updated_at: 30, created_at: 1 });
const pdf = skill({ id: "pdf", tags: ["docs"], targets: [target("claude")], update_status: "update_available", updated_at: 10, created_at: 3 });
const notes = skill({ id: "notes", source_type: "local", update_status: "up_to_date", updated_at: 20, created_at: 2 });

const byZed = skill({ id: "zed-pdf", source_type: "skillssh", source_ref: "zed/tools/pdf" });
const byAcme = skill({ id: "acme-lint", source_ref: "https://github.com/Acme/lint.git" });
const byAcmeToo = skill({ id: "acme-docs", source_type: "skillssh", source_ref: "acme/docs/readme" });
const byJane = skill({ id: "jane-notes", source_type: "local", author: "Jane" });
const nobody = skill({ id: "scratch", source_type: "import" });
const creatorSkills = [nobody, byZed, byJane, byAcme, byAcmeToo];

describe("filterLibrarySkills", () => {
  it("filters by agent, with a bucket for undeployed skills", () => {
    const all = [docx, pdf, notes];
    expect(filterLibrarySkills(all, query({ agents: new Set(["codex"]) }), displayName).map((s) => s.id)).toEqual(["docx"]);
    expect(filterLibrarySkills(all, query({ agents: new Set([NOT_DEPLOYED]) }), displayName).map((s) => s.id)).toEqual(["notes"]);
  });

  it("filters by update bucket, treating local sources as their own bucket", () => {
    const all = [docx, pdf, notes];
    expect(updateFilterOf(notes)).toBe("local");
    expect(updateFilterOf({ ...notes, update_status: "update_available" })).toBe("update_available");
    expect(updateFilterOf({ ...notes, source_type: "import", update_status: "source_missing" })).toBe("error");
    expect(updateFilterOf({ ...notes, update_status: "local_only" })).toBe("local");
    expect(filterLibrarySkills(all, query({ updates: new Set(["update_available"]) }), displayName).map((s) => s.id)).toEqual(["pdf"]);
    expect(filterLibrarySkills(all, query({ updates: new Set(["local", "up_to_date"]) }), displayName).map((s) => s.id)).toEqual(["docx", "notes"]);
  });

  it("keeps the untagged sentinel working alongside real tags", () => {
    const all = [docx, pdf, notes];
    expect(filterLibrarySkills(all, query({ tags: new Set([UNTAGGED_FILTER, "office"]) }), displayName).map((s) => s.id)).toEqual(["docx", "notes"]);
  });

  it("filters by creator: GitHub owner, frontmatter author or local", () => {
    const ids = (creators: string[]) =>
      filterLibrarySkills(creatorSkills, query({ creators: new Set(creators) }), displayName).map((s) => s.id);
    expect(ids(["github.com/acme"])).toEqual(["acme-lint", "acme-docs"]);
    expect(ids(["author:jane", LOCAL_CREATOR])).toEqual(["scratch", "jane-notes"]);
  });

  it("finds skills by searching their creator", () => {
    const found = (search: string) =>
      filterLibrarySkills(creatorSkills, query({ search }), displayName).map((s) => s.id);
    expect(found("@acme")).toEqual(["acme-lint", "acme-docs"]);
    expect(found("jane")).toEqual(["jane-notes"]);
  });

  it("respects preset enabled / available modes", () => {
    const inPreset = skill({ id: "a", preset_ids: ["p1"] });
    const out = skill({ id: "b" });
    const preset = { id: "p1", order: [], mode: "enabled" as const };
    expect(filterLibrarySkills([inPreset, out], query({ preset }), displayName).map((s) => s.id)).toEqual(["a"]);
    expect(filterLibrarySkills([inPreset, out], query({ preset: { ...preset, mode: "available" } }), displayName).map((s) => s.id)).toEqual(["b"]);
  });
});

describe("sortLibrarySkills", () => {
  it("sorts by the chosen key with name as a stable tiebreak", () => {
    const all = [docx, pdf, notes];
    expect(sortLibrarySkills(all, query({ sortBy: "updated" })).map((s) => s.id)).toEqual(["docx", "notes", "pdf"]);
    expect(sortLibrarySkills(all, query({ sortBy: "added" })).map((s) => s.id)).toEqual(["pdf", "notes", "docx"]);
    expect(sortLibrarySkills(all, query({ sortBy: "update_status" })).map((s) => s.id)).toEqual(["pdf", "docx", "notes"]);
  });

  it("puts preset-enabled skills first in preset order, then falls back to sortBy", () => {
    const a = skill({ id: "a", preset_ids: ["p1"], updated_at: 1 });
    const b = skill({ id: "b", preset_ids: ["p1"], updated_at: 9 });
    const c = skill({ id: "c", updated_at: 5 });
    const d = skill({ id: "d", updated_at: 7 });
    const preset = { id: "p1", order: ["b", "a"], mode: "all" as const };
    expect(sortLibrarySkills([a, b, c, d], query({ sortBy: "updated", preset })).map((s) => s.id)).toEqual(["b", "a", "d", "c"]);
  });
});

describe("groupLibrarySkills", () => {
  it("returns a single unkeyed group when grouping is off", () => {
    expect(groupLibrarySkills([docx, pdf], "none")).toEqual([{ key: "", skills: [docx, pdf] }]);
  });

  it("lists a multi-tag skill under every tag and puts the untagged bucket last", () => {
    const groups = groupLibrarySkills([notes, docx, pdf], "tag");
    expect(groups.map((g) => [g.key, g.skills.map((s) => s.id)])).toEqual([
      ["docs", ["docx", "pdf"]],
      ["office", ["docx"]],
      [NO_TAG_GROUP, ["notes"]],
    ]);
  });

  it("groups by creator alphabetically with local last", () => {
    const groups = groupLibrarySkills(creatorSkills, "creator");
    expect(groups.map((g) => [g.key, g.skills.map((s) => s.id)])).toEqual([
      ["github.com/acme", ["acme-lint", "acme-docs"]],
      ["author:jane", ["jane-notes"]],
      ["github.com/zed", ["zed-pdf"]],
      [LOCAL_CREATOR, ["scratch"]],
    ]);
  });

  it("groups by agent in order of first appearance with undeployed last", () => {
    const groups = groupLibrarySkills([notes, docx, pdf], "agent");
    expect(groups.map((g) => [g.key, g.skills.map((s) => s.id)])).toEqual([
      ["claude", ["docx", "pdf"]],
      ["codex", ["docx"]],
      [NOT_DEPLOYED, ["notes"]],
    ]);
  });
});

describe("libraryCreators", () => {
  it("counts skills per creator, most first, with local last", () => {
    expect(libraryCreators(creatorSkills).map((o) => [o.key, o.count])).toEqual([
      ["github.com/acme", 2],
      ["author:jane", 1],
      ["github.com/zed", 1],
      [LOCAL_CREATOR, 1],
    ]);
  });
});

describe("libraryFilterCounts", () => {
  const all = [docx, pdf, notes];
  const counts = (skills: ManagedSkill[], overrides: Partial<LibraryQuery> = {}) =>
    libraryFilterCounts(skills, query(overrides), displayName);

  it("counts an option against the other categories, not its own", () => {
    const withCodex = counts(all, { agents: new Set(["codex"]) });
    expect(withCodex.agents).toEqual(new Map([["claude", 2], ["codex", 1], [NOT_DEPLOYED, 1]]));
    expect(withCodex.tags).toEqual(new Map([["docs", 1], ["office", 1]]));
    expect(withCodex.sources).toEqual(new Map([["git", 1]]));
  });

  it("counts untagged and undeployed skills under their sentinels", () => {
    const result = counts(all);
    expect(result.tags.get(UNTAGGED_FILTER)).toBe(1);
    expect(result.agents.get(NOT_DEPLOYED)).toBe(1);
  });

  it("leaves skills with no update state out of the update counts", () => {
    expect(counts([docx, byAcme, byZed]).updates).toEqual(new Map([["up_to_date", 1]]));
  });

  it("applies search and preset mode like the list does", () => {
    expect(counts(all, { search: "pdf" }).tags).toEqual(new Map([["docs", 1]]));
    const inPreset = skill({ id: "a", source_type: "local", preset_ids: ["p1"] });
    const out = skill({ id: "b" });
    const preset = { id: "p1", order: [], mode: "enabled" as const };
    expect(counts([inPreset, out], { preset }).sources).toEqual(new Map([["local", 1]]));
  });

  it("counts creators by key, Local included", () => {
    const result = counts(creatorSkills, { creators: new Set(["github.com/acme"]) });
    expect(result.creators).toEqual(
      new Map([[LOCAL_CREATOR, 1], ["github.com/zed", 1], ["author:jane", 1], ["github.com/acme", 2]])
    );
    expect(result.sources).toEqual(new Map([["git", 1], ["skillssh", 1]]));
  });
});

describe("skillDisplayNames", () => {
  it("shows the name when it is unique", () => {
    const names = skillDisplayNames([skill({ id: "a", name: "Review" }), skill({ id: "b", name: "Docs" })]);
    expect(names.get("a")).toBe("Review");
    expect(names.get("b")).toBe("Docs");
  });

  it("shows the library folder name for duplicated names", () => {
    const names = skillDisplayNames([
      skill({ id: "a", name: "Review", central_path: "/lib/review-acme" }),
      skill({ id: "b", name: "Review", central_path: "C:\\lib\\review-local\\" }),
    ]);
    expect(names.get("a")).toBe("review-acme");
    expect(names.get("b")).toBe("review-local");
  });

  it("keeps the name when a duplicate's folder already carries it", () => {
    const names = skillDisplayNames([
      skill({ id: "a", name: "Review", central_path: "/lib/Review" }),
      skill({ id: "b", name: "Review", central_path: "/lib/review-2" }),
    ]);
    expect(names.get("a")).toBe("Review");
    expect(names.get("b")).toBe("review-2");
  });
});

describe("canRefreshSkill", () => {
  it("refreshes git and skills.sh skills", () => {
    expect(canRefreshSkill(skill({ id: "a", source_type: "git" }))).toBe(true);
    expect(canRefreshSkill(skill({ id: "a", source_type: "skillssh" }))).toBe(true);
  });

  it("refreshes local and imported skills only with a source_ref", () => {
    expect(canRefreshSkill(skill({ id: "a", source_type: "local", source_ref: "/src/a" }))).toBe(true);
    expect(canRefreshSkill(skill({ id: "a", source_type: "import", source_ref: "/src/a" }))).toBe(true);
    expect(canRefreshSkill(skill({ id: "a", source_type: "local", source_ref: null }))).toBe(false);
    expect(canRefreshSkill(skill({ id: "a", source_type: "import", source_ref: null }))).toBe(false);
  });
});

describe("togglableSkills", () => {
  const skills = [
    skill({ id: "on", preset_ids: ["p"] }),
    skill({ id: "off", preset_ids: [] }),
    skill({ id: "unselected", preset_ids: [] }),
  ];
  const selected = new Set(["on", "off"]);

  it("counts only the selected skills an enable would add", () => {
    expect(togglableSkills(skills, selected, "p", true).map((s) => s.id)).toEqual(["off"]);
  });

  it("counts only the selected skills a disable would remove", () => {
    expect(togglableSkills(skills, selected, "p", false).map((s) => s.id)).toEqual(["on"]);
  });
});
