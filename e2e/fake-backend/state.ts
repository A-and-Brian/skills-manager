import type {
  GitBackupStatus,
  GithubBackupConnectResult,
  GithubDeviceFlowStart,
  GithubDevicePollResult,
  ManagedSkill,
  Preset,
  Project,
  ProjectAgentTarget,
  ProjectSkill,
  ScanResult,
  SkillsShSkill,
  ToolInfo,
} from "../../src/lib/tauri";

/** What a test starts from. Every field is optional; a missing one is empty. */
export interface Seed {
  skills?: ManagedSkill[];
  /** `skill_count` is computed from the skills' `preset_ids`. */
  presets?: Preset[];
  activePresetId?: string | null;
  /** Saved skill order per preset id. */
  presetSkillOrder?: Record<string, string[]>;
  projects?: Project[];
  /** Skills found in each project, by project id. */
  projectSkills?: Record<string, ProjectSkill[]>;
  /** Agent targets of each project, by project id. */
  projectAgentTargets?: Record<string, ProjectAgentTarget[]>;
  tools?: ToolInfo[];
  settings?: Record<string, string>;
  gitStatus?: GitBackupStatus;
  /** The skills.sh catalog: the leaderboard shows all of it, search filters it by name. */
  market?: SkillsShSkill[];
  scan?: ScanResult;
  /** Answers of the native folder/file dialog, in order; null once used up. */
  dialogPaths?: string[];
  /** Skills `batch_import_folder` adds to the library. */
  batchImport?: ManagedSkill[];
  deviceFlow?: {
    start: GithubDeviceFlowStart;
    /** Statuses of successive polls; "pending" once used up. */
    polls: GithubDevicePollResult["status"][];
    result: GithubBackupConnectResult;
  };
}

/** The fake backend's in-memory data. Handlers read and change it. */
export type State = Required<Seed>;

export function createState({ settings, ...seed }: Seed): State {
  return structuredClone({
    skills: [],
    presets: [],
    activePresetId: null,
    presetSkillOrder: {},
    projects: [],
    projectSkills: {},
    projectAgentTargets: {},
    tools: [tool("claude_code", "Claude Code"), tool("codex", "Codex")],
    // Without it an empty library opens the first-run restore dialog.
    settings: { backup_first_run_prompt: "fresh", ...settings },
    gitStatus: gitStatus({ is_repo: false }),
    market: [],
    scan: { tools_scanned: 0, skills_found: 0, groups: [] },
    dialogPaths: [],
    batchImport: [],
    deviceFlow: {
      start: { device_code: "device-1", user_code: "ABCD-1234", verification_uri: "https://github.com/login/device", expires_in: 900, interval: 5 },
      polls: [],
      result: { url: "https://github.com/octo/skills-manager-backup.git", login: "octo", repo_created: false, repo_private: true, remote_has_content: true },
    },
    ...seed,
  });
}

// ── Factories for seeds: required fields only, sensible defaults for the rest ──

export function tool(key: string, display_name: string, extra: Partial<ToolInfo> = {}): ToolInfo {
  return {
    key,
    display_name,
    installed: true,
    skills_dir: `/home/e2e/.${key}/skills`,
    enabled: true,
    is_custom: false,
    has_path_override: false,
    project_relative_skills_dir: `.${key}/skills`,
    has_project_path_override: false,
    category: "coding",
    ...extra,
  };
}

export function skill(id: string, name: string, extra: Partial<ManagedSkill> = {}): ManagedSkill {
  return {
    id,
    name,
    description: `${name} description`,
    author: null,
    source_type: "local",
    source_ref: `/home/e2e/src/${name}`,
    source_ref_resolved: null,
    source_subpath: null,
    source_branch: null,
    source_revision: null,
    remote_revision: null,
    update_status: "up_to_date",
    last_checked_at: null,
    last_check_error: null,
    central_path: `/home/e2e/.skills-manager/skills/${name}`,
    enabled: true,
    created_at: 1_700_000_000,
    updated_at: 1_700_000_000,
    status: "ok",
    targets: [],
    preset_ids: [],
    tags: [],
    ...extra,
  };
}

export function preset(id: string, name: string, sort_order: number, extra: Partial<Preset> = {}): Preset {
  return {
    id,
    name,
    description: null,
    icon: null,
    sort_order,
    skill_count: 0,
    created_at: 1_700_000_000,
    updated_at: 1_700_000_000,
    ...extra,
  };
}

export function project(id: string, name: string, sort_order: number, extra: Partial<Project> = {}): Project {
  return {
    id,
    name,
    path: `/home/e2e/code/${name}`,
    workspace_type: "project",
    linked_agent_name: null,
    supports_skill_toggle: false,
    sort_order,
    skill_count: 0,
    sync_health: { in_sync: 0, project_newer: 0, center_newer: 0, diverged: 0, project_only: 0 },
    created_at: 1_700_000_000,
    updated_at: 1_700_000_000,
    agent_keys: null,
    deploy_mode: "link",
    ...extra,
  };
}

export function projectSkill(name: string, agent: string, extra: Partial<ProjectSkill> = {}): ProjectSkill {
  return {
    name,
    dir_name: name,
    relative_path: name,
    description: `${name} description`,
    author: null,
    path: `/home/e2e/code/project/.${agent}/skills/${name}`,
    files: ["SKILL.md"],
    enabled: true,
    agent,
    agent_display_name: agent,
    tags: [],
    in_center: false,
    sync_status: "project_only",
    center_skill_id: null,
    agents_overridden: false,
    alias_of: null,
    vendored: false,
    ...extra,
  };
}

export function agentTarget(key: string, display_name: string, extra: Partial<ProjectAgentTarget> = {}): ProjectAgentTarget {
  return {
    key,
    display_name,
    enabled: true,
    installed: true,
    is_custom: false,
    selected: true,
    relative_skills_dir: `.${key}/skills`,
    ...extra,
  };
}

export function gitStatus(extra: Partial<GitBackupStatus> = {}): GitBackupStatus {
  return {
    is_repo: true,
    remote_url: "https://github.com/octo/skills-manager-backup.git",
    branch: "main",
    has_changes: false,
    changed_skill_count: 0,
    ahead: 0,
    behind: 0,
    last_commit: null,
    last_commit_time: null,
    current_snapshot_tag: null,
    restored_from_tag: null,
    upstream_health: "healthy",
    ...extra,
  };
}
