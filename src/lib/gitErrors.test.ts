import type { TFunction } from "i18next";
import { describe, expect, it } from "vitest";
import {
  isAuthFailureMessage,
  isRecoverableSetupError,
  isSyncConflictError,
  mapGithubErrorMessage,
} from "./gitErrors";

// Returns the key itself, so tests can assert which copy was picked.
const t = ((key: string) => key) as unknown as TFunction;

describe("isSyncConflictError", () => {
  it("matches the conflict markers", () => {
    expect(isSyncConflictError(new Error("SYNC_CONFLICT: skills/a"))).toBe(true);
    expect(isSyncConflictError("CONFLICT (content): Merge conflict in a.md")).toBe(true);
  });

  it("ignores other errors", () => {
    expect(isSyncConflictError(new Error("Could not resolve host"))).toBe(false);
  });
});

describe("isRecoverableSetupError", () => {
  it.each([
    "fatal: refusing to merge unrelated histories",
    "! [rejected] main -> main (fetch first)",
    "error: failed to push some refs",
    "Updates were rejected because of a non-fast-forward update",
    "The current branch has no upstream branch",
    "SYNC_CONFLICT",
  ])("recovers from %s", (message) => {
    expect(isRecoverableSetupError(new Error(message))).toBe(true);
  });

  it("does not recover from auth or network errors", () => {
    expect(isRecoverableSetupError(new Error("Authentication failed"))).toBe(false);
  });
});

describe("mapGithubErrorMessage", () => {
  it.each([
    ["GITHUB_TOKEN_INVALID", "backup.github.errorToken"],
    ["GITHUB_SCOPE missing repo", "backup.github.errorScope"],
    ["KEYCHAIN_UNAVAILABLE", "backup.github.errorKeychain"],
    ["GITHUB_DEVICE_EXPIRED", "backup.github.deviceExpired"],
    ["GITHUB_DEVICE_DENIED", "backup.github.deviceDenied"],
  ])("maps %s to %s", (message, key) => {
    expect(mapGithubErrorMessage(new Error(message), t)).toBe(key);
  });

  it("adds the token fallback hint to network errors", () => {
    expect(mapGithubErrorMessage(new Error("GITHUB_NETWORK"), t)).toBe(
      "settings.gitErrorNetwork backup.github.deviceFallbackPat",
    );
    expect(mapGithubErrorMessage({ kind: "network", message: "timeout" }, t)).toBe(
      "settings.gitErrorNetwork backup.github.deviceFallbackPat",
    );
  });

  it("falls back to the git error copy", () => {
    expect(mapGithubErrorMessage(new Error("fatal: not a git repository"), t)).toBe("settings.gitErrorNotRepo");
  });
});

describe("isAuthFailureMessage", () => {
  it.each([
    "remote: Authentication failed for 'https://github.com/me/b.git'",
    "The requested URL returned error: 403",
    "remote: Invalid username or token",
    "fatal: could not read Username for 'https://github.com'",
  ])("matches %s", (message) => {
    expect(isAuthFailureMessage(message)).toBe(true);
  });

  it("ignores other failures", () => {
    expect(isAuthFailureMessage("Could not resolve host: github.com")).toBe(false);
  });
});
