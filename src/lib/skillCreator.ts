/**
 * Who made a skill, worked out from where it was installed from. A skill from
 * GitHub (skills.sh or git) is credited to the repo owner; another git host
 * gets the owner without an avatar; anything else falls back to the SKILL.md
 * frontmatter author, then to "Local".
 */

export type SkillCreator =
  | { kind: "github"; owner: string; repo: string; url: string }
  | { kind: "host"; host: string; owner: string; repo: string; url: string }
  | { kind: "author"; name: string }
  | { kind: "local" };

/** The fields a creator is read from; library, project and remote skills all have them. */
export interface CreatorSource {
  source_type: string;
  source_ref: string | null;
  source_ref_resolved?: string | null;
  author?: string | null;
}

/** Group and filter key for skills with no known creator. */
export const LOCAL_CREATOR = "__local__";

const SEGMENT = /^[\w.-]+$/;

function repoCreator(host: string, path: string): SkillCreator | null {
  const [owner, rawRepo] = path.split("/").filter(Boolean);
  const repo = rawRepo?.replace(/\.git$/i, "");
  if (!owner || !repo || !SEGMENT.test(owner) || !SEGMENT.test(repo)) return null;
  if (host === "github.com" || host === "www.github.com") {
    return { kind: "github", owner, repo, url: `https://github.com/${owner}/${repo}` };
  }
  return { kind: "host", host, owner, repo, url: `https://${host}/${owner}/${repo}` };
}

/**
 * Read host, owner and repo from a git source as the user typed it or as it
 * was resolved: `owner/repo`, `host/owner/repo`, `https://…` (with or without
 * `.git`, credentials or a `/tree/<branch>/…` suffix), `git@host:owner/repo`
 * and `ssh://…`. Filesystem paths give `null`.
 */
function gitCreator(ref: string): SkillCreator | null {
  const value = ref.trim();
  const scp = value.match(/^[\w.-]+@([^:/]+):(.+)$/);
  if (scp) return repoCreator(scp[1].toLowerCase(), scp[2]);

  const url = value.match(/^[a-z][a-z\d+.-]*:\/\/([^/]*)(.*)$/i);
  if (url) {
    // Drop credentials (`user:token@`) and the port from the authority.
    const host = url[1].slice(url[1].lastIndexOf("@") + 1).replace(/:\d*$/, "").toLowerCase();
    return host ? repoCreator(host, url[2]) : null;
  }

  if (/^[/.~]/.test(value) || value.includes("\\")) return null;
  const [first, ...rest] = value.split("/");
  return first.includes(".") && rest.length >= 2
    ? repoCreator(first.toLowerCase(), rest.join("/"))
    : repoCreator("github.com", value);
}

export function skillCreator(skill: CreatorSource): SkillCreator {
  let fromSource: SkillCreator | null = null;
  if (skill.source_type === "skillssh" && skill.source_ref) {
    // skills.sh refs are `owner/repo/skill_id` on GitHub.
    fromSource = repoCreator("github.com", skill.source_ref);
  } else if (skill.source_type === "git") {
    const ref = skill.source_ref_resolved || skill.source_ref;
    fromSource = ref ? gitCreator(ref) : null;
  }
  if (fromSource) return fromSource;
  const name = skill.author?.trim();
  return name ? { kind: "author", name } : { kind: "local" };
}

/** The owner or author name, for sorting; empty for local. */
export function creatorName(creator: SkillCreator): string {
  if (creator.kind === "github" || creator.kind === "host") return creator.owner;
  return creator.kind === "author" ? creator.name : "";
}

/** `@owner` or the author's name; empty for local, which is labelled by the UI. */
export function creatorLabel(creator: SkillCreator): string {
  const name = creatorName(creator);
  return creator.kind === "github" || creator.kind === "host" ? `@${name}` : name;
}

/** Stable, case-insensitive key for grouping and filtering by creator. */
export function creatorKey(creator: SkillCreator): string {
  switch (creator.kind) {
    case "github":
      return `github.com/${creator.owner.toLowerCase()}`;
    case "host":
      return `${creator.host}/${creator.owner.toLowerCase()}`;
    case "author":
      return `author:${creator.name.toLowerCase()}`;
    default:
      return LOCAL_CREATOR;
  }
}

export function creatorKeyOf(skill: CreatorSource): string {
  return creatorKey(skillCreator(skill));
}
