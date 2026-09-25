/**
 * Orders items by a previously saved list of keys. Saved keys come first in
 * their saved order; saved keys with no matching item are ignored, and items
 * missing from the saved list are appended in their original order.
 */
export function applyStoredOrder<T extends { key: string }>(items: T[], storedOrder: string[]): T[] {
  return [
    ...storedOrder.flatMap((key) => {
      const item = items.find((i) => i.key === key);
      return item ? [item] : [];
    }),
    ...items.filter((i) => !storedOrder.includes(i.key)),
  ];
}
