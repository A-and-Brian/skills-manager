import { describe, expect, it } from "vitest";
import { parseInstallSearch } from "./installSearch";

describe("parseInstallSearch", () => {
  it("keeps a known tab", () => {
    expect(parseInstallSearch({ tab: "local" })).toEqual({ tab: "local" });
    expect(parseInstallSearch({ tab: "git" })).toEqual({ tab: "git" });
  });

  it("drops a missing, unknown, or non-string tab", () => {
    for (const search of [{}, { tab: "Local" }, { tab: 1 }]) {
      const parsed = parseInstallSearch(search);
      expect(parsed).toHaveProperty("tab");
      expect(parsed.tab).toBeUndefined();
    }
  });
});
