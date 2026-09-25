import { describe, expect, it } from "vitest";
import hostDispatchRs from "../../src-tauri/src/core/host_dispatch.rs?raw";
import tauriTs from "./tauri.ts?raw";
import {
  HOST_SCOPED_COMMANDS,
  HOST_SCOPED_SETTING_KEYS,
  LOCAL_ONLY_SETTING_KEYS,
  LOCAL_ONLY_SETTING_PREFIXES,
  VIEW_SETTING_KEYS,
  isHostScoped,
  isHostScopedSetting,
} from "./hostScope";

/** The quoted names in a Rust `const NAME: &[&str] = &[ … ];` list. */
function rustStringList(name: string): string[] {
  const block = hostDispatchRs.match(new RegExp(`const ${name}: &\\[&str\\] = &\\[([\\s\\S]*?)\\];`));
  if (!block) throw new Error(`${name} not found in host_dispatch.rs`);
  return [...block[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
}

/** The top-level keys of the object literal whose `{` is at `open`. */
function objectKeys(source: string, open: number): string[] {
  const entries: string[] = [];
  let depth = 0;
  let entry = "";
  for (const c of source.slice(open)) {
    if ("{([".includes(c)) depth++;
    else if ("})]".includes(c)) depth--;
    if (depth === 0) break;
    if (depth === 1 && (c === "{" || c === ",")) {
      entries.push(entry);
      entry = "";
    } else {
      entry += c;
    }
  }
  entries.push(entry);
  return entries.map((e) => e.split(":")[0].trim()).filter(Boolean);
}

/** Every `invoke("command", { … })` in tauri.ts: command → the key lists it is called with. */
function invokedArgs(): Map<string, string[][]> {
  const calls = new Map<string, string[][]>();
  for (const m of tauriTs.matchAll(/\binvoke(?:<[^(]*?>)?\(\s*"(\w+)"\s*([,)])/g)) {
    let keys: string[] = [];
    if (m[2] === ",") {
      const after = m.index + m[0].length;
      const open = after + tauriTs.slice(after).search(/\S/);
      if (tauriTs[open] !== "{") throw new Error(`${m[1]}: args are not an object literal`);
      keys = objectKeys(tauriTs, open);
    }
    calls.set(m[1], [...(calls.get(m[1]) ?? []), keys]);
  }
  return calls;
}

/** Every arm of `dispatch`: command → the keys it reads with `a.req` / `a.opt`. */
function dispatcherArgs(): Map<string, { req: string[]; opt: string[] }> {
  const start = hostDispatchRs.indexOf("pub fn dispatch(");
  const body = hostDispatchRs.slice(start, hostDispatchRs.indexOf("_ => Err(", start));
  const heads = [...body.matchAll(/^\s*("\w+"(?:\s*\|\s*"\w+")*)\s*=>/gm)];
  const arms = new Map<string, { req: string[]; opt: string[] }>();
  heads.forEach((head, i) => {
    const arm = body.slice(head.index, heads[i + 1]?.index);
    const read = (how: string) =>
      [...arm.matchAll(new RegExp(`a\\.${how}\\("(\\w+)"\\)`, "g"))].map((m) => m[1]);
    for (const [, command] of head[1].matchAll(/"(\w+)"/g)) {
      arms.set(command, { req: read("req"), opt: read("opt") });
    }
  });
  return arms;
}

const rustLocalKeys = rustStringList("LOCAL_ONLY_SETTINGS");
const rustLocalPrefixes = rustStringList("LOCAL_ONLY_SETTING_PREFIXES");
const refusedByHost = (key: string) =>
  rustLocalKeys.includes(key) || rustLocalPrefixes.some((prefix) => key.startsWith(prefix));

describe("host-scoped commands", () => {
  it("are exactly the commands the Rust dispatcher accepts", () => {
    const rust = rustStringList("COMMANDS");
    expect(rust.length).toBeGreaterThan(90);
    expect([...HOST_SCOPED_COMMANDS].sort()).toEqual([...rust].sort());
  });

  it("route library, project and preset work to the host", () => {
    expect(isHostScoped("get_managed_skills", {})).toBe(true);
    expect(isHostScoped("add_project", { path: "/srv/app" })).toBe(true);
    expect(isHostScoped("switch_preset", { presetId: "p" })).toBe(true);
    expect(isHostScoped("list_directory", { path: null })).toBe(true);
  });

  it("keep backup, browsing, app and session commands on this computer", () => {
    for (const command of [
      "git_backup_status",
      "github_backup_connect",
      "fetch_leaderboard",
      "search_skillssh",
      "check_app_update",
      "open_central_repo_folder",
      "slugify_skill_names",
      "remote_hosts_list",
      "remote_host_connect",
      "remote_invoke",
    ]) {
      expect(isHostScoped(command, {})).toBe(false);
    }
  });
});

describe("host-scoped command arguments", () => {
  it("are the names the Rust dispatcher reads", () => {
    const arms = dispatcherArgs();
    expect([...arms.keys()].sort()).toEqual(rustStringList("COMMANDS").sort());

    let checked = 0;
    for (const [command, calls] of invokedArgs()) {
      if (!HOST_SCOPED_COMMANDS.has(command)) continue;
      const { req, opt } = arms.get(command)!;
      for (const keys of calls) {
        for (const key of keys) expect([...req, ...opt], `${command} reads no ${key}`).toContain(key);
        for (const key of req) expect(keys, `${command} needs ${key}`).toContain(key);
      }
      checked++;
    }
    expect(checked).toBeGreaterThan(80);
  });
});

describe("settings routing", () => {
  it("sends machine settings to the host", () => {
    expect(isHostScoped("get_settings", { key: "sync_mode" })).toBe(true);
    expect(isHostScoped("set_settings", { key: "proxy_url", value: "" })).toBe(true);
    expect(isHostScoped("get_settings", { key: "project_last_used_export_agents:abc" })).toBe(true);
  });

  it("keeps app, backup and view settings here", () => {
    for (const key of ["theme", "text_size", "git_backup_remote_url", "backup_device_name", "library_group_by"]) {
      expect(isHostScoped("get_settings", { key })).toBe(false);
      expect(isHostScoped("set_settings", { key, value: "x" })).toBe(false);
    }
  });

  it("keeps a call without a key here", () => {
    expect(isHostScoped("get_settings", {})).toBe(false);
    expect(isHostScoped("get_settings")).toBe(false);
  });

  it("mirror the settings a host refuses", () => {
    expect([...LOCAL_ONLY_SETTING_KEYS].sort()).toEqual([...rustLocalKeys].sort());
    expect([...LOCAL_ONLY_SETTING_PREFIXES].sort()).toEqual([...rustLocalPrefixes].sort());
  });

  it("never route a setting the host would refuse", () => {
    for (const key of HOST_SCOPED_SETTING_KEYS) {
      expect(refusedByHost(key), key).toBe(false);
    }
  });

  it("classify every setting key the UI uses", () => {
    const sources = import.meta.glob<string>(["../**/*.{ts,tsx}", "!../**/*.test.ts"], {
      query: "?raw",
      import: "default",
      eager: true,
    });
    const keys = new Set<string>();
    for (const source of Object.values(sources)) {
      for (const m of source.matchAll(/(?:getSettings|setSettings)\(\s*"([^"]+)"/g)) keys.add(m[1]);
      for (const m of source.matchAll(/_SETTING(?:_KEY)? = "([^"]+)"/g)) keys.add(m[1]);
    }
    expect(keys.size).toBeGreaterThan(10);
    for (const key of keys) {
      const classified =
        isHostScopedSetting(key) || refusedByHost(key) || VIEW_SETTING_KEYS.includes(key);
      expect(classified, key).toBe(true);
    }
  });
});
