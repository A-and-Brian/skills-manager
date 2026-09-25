import { describe, expect, it } from "vitest";
import { isSettingsPath, resolveSettingsCategory, settingsPath } from "./categories";

describe("resolveSettingsCategory", () => {
  it("returns a known category", () => {
    expect(resolveSettingsCategory("general")).toBe("general");
    expect(resolveSettingsCategory("remote")).toBe("remote");
  });

  it("returns null for a missing, unknown, or differently cased slug", () => {
    expect(resolveSettingsCategory(undefined)).toBeNull();
    expect(resolveSettingsCategory("")).toBeNull();
    expect(resolveSettingsCategory("proxy")).toBeNull();
    expect(resolveSettingsCategory("About")).toBeNull();
  });
});

describe("settingsPath", () => {
  it("defaults to the general category", () => {
    expect(settingsPath()).toBe("/settings/general");
  });

  it("builds the path for a category", () => {
    expect(settingsPath("about")).toBe("/settings/about");
  });
});

describe("isSettingsPath", () => {
  it("matches the settings page and its categories", () => {
    expect(isSettingsPath("/settings")).toBe(true);
    expect(isSettingsPath("/settings/about")).toBe(true);
  });

  it("does not match other pages that share a prefix", () => {
    expect(isSettingsPath("/settingsx")).toBe(false);
    expect(isSettingsPath("/remote/x")).toBe(false);
  });
});
