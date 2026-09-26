import { describe, expect, it } from "vitest";
import type { ProjectSkill } from "./tauri";
import {
  filterProjectSkillGroups,
  getAgentDotTargets,
  getAssignedAgents,
  groupProjectSkills,
  isCenterUpdatable,
  isProjectUpdatable,
  isVendoredSkillsDir,
  parseLastUsedAgents,
  pickInitialAgents,
  type ProjectSkillFilter,
} from "./projectSkillGroups";
import { UNTAGGED_FILTER } from "./skillTags";

function variant(agent: string, displayName: string, overrides: Partial<ProjectSkill> = {}): ProjectSkill {
  return {
    name: "Review",
    dir_name: "review",
    relative_path: "review",
    description: null,
    author: null,
    path: `/p/.${agent}/skills/review`,
    files: ["SKILL.md"],
    enabled: true,
    agent,
    agent_display_name: displayName,
    tags: [],
    in_center: true,
    sync_status: "in_sync",
    center_skill_id: "lib-review",
    agents_overridden: false,
    alias_of: null,
    vendored: false,
    ...overrides,
  };
}

const vendored = variant("cline", "Cline / Warp", { path: "/p/.agents/skills/review", vendored: true });
const claudeLink = variant("claude_code", "Claude Code", { alias_of: "review" });
const cursorLink = variant("cursor", "Cursor", { alias_of: "review" });

describe("groupProjectSkills", () => {
  it("folds links into .agents/skills under the vendored copy", () => {
    const [group] = groupProjectSkills([claudeLink, vendored, cursorLink]);

    expect(group.variants).toHaveLength(3);
    expect(group.primaryVariant).toBe(vendored);
    expect(group.effectiveVariants).toEqual([vendored]);
    expect(group.vendoredVariant).toBe(vendored);
  });

  it("keeps a real directory beside the vendored copy as its own copy", () => {
    const realDir = variant("claude_code", "Claude Code");
    const [group] = groupProjectSkills([vendored, realDir, cursorLink]);

    expect(group.effectiveVariants).toEqual([realDir, vendored]);
  });

  it("touches every copy of a skill nothing links to through .agents/skills", () => {
    const libraryLink = variant("cline", "Cline / Warp", { path: "/p/.agents/skills/review" });
    const claude = variant("claude_code", "Claude Code");
    const [group] = groupProjectSkills([claude, libraryLink]);

    expect(group.effectiveVariants).toEqual(group.variants);
    // A link kept in .agents/skills is an ordinary copy, not a vendored one.
    expect(group.vendoredVariant).toBeNull();
  });

  it("reports the most pressing status of any copy", () => {
    const [group] = groupProjectSkills(
      [vendored, variant("cursor", "Cursor", { sync_status: "center_newer" }), variant("pi", "Pi", { sync_status: "project_newer" })]
    );

    expect(group.status).toBe("project_newer");
    expect(group.totalCount).toBe(3);
  });

  it("still shows a skill that only has links", () => {
    const [group] = groupProjectSkills([cursorLink, claudeLink]);

    expect(group.primaryVariant).toBe(claudeLink);
    expect(group.effectiveVariants).toHaveLength(2);
  });

  it("groups by relative path regardless of case and sorts skills by name", () => {
    const other = variant("claude_code", "Claude Code", { name: "api", relative_path: "api" });
    const groups = groupProjectSkills([vendored, variant("cursor", "Cursor", { relative_path: "Review" }), other]);

    expect(groups.map((group) => [group.name, group.totalCount])).toEqual([["api", 1], ["Review", 2]]);
  });
});

describe("isVendoredSkillsDir", () => {
  it("matches .agents/skills however it is written", () => {
    expect(isVendoredSkillsDir(".agents/skills")).toBe(true);
    expect(isVendoredSkillsDir("./.agents/skills/")).toBe(true);
    expect(isVendoredSkillsDir(".agents\\skills")).toBe(true);
    expect(isVendoredSkillsDir(".claude/skills")).toBe(false);
  });
});

describe("isCenterUpdatable / isProjectUpdatable", () => {
  it.each([
    ["project_only", true, false],
    ["in_sync", false, false],
    ["project_newer", true, true],
    ["center_newer", false, true],
    ["diverged", true, true],
  ] as const)("%s: center %s, project %s", (status, center, project) => {
    expect(isCenterUpdatable(status)).toBe(center);
    expect(isProjectUpdatable(status)).toBe(project);
  });
});

describe("getAssignedAgents / getAgentDotTargets", () => {
  const variants = [
    variant("cursor", "Cursor"),
    variant("claude_code", "Claude Code"),
    variant("cursor", "Cursor", { relative_path: "other" }),
  ];

  it("lists each agent once, sorted by key", () => {
    expect(getAssignedAgents(variants)).toEqual(["claude_code", "cursor"]);
  });

  it("gives one dot per agent in variant order", () => {
    expect(getAgentDotTargets(variants)).toEqual([
      { key: "cursor", display_name: "Cursor" },
      { key: "claude_code", display_name: "Claude Code" },
    ]);
  });
});

describe("filterProjectSkillGroups", () => {
  const groups = groupProjectSkills([
    variant("claude_code", "Claude Code", { name: "Review", relative_path: "review", tags: ["code"] }),
    variant("claude_code", "Claude Code", {
      name: "Docs",
      relative_path: "docs",
      description: "Writes REVIEW notes",
      enabled: false,
    }),
  ]);
  const names = (filter: Partial<ProjectSkillFilter>) =>
    filterProjectSkillGroups(groups, { search: "", tags: new Set(), mode: "all", ...filter }).map((g) => g.name);

  it("searches name and description ignoring case", () => {
    expect(names({ search: "Review" })).toEqual(["Docs", "Review"]);
    expect(names({ search: "doc" })).toEqual(["Docs"]);
  });

  it("filters by tag, including the untagged pill", () => {
    expect(names({ tags: new Set(["code"]) })).toEqual(["Review"]);
    expect(names({ tags: new Set([UNTAGGED_FILTER]) })).toEqual(["Docs"]);
  });

  it("filters by enabled on any agent", () => {
    expect(names({ mode: "enabled" })).toEqual(["Review"]);
    expect(names({ mode: "disabled" })).toEqual(["Docs"]);
  });
});

describe("parseLastUsedAgents", () => {
  it("reads a stored list of agent keys", () => {
    expect(parseLastUsedAgents('["cursor","claude_code"]')).toEqual(["cursor", "claude_code"]);
  });

  it("drops entries that are not strings", () => {
    expect(parseLastUsedAgents('["cursor",1,null]')).toEqual(["cursor"]);
  });

  it("is null when missing, malformed or not a list", () => {
    expect(parseLastUsedAgents(null)).toBeNull();
    expect(parseLastUsedAgents("")).toBeNull();
    expect(parseLastUsedAgents("{not json")).toBeNull();
    expect(parseLastUsedAgents('{"agents":["cursor"]}')).toBeNull();
  });
});

describe("pickInitialAgents", () => {
  const available = new Set(["claude_code", "cursor", "cline"]);

  it("uses the last-used agents that are still available when the project never chose", () => {
    expect(pickInitialAgents(available, ["claude_code"], ["cursor", "gone"], false)).toEqual(["cursor"]);
  });

  it("keeps the project's own selection when it has one", () => {
    expect(pickInitialAgents(available, ["claude_code"], ["cursor"], true)).toEqual(["claude_code"]);
  });

  it("falls back to the selection when no last-used agent is available", () => {
    expect(pickInitialAgents(available, ["claude_code", "gone"], ["gone"], false)).toEqual(["claude_code"]);
    expect(pickInitialAgents(available, ["cline"], null, false)).toEqual(["cline"]);
  });
});
