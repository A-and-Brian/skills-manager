import type { DirectoryEntry } from "./tauri";

// Remote hosts run Linux or macOS, so their paths are POSIX.

export interface Breadcrumb {
  name: string;
  path: string;
}

/** Each folder from the root down to the absolute `path`. */
export function pathBreadcrumbs(path: string): Breadcrumb[] {
  const crumbs: Breadcrumb[] = [{ name: "/", path: "/" }];
  let current = "";
  for (const part of path.split("/").filter(Boolean)) {
    current += `/${part}`;
    crumbs.push({ name: part, path: current });
  }
  return crumbs;
}

/** The folder above the absolute `path`, or null at the root. */
export function parentPath(path: string): string | null {
  const trimmed = path.replace(/\/+$/, "");
  if (!trimmed) return null;
  const cut = trimmed.lastIndexOf("/");
  return cut <= 0 ? "/" : trimmed.slice(0, cut);
}

/** Whether `name` ends in one of `extensions` (given without dots), in any case. */
export function hasExtension(name: string, extensions: string[]): boolean {
  const lower = name.toLowerCase();
  return extensions.some((ext) => lower.endsWith(`.${ext.toLowerCase()}`));
}

/**
 * What a picker shows: folders always, files only when choosing a file and
 * only those with an allowed extension. `extensions` is null for a folder.
 */
export function pickableEntries(entries: DirectoryEntry[], extensions: string[] | null): DirectoryEntry[] {
  return entries.filter((entry) => entry.is_dir || (extensions !== null && hasExtension(entry.name, extensions)));
}
