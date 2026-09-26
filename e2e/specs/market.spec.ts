import { expect, test } from "../fixtures";
import type { SkillsShSkill } from "../../src/lib/tauri";
import type { Page } from "@playwright/test";

// F2 + F5: the skills.sh market: debounced search, paging and the source
// filter's overflow menu.

const SOURCES = Array.from({ length: 12 }, (_, i) => `contributor-number-${i + 1}/agent-skills`);
const pad = (n: number) => String(n).padStart(2, "0");
// 30 skills: more than one page of 24. Every third one is a "pdf" skill.
const market: SkillsShSkill[] = Array.from({ length: 30 }, (_, i) => {
  const name = i % 3 === 0 ? `pdf-tool-${pad(i + 1)}` : `skill-${pad(i + 1)}`;
  return { id: `m${i + 1}`, skill_id: name, name, source: SOURCES[i % SOURCES.length], installs: 1000 - i };
});

const skillNames = (page: Page) => page.getByRole("heading", { level: 3 });
const search = (page: Page) => page.getByPlaceholder("Search skills.sh market...");

test.beforeEach(async ({ backend }) => {
  await backend.seed({ market });
});

test("pages through the leaderboard", async ({ page }) => {
  await page.goto("/install");
  await expect(skillNames(page)).toHaveCount(24);
  await expect(skillNames(page).first()).toHaveText("pdf-tool-01");

  await page.getByRole("button", { name: "Next" }).click();
  await expect(skillNames(page)).toHaveCount(6);
  await expect(skillNames(page).first()).toHaveText("pdf-tool-25");

  await page.getByRole("button", { name: "1", exact: true }).click();
  await expect(skillNames(page)).toHaveCount(24);
  await expect(skillNames(page).first()).toHaveText("pdf-tool-01");
});

test("searches once the typing stops", async ({ page, backend }) => {
  await page.clock.install();
  await page.goto("/install");
  await expect(skillNames(page)).toHaveCount(24);
  await page.clock.pauseAt(Date.now() + 60_000);

  await search(page).fill("pd");
  await page.clock.runFor(300);
  await search(page).fill("pdf");
  await page.clock.runFor(300);
  expect(await backend.calls("search_skillssh")).toEqual([]);

  // The debounce restarted at "pdf"; let it run out.
  await expect
    .poll(async () => {
      await page.clock.runFor(100);
      return backend.calls("search_skillssh");
    })
    .toEqual([{ query: "pdf", limit: 60 }]);
  await expect(skillNames(page)).toHaveCount(10);
  for (const name of await skillNames(page).allTextContents()) expect(name).toMatch(/^pdf-tool-/);
});

test("filters by a source from the overflow menu, with the mouse", async ({ page }) => {
  await page.goto("/install");
  await page.getByRole("button", { name: /more$/ }).click();
  await page.getByRole("option", { name: "@contributor-number-12/agent-skills" }).click();

  await expect(page.getByRole("listbox")).toHaveCount(0);
  await expect(skillNames(page)).toHaveText(["skill-12", "skill-24"]);
});

test("filters by a source from the overflow menu, with the keyboard", async ({ page }) => {
  await page.goto("/install");
  await page.getByRole("button", { name: /more$/ }).click();
  await page.keyboard.type("number-11/");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");

  await expect(page.getByRole("listbox")).toHaveCount(0);
  await expect(skillNames(page)).toHaveText(["skill-11", "skill-23"]);
});
