import { describe, expect, it } from "vitest";
import {
  copyCreator,
  creatorKeyOf,
  creatorLabel,
  LOCAL_CREATOR,
  skillCreator,
  type CreatorSource,
  type SkillCreator,
} from "./skillCreator";

const git = (source_ref: string | null, source_ref_resolved: string | null = null): CreatorSource => ({
  source_type: "git",
  source_ref,
  source_ref_resolved,
});

const acmeTools: SkillCreator = { kind: "github", owner: "acme", repo: "tools", url: "https://github.com/acme/tools" };

describe("skillCreator", () => {
  it("credits a skills.sh skill to the GitHub repo owner", () => {
    expect(skillCreator({ source_type: "skillssh", source_ref: "acme/tools/pdf" })).toEqual(acmeTools);
  });

  it.each([
    "acme/tools",
    "https://github.com/acme/tools",
    "https://github.com/acme/tools.git",
    "https://github.com/acme/tools/",
    "https://github.com/acme/tools/tree/main/skills/pdf",
    "git@github.com:acme/tools.git",
    "ssh://git@github.com:22/acme/tools.git",
    "github.com/acme/tools",
    "https://x-access-token:secret@github.com/acme/tools.git",
  ])("resolves the git source %s to the same GitHub owner and link", (ref) => {
    expect(skillCreator(git(ref))).toEqual(acmeTools);
  });

  it("prefers the resolved URL over the raw source", () => {
    expect(skillCreator(git("old/name", "https://github.com/acme/tools.git"))).toEqual(acmeTools);
  });

  it("links other git hosts without treating them as GitHub", () => {
    expect(skillCreator(git("https://gitlab.com/acme/tools.git"))).toEqual({
      kind: "host",
      host: "gitlab.com",
      owner: "acme",
      repo: "tools",
      url: "https://gitlab.com/acme/tools",
    });
  });

  it("falls back to the frontmatter author, then to local", () => {
    expect(skillCreator({ source_type: "local", source_ref: "/Users/me/skills/pdf", author: " Jane " })).toEqual({
      kind: "author",
      name: "Jane",
    });
    expect(skillCreator({ source_type: "import", source_ref: null })).toEqual({ kind: "local" });
    expect(skillCreator({ source_type: "local", source_ref: null, author: "  " })).toEqual({ kind: "local" });
  });

  it("uses the author when a git source cannot be read", () => {
    expect(skillCreator({ ...git("/tmp/some/repo"), author: "Jane" })).toEqual({ kind: "author", name: "Jane" });
    expect(skillCreator(git("not a url"))).toEqual({ kind: "local" });
  });
});

describe("copyCreator", () => {
  it("credits a project copy like its library skill, else by its own author", () => {
    const library: CreatorSource = { source_type: "skillssh", source_ref: "acme/tools/pdf", author: "Someone" };
    expect(copyCreator(library, "Jane")).toEqual(acmeTools);
    expect(copyCreator(undefined, "Jane")).toEqual({ kind: "author", name: "Jane" });
    expect(copyCreator(undefined, null)).toEqual({ kind: "local" });
  });
});

describe("creator labels and keys", () => {
  it("labels owners with @ and authors by name", () => {
    expect(creatorLabel(acmeTools)).toBe("@acme");
    expect(creatorLabel({ kind: "author", name: "Jane" })).toBe("Jane");
    expect(creatorLabel({ kind: "local" })).toBe("");
  });

  it("folds case so one owner is one key, and keeps local on its sentinel", () => {
    expect(creatorKeyOf(git("Acme/tools"))).toBe(creatorKeyOf(git("https://github.com/acme/other")));
    expect(creatorKeyOf({ source_type: "local", source_ref: null, author: "Jane" })).toBe(
      creatorKeyOf({ source_type: "import", source_ref: null, author: "jane" })
    );
    expect(creatorKeyOf({ source_type: "local", source_ref: null })).toBe(LOCAL_CREATOR);
  });
});
