import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const dialogOpen = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: dialogOpen }));

import { setActiveHostId } from "./hostCall";
import { pickPath, setRemotePicker } from "./pickPath";

beforeEach(() => dialogOpen.mockReset());
afterEach(() => {
  setActiveHostId(null);
  setRemotePicker(null);
});

describe("pickPath on this computer", () => {
  it("opens the native dialog for a folder or a filtered file", async () => {
    dialogOpen.mockResolvedValue("/Users/me/skills");
    await expect(pickPath({ directory: true }, { startPath: "/ignored" })).resolves.toBe("/Users/me/skills");
    await pickPath({ files: ["zip", "skill"], filterName: "Skills" });
    expect(dialogOpen).toHaveBeenNthCalledWith(1, { directory: true, multiple: false });
    expect(dialogOpen).toHaveBeenNthCalledWith(2, {
      multiple: false,
      filters: [{ name: "Skills", extensions: ["zip", "skill"] }],
    });
  });

  it("resolves null when the dialog is cancelled", async () => {
    dialogOpen.mockResolvedValue(null);
    await expect(pickPath({ directory: true })).resolves.toBeNull();
  });
});

describe("pickPath with a remote host active", () => {
  beforeEach(() => setActiveHostId("host-1"));

  it("asks the remote browser and never the native dialog", async () => {
    const remote = vi.fn().mockResolvedValue("/home/me/project");
    setRemotePicker(remote);
    await expect(pickPath({ directory: true }, { startPath: "/home/me" })).resolves.toBe("/home/me/project");
    expect(remote).toHaveBeenCalledWith({ directory: true }, { startPath: "/home/me" });
    expect(dialogOpen).not.toHaveBeenCalled();
  });

  it("fails rather than falling back to this computer's disk", async () => {
    await expect(pickPath({ directory: true })).rejects.toThrow();
    expect(dialogOpen).not.toHaveBeenCalled();
  });
});
