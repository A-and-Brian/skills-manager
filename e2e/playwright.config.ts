import { defineConfig, devices } from "@playwright/test";

// Not 1420: that is the Tauri dev server's port, and a running `pnpm tauri:dev`
// must neither block the tests nor be reused without the fake backend.
const PORT = 1421;
const CI = !!process.env.CI;

export default defineConfig({
  testDir: "./specs",
  outputDir: "./test-results",
  fullyParallel: true,
  forbidOnly: CI,
  retries: 0,
  reporter: CI ? [["list"], ["html", { open: "never", outputFolder: "./playwright-report" }]] : "list",
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: `pnpm exec vite --mode e2e --port ${PORT}`,
    cwd: "..",
    url: `http://127.0.0.1:${PORT}`,
    reuseExistingServer: !CI,
  },
});
