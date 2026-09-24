import { describe, expect, it } from "vitest";
import type { ManagedSkill } from "./tauri";
import {
  filterLibrarySkills,
  groupLibrarySkills,
  NO_TAG_GROUP,
  NOT_DEPLOYED,
  sortLibrarySkills,
  updateFilterOf,
  type LibraryQuery,
} from "./librarySkillQuery";

const UNTAGGED = "__untagged_filter__";

function skill(overrides: Partial<ManagedSkill> & { id: string }): ManagedSkill {
  return {
    name: overrides.id,
    description: null,
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
    untaggedSentinel: UNTAGGED,
    agents: new Set(),
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

describe("filterLibrarySkills", () => {
  it("filters by agent, with a bucket for undeployed skills", () => {
    const all = [docx, pdf, notes];
    expect(filterLibrarySkills(all, query({ agents: new Set(["codex"]) }), displayName).map((s) => s.id)).toEqual(["docx"]);
    expect(filterLibrarySkills(all, query({ agents: new Set([NOT_DEPLOYED]) }), displayName).map((s) => s.id)).toEqual(["notes"]);
  });

  it("filters by update bucket, treating local sources as their own bucket", () => {
    const all = [docx, pdf, notes];
    expect(updateFilterOf(notes)).toBe("local");
    expect(filterLibrarySkills(all, query({ updates: new Set(["update_available"]) }), displayName).map((s) => s.id)).toEqual(["pdf"]);
    expect(filterLibrarySkills(all, query({ updates: new Set(["local", "up_to_date"]) }), displayName).map((s) => s.id)).toEqual(["docx", "notes"]);
  });

  it("keeps the untagged sentinel working alongside real tags", () => {
    const all = [docx, pdf, notes];
    expect(filterLibrarySkills(all, query({ tags: new Set([UNTAGGED, "office"]) }), displayName).map((s) => s.id)).toEqual(["docx", "notes"]);
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

  it("groups by agent in order of first appearance with undeployed last", () => {
    const groups = groupLibrarySkills([notes, docx, pdf], "agent");
    expect(groups.map((g) => [g.key, g.skills.map((s) => s.id)])).toEqual([
      ["claude", ["docx", "pdf"]],
      ["codex", ["docx"]],
      [NOT_DEPLOYED, ["notes"]],
    ]);
  });
});
