import { expect, test } from "../fixtures";
import { skill } from "../fake-backend/state";
import type { Page } from "@playwright/test";

// F2 + F3: renaming and deleting a tag from the library filter's tag menu, and
// the tag filter following the change.

const seed = {
  skills: [
    skill("s1", "alpha", { tags: ["writing"] }),
    skill("s2", "beta", { tags: ["writing", "draft"] }),
    skill("s3", "gamma"),
  ],
};

const skillNames = (page: Page) => page.getByRole("heading", { level: 3 });
const tagOption = (page: Page, tag: string) => page.getByRole("checkbox", { name: new RegExp(`^${tag}\\b`) });

async function openTagFilter(page: Page) {
  await page.getByRole("button", { name: "Filter" }).click();
  await page.getByRole("dialog", { name: "Filters" }).getByRole("button", { name: /^Tag/ }).click();
}

test.beforeEach(async ({ page, backend }) => {
  await backend.seed(seed);
  await page.goto("/my-skills");
  await expect(skillNames(page)).toHaveText(["alpha", "beta", "gamma"]);
  await openTagFilter(page);
  await tagOption(page, "writing").click();
  await expect(skillNames(page)).toHaveText(["alpha", "beta"]);
});

test("renaming a tag keeps it selected in the filter", async ({ page, backend }) => {
  await tagOption(page, "writing").click({ button: "right" });
  await page.getByRole("button", { name: "Rename tag" }).click();
  await expect(page.getByRole("heading", { name: "Rename tag" })).toBeVisible();
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("prose");
  await page.getByRole("button", { name: "Save" }).click();

  await expect(page.getByText("Tag renamed")).toBeVisible();
  expect(await backend.calls("rename_tag")).toEqual([{ oldName: "writing", newName: "prose" }]);
  await expect(tagOption(page, "writing")).toHaveCount(0);
  await expect(tagOption(page, "prose")).toBeChecked();
  await expect(skillNames(page)).toHaveText(["alpha", "beta"]);
});

test("deleting the selected tag drops it from the filter", async ({ page, backend }) => {
  await tagOption(page, "writing").click({ button: "right" });
  await page.getByRole("button", { name: "Delete tag" }).click();
  await page.getByRole("button", { name: "Delete", exact: true }).click();

  await expect(page.getByText("Tag deleted")).toBeVisible();
  expect(await backend.calls("delete_tag")).toEqual([{ name: "writing" }]);
  await expect(tagOption(page, "writing")).toHaveCount(0);
  await expect(tagOption(page, "draft")).not.toBeChecked();
  await expect(skillNames(page)).toHaveText(["alpha", "beta", "gamma"]);
});
