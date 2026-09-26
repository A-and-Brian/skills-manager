import { expect, test } from "../fixtures";
import { skill } from "../fake-backend/state";

// F5: batch import from a folder picked in the native dialog, with progress
// from `batch-import-progress` events.

test("batch imports a folder and shows its progress", async ({ page, backend }) => {
  await backend.seed({
    dialogPaths: ["/home/e2e/team-skills"],
    batchImport: [skill("n1", "changelog"), skill("n2", "release-notes")],
  });
  await page.goto("/install?tab=local");
  await expect(page.getByText("Scan Local Skills")).toBeVisible();
  await backend.hold("batch_import_folder");

  await page.getByRole("button", { name: "Batch import from folder" }).click();
  await expect(page.getByText("Scanning for skills...")).toBeVisible();
  expect(await backend.calls("plugin:dialog|open")).toEqual([{ options: { directory: true, multiple: false } }]);
  await expect.poll(() => backend.calls("batch_import_folder")).toEqual([{ folderPath: "/home/e2e/team-skills" }]);

  await backend.emit("batch-import-progress", { current: 1, total: 2, name: "changelog" });
  await expect(page.getByText("Importing 1/2: changelog...")).toBeVisible();
  await backend.emit("batch-import-progress", { current: 2, total: 2, name: "release-notes" });
  await expect(page.getByText("Importing 2/2: release-notes...")).toBeVisible();

  await backend.release("batch_import_folder");
  await expect(page.getByText("Imported 2 skill(s), skipped 0 already-imported")).toBeVisible();

  await page.getByRole("link", { name: "Library" }).click();
  await expect(page.getByRole("heading", { level: 3 })).toHaveText(["changelog", "release-notes"]);
});
