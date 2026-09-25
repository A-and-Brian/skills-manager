import { describe, expect, it } from "vitest";
import { hasExtension, parentPath, pathBreadcrumbs, pickableEntries } from "./remotePath";

describe("pathBreadcrumbs", () => {
  it("lists every folder from the root", () => {
    expect(pathBreadcrumbs("/home/me/projects")).toEqual([
      { name: "/", path: "/" },
      { name: "home", path: "/home" },
      { name: "me", path: "/home/me" },
      { name: "projects", path: "/home/me/projects" },
    ]);
  });

  it("ignores repeated and trailing slashes", () => {
    expect(pathBreadcrumbs("/srv//data/").map((c) => c.path)).toEqual(["/", "/srv", "/srv/data"]);
    expect(pathBreadcrumbs("/")).toEqual([{ name: "/", path: "/" }]);
  });
});

describe("parentPath", () => {
  it("goes up one folder and stops at the root", () => {
    expect(parentPath("/home/me/projects")).toBe("/home/me");
    expect(parentPath("/home/me/")).toBe("/home");
    expect(parentPath("/home")).toBe("/");
    expect(parentPath("/")).toBeNull();
  });
});

describe("pickableEntries", () => {
  const entries = [
    { name: ".claude", path: "/h/.claude", is_dir: true },
    { name: "skills", path: "/h/skills", is_dir: true },
    { name: "pdf.zip", path: "/h/pdf.zip", is_dir: false },
    { name: "Docs.SKILL", path: "/h/Docs.SKILL", is_dir: false },
    { name: "notes.md", path: "/h/notes.md", is_dir: false },
  ];

  it("shows only folders when choosing a folder", () => {
    expect(pickableEntries(entries, null).map((e) => e.name)).toEqual([".claude", "skills"]);
  });

  it("adds files with an allowed extension, in any case", () => {
    expect(pickableEntries(entries, ["zip", "skill"]).map((e) => e.name)).toEqual([
      ".claude",
      "skills",
      "pdf.zip",
      "Docs.SKILL",
    ]);
  });

  it("matches whole extensions only", () => {
    expect(hasExtension("archive.zip", ["zip"])).toBe(true);
    expect(hasExtension("archivezip", ["zip"])).toBe(false);
    expect(hasExtension("a.zip.md", ["zip"])).toBe(false);
  });
});
