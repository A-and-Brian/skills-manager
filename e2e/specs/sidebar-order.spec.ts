import { dragOnto, expect, test } from "../fixtures";
import { preset, project, tool } from "../fake-backend/state";
import type { Page } from "@playwright/test";

// F1: the sidebar's presets, projects and agents are reordered by dragging.

const seed = {
  presets: [preset("p1", "Writing", 0), preset("p2", "Research", 1), preset("p3", "Travel", 2)],
  projects: [project("w1", "api", 0), project("w2", "web", 1), project("w3", "docs", 2)],
  tools: [tool("claude_code", "Claude Code"), tool("codex", "Codex"), tool("cursor", "Cursor")],
};

/** Every draggable sidebar row, top to bottom: presets, agents, projects. */
const sortable = (page: Page) => page.locator('[aria-roledescription="sortable"]');
const row = (page: Page, name: string) => sortable(page).filter({ hasText: name });

async function drag(page: Page, name: string, onto: string) {
  const item = row(page, name);
  await dragOnto(page, item, item.getByTestId("drag-handle"), row(page, onto));
}

test.beforeEach(async ({ page, backend }) => {
  await backend.seed(seed);
  await page.goto("/");
  await expect(sortable(page)).toHaveText(["Writing", "Research", "Travel", "Claude Code", "Codex", "Cursor", "api", "web", "docs"]);
});

test("dragging a preset saves the new order", async ({ page, backend }) => {
  await drag(page, "Travel", "Writing");

  await expect(sortable(page).filter({ hasText: /Writing|Research|Travel/ })).toHaveText(["Travel", "Writing", "Research"]);
  expect(await backend.calls("reorder_presets")).toEqual([{ ids: ["p3", "p1", "p2"] }]);

  await page.reload();
  await expect(sortable(page).filter({ hasText: /Writing|Research|Travel/ })).toHaveText(["Travel", "Writing", "Research"]);
});

test("a failed preset save puts the old order back and says so", async ({ page, backend }) => {
  await backend.failNext("reorder_presets");
  await drag(page, "Travel", "Writing");

  await expect(page.getByText("Something went wrong")).toBeVisible();
  await expect(sortable(page).filter({ hasText: /Writing|Research|Travel/ })).toHaveText(["Writing", "Research", "Travel"]);
});

test("dragging a project saves the new order", async ({ page, backend }) => {
  await drag(page, "api", "docs");

  await expect(sortable(page).filter({ hasText: /^(api|web|docs)$/ })).toHaveText(["web", "docs", "api"]);
  expect(await backend.calls("reorder_projects")).toEqual([{ ids: ["w2", "w3", "w1"] }]);
});

test("a failed project save puts the old order back and says so", async ({ page, backend }) => {
  await backend.failNext("reorder_projects");
  await drag(page, "api", "docs");

  await expect(page.getByText("Something went wrong")).toBeVisible();
  await expect(sortable(page).filter({ hasText: /^(api|web|docs)$/ })).toHaveText(["api", "web", "docs"]);
});

test("the agent order survives a reload", async ({ page }) => {
  await drag(page, "Cursor", "Claude Code");

  const agents = sortable(page).filter({ hasText: /Claude Code|Codex|Cursor/ });
  await expect(agents).toHaveText(["Cursor", "Claude Code", "Codex"]);
  await page.reload();
  await expect(agents).toHaveText(["Cursor", "Claude Code", "Codex"]);
});
