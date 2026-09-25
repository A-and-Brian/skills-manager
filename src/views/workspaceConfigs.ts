import type { ToolCategory } from "../lib/tauri";

/**
 * Drives a single category of the workspace (coding agents or lobster agents).
 * Most of the per-agent UX is shared across categories; only the top-level
 * page title, the empty state copy, the URL prefix, and the category filter
 * differ.
 */
export interface WorkspaceConfig {
  category: ToolCategory;
  /** URL of the "all agents" overview; an agent page appends `/<agentKey>`. */
  basePath: string;
  routePath: "/global-workspace/{-$agentKey}" | "/lobster-workspace/{-$agentKey}";
  i18nKeys: {
    /** Heading shown on the "all agents" overview page. */
    title: string;
    /** Heading shown when no agents in this category are installed. */
    noAgents: string;
    /** Hint shown under the no-agents heading. */
    noAgentsHint: string;
  };
}

export const CODING_WORKSPACE_CONFIG: WorkspaceConfig = {
  category: "coding",
  basePath: "/global-workspace",
  routePath: "/global-workspace/{-$agentKey}",
  i18nKeys: {
    title: "globalWorkspace.title",
    noAgents: "globalWorkspace.noAgents",
    noAgentsHint: "globalWorkspace.noAgentsHint",
  },
};

export const LOBSTER_WORKSPACE_CONFIG: WorkspaceConfig = {
  category: "lobster",
  basePath: "/lobster-workspace",
  routePath: "/lobster-workspace/{-$agentKey}",
  i18nKeys: {
    title: "lobsterWorkspace.title",
    noAgents: "lobsterWorkspace.noAgents",
    noAgentsHint: "lobsterWorkspace.noAgentsHint",
  },
};
