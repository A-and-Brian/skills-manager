import { dragOnto, expect, test } from "../fixtures";
import { preset, skill } from "../fake-backend/state";
import type { Page } from "@playwright/test";

// F3: reordering the viewed preset's skills in the library by dragging.

const inPreset = { preset_ids: ["p1"] };
const seed = {
  presets: [preset("p1", "Writing", 0)],
  activePresetId: "p1",
  skills: [skill("s1", "alpha", inPreset), skill("s2", "beta", inPreset), skill("s3", "gamma", inPreset)],
  presetSkillOrder: { p1: ["s1", "s2", "s3"] },
};

const skillNames = (page: Page) => page.getByRole("heading", { level: 3 });

async function drag(page: Page, name: string, onto: string) {
  const card = (n: string) =>
    page.locator('[aria-roledescription="sortable"]').filter({ has: page.getByRole("heading", { name: n, exact: true }) });
  await dragOnto(page, card(name), card(name).getByTitle("Drag to reorder"), card(onto));
}

test.beforeEach(async ({ page, backend }) => {
  await backend.seed(seed);
  await page.goto("/my-skills");
  await expect(skillNames(page)).toHaveText(["alpha", "beta", "gamma"]);
});

test("dragging a skill saves the preset's new order", async ({ page, backend }) => {
  await drag(page, "gamma", "alpha");

  await expect(skillNames(page)).toHaveText(["gamma", "alpha", "beta"]);
  expect(await backend.calls("reorder_preset_skills")).toEqual([{ presetId: "p1", skillIds: ["s3", "s1", "s2"] }]);

  await page.reload();
  await expect(skillNames(page)).toHaveText(["gamma", "alpha", "beta"]);
});

test("a failed save puts the saved order back", async ({ page, backend }) => {
  await backend.failNext("reorder_preset_skills");
  await drag(page, "gamma", "alpha");

  await expect.poll(() => backend.calls("reorder_preset_skills")).toHaveLength(1);
  await expect(skillNames(page)).toHaveText(["alpha", "beta", "gamma"]);
});
