import { describe, expect, it } from "vitest";
import type { ManagedSkill } from "./tauri";
import { findInstalledByGitUrl, githubRepoWebUrl } from "./gitUrl";

function skill(id: string, source_ref: string | null): ManagedSkill {
  return { id, name: id, source_ref } as ManagedSkill;
}

describe("githubRepoWebUrl", () => {
  it("builds the page URL from ssh and https remotes", () => {
    expect(githubRepoWebUrl("git@github.com:me/backup.git")).toBe("https://github.com/me/backup");
    expect(githubRepoWebUrl("https://github.com/me/backup.git")).toBe("https://github.com/me/backup");
    expect(githubRepoWebUrl("https://github.com/me/backup")).toBe("https://github.com/me/backup");
  });

  it("is null for other hosts or an empty remote", () => {
    expect(githubRepoWebUrl("git@gitlab.com:me/backup.git")).toBeNull();
    expect(githubRepoWebUrl("")).toBeNull();
  });
});

describe("findInstalledByGitUrl", () => {
  const skills = [
    skill("local", null),
    skill("https", "https://github.com/Acme/Skills.git"),
    skill("ssh", "git@github.com:acme/tools.git"),
  ];

  it("ignores case, surrounding space and a .git suffix", () => {
    expect(findInstalledByGitUrl(skills, " https://github.com/acme/skills ")?.id).toBe("https");
  });

  it("matches a source ending in the same owner/repo", () => {
    expect(findInstalledByGitUrl(skills, "acme/skills")?.id).toBe("https");
    expect(findInstalledByGitUrl(skills, "https://mirror.example.com/acme/skills.git")?.id).toBe("https");
  });

  it("matches ssh sources by the full URL", () => {
    expect(findInstalledByGitUrl(skills, "git@github.com:ACME/tools.git")?.id).toBe("ssh");
  });

  it("finds nothing for an unknown repo", () => {
    expect(findInstalledByGitUrl(skills, "https://github.com/acme/other")).toBeUndefined();
  });
});
