import { expect, test } from "../fixtures";
import { skill } from "../fake-backend/state";
import type { Page } from "@playwright/test";

// F3: the library view: seeded skills, grid and list, and group and sort
// preferences saved through the settings commands.

const seed = {
  skills: [
    skill("s1", "alpha", { tags: ["writing"], created_at: 100 }),
    skill("s2", "beta", { tags: ["writing"], created_at: 300 }),
    skill("s3", "gamma", { created_at: 200 }),
  ],
};

/** Skill names and group headings, top to bottom. */
const headings = (page: Page) => page.getByRole("heading", { level: 3 });
const skillHeading = (page: Page, name: string) => page.getByRole("heading", { name, exact: true });

async function topOf(page: Page, name: string) {
  const box = await skillHeading(page, name).boundingBox();
  return box!.y;
}

test.beforeEach(async ({ page, backend }) => {
  await backend.seed(seed);
  await page.goto("/my-skills");
});

test("shows the seeded skills, sorted by name", async ({ page }) => {
  await expect(page.getByRole("heading", { name: /^Library/, level: 1 })).toContainText("3");
  await expect(headings(page)).toHaveText(["alpha", "beta", "gamma"]);
});

test("switches between grid and list", async ({ page }) => {
  await expect(headings(page)).toHaveText(["alpha", "beta", "gamma"]);
  // Grid: the cards share a row. List: one row each.
  expect(await topOf(page, "beta")).toBe(await topOf(page, "alpha"));

  await page.getByTestId("view-list").click();
  await expect.poll(async () => (await topOf(page, "beta")) > (await topOf(page, "alpha"))).toBe(true);

  await page.getByTestId("view-grid").click();
  await expect.poll(async () => (await topOf(page, "beta")) === (await topOf(page, "alpha"))).toBe(true);
});

test("group and sort are saved and come back after a reload", async ({ page, backend }) => {
  await page.getByRole("combobox", { name: "Sort" }).selectOption({ label: "Recently added" });
  await expect(headings(page)).toHaveText(["beta", "gamma", "alpha"]);

  await page.getByRole("combobox", { name: "Group by" }).selectOption({ label: "By tag" });
  await expect(headings(page)).toHaveText([/^writing/, "beta", "alpha", /^Untagged/, "gamma"]);

  expect(await backend.calls("set_settings")).toEqual([
    { key: "library_sort_by", value: "added" },
    { key: "library_group_by", value: "tag" },
  ]);

  await page.reload();
  await expect(page.getByRole("combobox", { name: "Sort" })).toHaveValue("added");
  await expect(page.getByRole("combobox", { name: "Group by" })).toHaveValue("tag");
  await expect(headings(page)).toHaveText([/^writing/, "beta", "alpha", /^Untagged/, "gamma"]);
});
