import { describe, expect, it } from "vitest";
import { isForActiveHost } from "./hostEvents";

describe("isForActiveHost", () => {
  it("accepts this computer's events only while it is shown", () => {
    expect(isForActiveHost({ skill_id: "x", phase: "cloning" }, null)).toBe(true);
    expect(isForActiveHost(null, null)).toBe(true);
    expect(isForActiveHost({ skill_id: "x", phase: "cloning" }, "box")).toBe(false);
    expect(isForActiveHost(null, "box")).toBe(false);
  });

  it("accepts a host's events only while that host is active", () => {
    expect(isForActiveHost({ host_id: "box", current: 1 }, "box")).toBe(true);
    expect(isForActiveHost({ host_id: "box" }, "other")).toBe(false);
    expect(isForActiveHost({ host_id: "box" }, null)).toBe(false);
  });
});
