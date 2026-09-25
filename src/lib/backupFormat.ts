/** A snapshot tag (`sm-v-YYYYMMDD-HHMMSS-<suffix>`) without its prefix and suffix. */
export function displaySnapshotLabel(tag: string) {
  const raw = tag.startsWith("sm-v-") ? tag.slice("sm-v-".length) : tag;
  const parts = raw.split("-");
  if (parts.length < 3) return raw;
  return `${parts[0]}-${parts[1]}`;
}

/** The snapshot time as `YYYY-MM-DD HH:MM`, or the plain label if the tag has no time. */
export function formatSnapshotWhen(tag: string | null) {
  if (!tag) return null;
  const label = displaySnapshotLabel(tag);
  const match = label.match(/^(\d{4})(\d{2})(\d{2})-(\d{2})(\d{2})(\d{2})$/);
  if (!match) return label;
  const [, year, month, day, hour, min] = match;
  return `${year}-${month}-${day} ${hour}:${min}`;
}

export function formatBytes(bytes: number) {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(0)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}
