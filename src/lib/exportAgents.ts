import type { ProjectAgentTarget } from "./tauri";

const PROJECT_EXPORT_AGENT_PRIORITY = ["claude_code", "codex", "cursor", "gemini_cli", "github_copilot"];

// Keys of project agents that can actually receive skills right now: both
// installed on disk and enabled by the user. Used everywhere export targets
// are derived so disabled/uninstalled agents never get project-local skills.
export function enabledInstalledAgentKeys(targets: ProjectAgentTarget[]): string[] {
  return targets.filter((target) => target.installed && target.enabled).map((target) => target.key);
}

// The project's selected agents that can receive skills right now. A project
// that never chose has every available agent selected, so for it this is
// every installed and enabled agent, as it always was.
export function getDefaultExportAgents(targets: ProjectAgentTarget[]): string[] {
  const enabledKeys = enabledInstalledAgentKeys(targets.filter((target) => target.selected));
  const availableKeys = new Set(enabledKeys);
  // Priority agents first, then every other selected agent in its detected
  // order. None are dropped: preset export must reach each one the project
  // uses (issue #400 — non-priority agents like "pi" were silently dropped
  // when any priority agent was on).
  const prioritized = PROJECT_EXPORT_AGENT_PRIORITY.filter((key) => availableKeys.has(key));
  const rest = enabledKeys.filter((key) => !prioritized.includes(key));
  return Array.from(new Set([...prioritized, ...rest]));
}
