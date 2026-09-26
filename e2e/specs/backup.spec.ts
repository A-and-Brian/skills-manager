import { expect, test, type FakeBackend } from "../fixtures";
import { gitStatus } from "../fake-backend/state";
import type { GitBackupStatus, GithubDevicePollResult } from "../../src/lib/tauri";
import type { Page } from "@playwright/test";

// F2 + F6: the backup page: its status states, GitHub device-flow sign-in and
// the refresh after a background backup.

const REMOTE = "https://github.com/octo/skills-manager-backup.git";

const statusCases: { state: string; status: GitBackupStatus; title: string; detail: string }[] = [
  { state: "no repository", status: gitStatus({ is_repo: false, remote_url: null }), title: "Not connected", detail: "Save a backup repository URL" },
  { state: "backed up", status: gitStatus(), title: "Backed up", detail: "Latest visible snapshot: no snapshot yet." },
  { state: "local changes", status: gitStatus({ ahead: 2 }), title: "Unbacked changes", detail: "Local changes: 2 · remote updates: 0." },
  { state: "remote changes", status: gitStatus({ behind: 3 }), title: "Updates from your other devices", detail: "3 update(s) were pushed" },
  { state: "unrelated histories", status: gitStatus({ upstream_health: "unrelated_histories" }), title: "Backup needs attention", detail: "cannot sync safely" },
];

/** The status card's headline. */
const statusTitle = (page: Page, title: string) => page.getByRole("heading", { name: title, level: 2 });

for (const { state, status, title, detail } of statusCases) {
  test(`status: ${state}`, async ({ page, backend }) => {
    await backend.seed({ gitStatus: status, settings: { git_backup_remote_url: status.remote_url ?? "" } });
    await page.goto("/backup");
    await expect(statusTitle(page, title)).toBeVisible();
    await expect(page.getByText(detail)).toBeVisible();
  });
}

test("refreshes when a background backup completes", async ({ page, backend }) => {
  await backend.seed({ gitStatus: gitStatus({ ahead: 1 }), settings: { git_backup_remote_url: REMOTE } });
  await page.goto("/backup");
  await expect(statusTitle(page, "Unbacked changes")).toBeVisible();

  await backend.patch({ gitStatus: gitStatus() });
  await backend.emit("backup-auto-completed", { ok: true, pending: false, error: null });
  await expect(statusTitle(page, "Backed up")).toBeVisible();

  await backend.emit("backup-auto-completed", { ok: false, pending: false, error: "could not reach the remote" });
  await expect(statusTitle(page, "Backup failed")).toBeVisible();
});

test.describe("GitHub device flow", () => {
  const signIn = (page: Page) => page.getByRole("button", { name: "Sign in with GitHub" });
  const userCode = (page: Page) => page.getByText("WXYZ-9876");

  /** A repository without a remote, so the page offers GitHub sign-in; the clock is frozen. */
  async function open(page: Page, backend: FakeBackend, polls: GithubDevicePollResult["status"][], expiresIn = 900) {
    await backend.seed({
      gitStatus: gitStatus({ remote_url: null }),
      deviceFlow: {
        start: { device_code: "device-1", user_code: "WXYZ-9876", verification_uri: "https://github.com/login/device", expires_in: expiresIn, interval: 5 },
        polls,
        result: { url: REMOTE, login: "octo", repo_created: false, repo_private: true, remote_has_content: true },
      },
    });
    await page.clock.install();
    await page.goto("/backup");
    await expect(signIn(page)).toBeEnabled();
    await page.clock.pauseAt(Date.now() + 60_000);
    await signIn(page).click();
    await expect(userCode(page)).toBeVisible();
  }

  /** Let `ms` of fake time pass, then wait for the poll count to reach `count`. */
  async function advance(page: Page, backend: FakeBackend, ms: number, count: number) {
    await page.clock.runFor(ms);
    await expect.poll(async () => (await backend.calls("github_device_flow_poll")).length).toBe(count);
  }

  test("connects once GitHub authorizes the code", async ({ page, backend }) => {
    await open(page, backend, ["pending", "connected"]);
    expect(await backend.calls("plugin:opener|open_url")).toContainEqual(
      expect.objectContaining({ url: "https://github.com/login/device" }),
    );

    await advance(page, backend, 5_000, 1);
    await expect(userCode(page)).toBeVisible();
    await advance(page, backend, 5_000, 2);

    // No toast check here: sonner shows toasts from a timer, and the clock is frozen.
    await expect(statusTitle(page, "Backed up")).toBeVisible();
    expect(await backend.calls("github_device_flow_poll")).toEqual([
      { deviceCode: "device-1", repoName: "skills-manager-backup" },
      { deviceCode: "device-1", repoName: "skills-manager-backup" },
    ]);
    expect(await backend.calls("git_backup_set_remote")).toEqual([{ url: REMOTE }]);
  });

  test("polls 5 seconds slower after slow_down", async ({ page, backend }) => {
    await open(page, backend, ["slow_down", "connected"]);

    await advance(page, backend, 5_000, 1);
    await page.clock.runFor(9_000);
    expect(await backend.calls("github_device_flow_poll")).toHaveLength(1);
    await advance(page, backend, 1_000, 2);

    await expect(statusTitle(page, "Backed up")).toBeVisible();
  });

  test("says so when the code expires", async ({ page, backend }) => {
    await open(page, backend, [], 12);

    await advance(page, backend, 5_000, 1);
    await advance(page, backend, 5_000, 2);
    await advance(page, backend, 5_000, 3);

    await expect(page.getByText("The verification code expired.")).toBeVisible();
    await expect(userCode(page)).toHaveCount(0);
    await expect(signIn(page)).toBeEnabled();
  });

  test("stops polling when cancelled", async ({ page, backend }) => {
    await open(page, backend, ["connected"]);

    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(userCode(page)).toHaveCount(0);
    await expect(signIn(page)).toBeEnabled();

    await page.clock.runFor(10_000);
    expect(await backend.calls("github_device_flow_poll")).toEqual([]);
    await expect(statusTitle(page, "Not connected")).toBeVisible();
  });
});
