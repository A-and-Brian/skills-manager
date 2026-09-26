import type {
  AppUpdateInfo,
  BatchImportResult,
  GitBackupSizeReport,
  GithubDevicePollResult,
  Preset,
  ProjectSkill,
} from "../../src/lib/tauri";
import type { State } from "./state";

/**
 * A fake command: reads and changes `state`, returns what the real command
 * would. Throwing rejects the invoke, like a backend error.
 */
type Handler<A> = (args: A, state: State) => unknown;

const nothing = () => null;

function presetsWithCounts(state: State): Preset[] {
  return [...state.presets]
    .sort((a, b) => a.sort_order - b.sort_order)
    .map((p) => ({ ...p, skill_count: state.skills.filter((s) => s.preset_ids.includes(p.id)).length }));
}

function findProjectSkill(state: State, projectId: string, relativePath: string, agent: string): ProjectSkill {
  const found = state.projectSkills[projectId]?.find(
    (s) => s.relative_path === relativePath && s.agent === agent,
  );
  if (!found) throw new Error(`fake backend: no project skill ${relativePath} for ${agent}`);
  return found;
}

function markInSync(state: State, { projectId, skillRelativePath, agent }: ProjectSkillArgs) {
  const found = findProjectSkill(state, projectId, skillRelativePath, agent);
  found.sync_status = "in_sync";
  found.in_center = true;
}

type ProjectSkillArgs = { projectId: string; skillRelativePath: string; agent: string };

// Only the commands the specs reach. Add one when a spec needs it; an unknown
// command fails the test with "fake backend: no handler for <cmd>".
export const handlers: Record<string, Handler<never>> = {
  // ── App shell ──
  log_startup_event: nothing,
  remote_host_disconnect: nothing,
  remote_hosts_list: () => [],
  check_app_update: (): AppUpdateInfo => ({
    has_update: false,
    current_version: "1.40.0",
    latest_version: "1.40.0",
    release_url: "",
  }),
  "plugin:opener|open_url": nothing,
  "plugin:dialog|open": (_args: unknown, state) => state.dialogPaths.shift() ?? null,

  // ── Settings ──
  get_settings: ({ key }: { key: string }, state) => state.settings[key] ?? null,
  set_settings: ({ key, value }: { key: string; value: string }, state) => {
    state.settings[key] = value;
    return null;
  },

  // ── Tools ──
  get_tool_status: (_args: unknown, state) => state.tools,

  // ── Skills and tags ──
  get_managed_skills: (_args: unknown, state) => state.skills,
  check_all_skill_updates: nothing,
  get_all_tags: (_args: unknown, state) => [...new Set(state.skills.flatMap((s) => s.tags))].sort(),
  rename_tag: ({ oldName, newName }: { oldName: string; newName: string }, state) => {
    for (const s of state.skills) {
      s.tags = [...new Set(s.tags.map((tag) => (tag === oldName ? newName : tag)))];
    }
    return null;
  },
  delete_tag: ({ name }: { name: string }, state) => {
    for (const s of state.skills) s.tags = s.tags.filter((tag) => tag !== name);
    return null;
  },
  get_skill_document: ({ skillId }: { skillId: string }, state) => {
    const found = state.skills.find((s) => s.id === skillId);
    return { skill_id: skillId, filename: "SKILL.md", content: `# ${found?.name}\n\nLibrary copy.`, central_path: found?.central_path ?? "" };
  },

  // ── Presets ──
  get_presets: (_args: unknown, state) => presetsWithCounts(state),
  get_active_preset: (_args: unknown, state) =>
    presetsWithCounts(state).find((p) => p.id === state.activePresetId) ?? null,
  reorder_presets: ({ ids }: { ids: string[] }, state) => {
    for (const p of state.presets) p.sort_order = ids.indexOf(p.id);
    return null;
  },
  get_preset_skill_order: ({ presetId }: { presetId: string }, state) => state.presetSkillOrder[presetId] ?? [],
  reorder_preset_skills: ({ presetId, skillIds }: { presetId: string; skillIds: string[] }, state) => {
    state.presetSkillOrder[presetId] = skillIds;
    return null;
  },

  // ── Projects ──
  get_projects: (_args: unknown, state) => [...state.projects].sort((a, b) => a.sort_order - b.sort_order),
  reorder_projects: ({ ids }: { ids: string[] }, state) => {
    for (const p of state.projects) p.sort_order = ids.indexOf(p.id);
    return null;
  },
  get_project_skills: ({ projectId }: { projectId: string }, state) => state.projectSkills[projectId] ?? [],
  get_project_agent_targets: ({ projectId }: { projectId: string }, state) =>
    state.projectAgentTargets[projectId] ?? [],
  get_project_skill_document: ({ skillRelativePath }: ProjectSkillArgs) => ({
    skill_name: skillRelativePath,
    filename: "SKILL.md",
    content: `# ${skillRelativePath}\n\nProject copy.`,
  }),
  slugify_skill_names: ({ names }: { names: string[] }) =>
    names.map((name) => name.toLowerCase().replace(/[^a-z0-9]+/g, "-")),
  update_project_skill_to_center: (args: ProjectSkillArgs, state) => {
    markInSync(state, args);
    return null;
  },
  update_project_skill_from_center: (args: ProjectSkillArgs, state) => {
    markInSync(state, args);
    return null;
  },

  // ── Install ──
  fetch_leaderboard: (_args: unknown, state) => state.market,
  search_skillssh: ({ query, limit }: { query: string; limit: number | null }, state) =>
    state.market
      .filter((s) => s.name.toLowerCase().includes(query.toLowerCase()))
      .slice(0, limit ?? undefined),
  scan_local_skills: (_args: unknown, state) => state.scan,
  batch_import_folder: (_args: { folderPath: string }, state): BatchImportResult => {
    const imported = state.batchImport.splice(0);
    state.skills.push(...imported);
    return { imported: imported.length, skipped: 0, errors: [] };
  },

  // ── Backup ──
  git_backup_status: (_args: unknown, state) => state.gitStatus,
  git_backup_fetch: nothing,
  git_backup_migrate_credentials: nothing,
  git_backup_pending_conflicts: () => [],
  git_backup_list_versions: () => [],
  git_backup_size_report: (): GitBackupSizeReport => ({
    total_bytes: 1024,
    oversized: [],
    skill_limit_bytes: 10_000_000,
    repo_warn_bytes: 100_000_000,
  }),
  git_backup_set_remote: ({ url }: { url: string }, state) => {
    state.gitStatus.remote_url = url;
    return url;
  },
  backup_device_name: () => "e2e-machine",
  github_device_flow_start: (_args: unknown, state) => state.deviceFlow.start,
  github_device_flow_poll: (_args: unknown, state): GithubDevicePollResult => {
    const status = state.deviceFlow.polls.shift() ?? "pending";
    return { status, result: status === "connected" ? state.deviceFlow.result : null };
  },
};
