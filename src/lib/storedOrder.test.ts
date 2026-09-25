import { describe, expect, it } from "vitest";
import { applyStoredOrder } from "./storedOrder";

function items(...keys: string[]) {
  return keys.map((key) => ({ key }));
}

function keysOf(list: { key: string }[]) {
  return list.map((item) => item.key);
}

describe("applyStoredOrder", () => {
  it("puts stored keys first, in stored order", () => {
    expect(keysOf(applyStoredOrder(items("a", "b", "c"), ["c", "a", "b"]))).toEqual(["c", "a", "b"]);
  });

  it("ignores stored keys with no matching item", () => {
    expect(keysOf(applyStoredOrder(items("a", "b"), ["gone", "b", "a"]))).toEqual(["b", "a"]);
  });

  it("appends items missing from the stored order in their original order", () => {
    expect(keysOf(applyStoredOrder(items("a", "b", "c", "d"), ["c"]))).toEqual(["c", "a", "b", "d"]);
  });

  it("keeps the original order when nothing is stored", () => {
    expect(keysOf(applyStoredOrder(items("a", "b"), []))).toEqual(["a", "b"]);
  });
});
