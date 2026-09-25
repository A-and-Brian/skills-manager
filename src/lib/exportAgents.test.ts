import { describe, expect, it } from "vitest";
import type { ProjectAgentTarget } from "./tauri";
import { getDefaultExportAgents } from "./exportAgents";

function target(key: string, overrides: Partial<ProjectAgentTarget> = {}): ProjectAgentTarget {
  return {
    key,
    display_name: key,
    enabled: true,
    installed: true,
    is_custom: false,
    selected: true,
    relative_skills_dir: `.${key}/skills`,
    ...overrides,
  };
}

describe("getDefaultExportAgents", () => {
  it("keeps every available agent for a project that never chose, priority agents first", () => {
    const targets = [target("pi"), target("cursor"), target("claude_code")];

    expect(getDefaultExportAgents(targets)).toEqual(["claude_code", "cursor", "pi"]);
  });

  it("uses only the agents the project selected", () => {
    const targets = [target("claude_code"), target("cursor", { selected: false }), target("pi", { selected: false })];

    expect(getDefaultExportAgents(targets)).toEqual(["claude_code"]);
  });

  it("drops selected agents that are not installed or are turned off", () => {
    const targets = [
      target("claude_code", { installed: false }),
      target("cursor", { enabled: false }),
      target("pi"),
    ];

    expect(getDefaultExportAgents(targets)).toEqual(["pi"]);
  });

  it("returns nothing for a project with no agents selected", () => {
    expect(getDefaultExportAgents([target("claude_code", { selected: false })])).toEqual([]);
  });
});
