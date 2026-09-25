import type { ManagedSkill } from "./tauri";

/** The https page of a GitHub remote (ssh or https form), or null for other hosts. */
export function githubRepoWebUrl(remoteUrl: string): string | null {
  const match = remoteUrl.match(/github\.com[/:]([^/]+\/[^/]+?)(\.git)?$/);
  return match ? `https://github.com/${match[1]}` : null;
}

/**
 * The installed skill whose source is this git URL. Ignores case and a `.git`
 * suffix, and also matches a source that ends in the same `owner/repo`.
 */
export function findInstalledByGitUrl(skills: ManagedSkill[], url: string): ManagedSkill | undefined {
  const trimmed = url.trim().replace(/\.git$/, "").toLowerCase();
  return skills.find((s) => {
    if (!s.source_ref) return false;
    const ref = s.source_ref.replace(/\.git$/, "").toLowerCase();
    return ref === trimmed || ref.endsWith("/" + trimmed.split("/").slice(-2).join("/"));
  });
}
