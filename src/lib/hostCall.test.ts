import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const tauriInvoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke: tauriInvoke }));

import { getActiveHostId, invoke, setActiveHostId, trackHost } from "./hostCall";
import { getLocalSettings, getSettings } from "./tauri";

beforeEach(() => {
  tauriInvoke.mockReset();
  tauriInvoke.mockResolvedValue("ok");
});

afterEach(() => setActiveHostId(null));

describe("invoke on this computer", () => {
  it("runs every command locally", async () => {
    await invoke("get_managed_skills");
    await invoke("set_settings", { key: "sync_mode", value: "copy" });
    expect(tauriInvoke).toHaveBeenNthCalledWith(1, "get_managed_skills", undefined, undefined);
    expect(tauriInvoke).toHaveBeenNthCalledWith(
      2,
      "set_settings",
      { key: "sync_mode", value: "copy" },
      undefined
    );
  });
});

describe("invoke with a remote host active", () => {
  beforeEach(() => setActiveHostId("host-1"));

  it("sends host-scoped commands through remote_invoke with their args", async () => {
    const args = { skillId: "s1", tool: "claude" };
    await expect(invoke("sync_skill_to_tool", args)).resolves.toBe("ok");
    expect(tauriInvoke).toHaveBeenCalledWith(
      "remote_invoke",
      { hostId: "host-1", command: "sync_skill_to_tool", args },
      undefined
    );
  });

  it("sends an empty args object when the command takes none", async () => {
    await invoke("get_presets");
    expect(tauriInvoke).toHaveBeenCalledWith(
      "remote_invoke",
      { hostId: "host-1", command: "get_presets", args: {} },
      undefined
    );
  });

  it("routes settings by key", async () => {
    await invoke("get_settings", { key: "proxy_url" });
    await invoke("get_settings", { key: "theme" });
    expect(tauriInvoke).toHaveBeenNthCalledWith(
      1,
      "remote_invoke",
      { hostId: "host-1", command: "get_settings", args: { key: "proxy_url" } },
      undefined
    );
    expect(tauriInvoke).toHaveBeenNthCalledWith(2, "get_settings", { key: "theme" }, undefined);
  });

  it("keeps local commands local", async () => {
    await invoke("git_backup_status");
    await invoke("remote_host_disconnect");
    expect(tauriInvoke).toHaveBeenNthCalledWith(1, "git_backup_status", undefined, undefined);
    expect(tauriInvoke).toHaveBeenNthCalledWith(2, "remote_host_disconnect", undefined, undefined);
  });

  it("passes remote errors through unchanged", async () => {
    const error = { kind: "network", message: "Connection to box was lost" };
    tauriInvoke.mockRejectedValueOnce(error);
    await expect(invoke("get_managed_skills")).rejects.toBe(error);
  });

  it("reads this computer's proxy for the app updater", async () => {
    await getSettings("proxy_url");
    await getLocalSettings("proxy_url");
    expect(tauriInvoke).toHaveBeenNthCalledWith(
      1,
      "remote_invoke",
      { hostId: "host-1", command: "get_settings", args: { key: "proxy_url" } },
      undefined
    );
    expect(tauriInvoke).toHaveBeenNthCalledWith(2, "get_settings", { key: "proxy_url" }, undefined);
  });

  it("goes back to local calls after switching to this computer", async () => {
    setActiveHostId(null);
    expect(getActiveHostId()).toBeNull();
    await invoke("get_managed_skills");
    expect(tauriInvoke).toHaveBeenCalledWith("get_managed_skills", undefined, undefined);
  });
});

describe("trackHost", () => {
  it("tells whether an answer still belongs to the machine on screen", () => {
    setActiveHostId("host-1");
    const onHost1 = trackHost();
    expect(onHost1()).toBe(true);

    setActiveHostId("host-2");
    expect(onHost1()).toBe(false);
    const onHost2 = trackHost();

    setActiveHostId(null);
    expect(onHost2()).toBe(false);
    expect(trackHost()()).toBe(true);
  });
});
