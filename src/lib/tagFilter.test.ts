import { describe, expect, it } from "vitest";
import { UNTAGGED_FILTER } from "./skillTags";
import { matchesTagFilter, replaceTagInFilters, tagSuggestions } from "./tagFilter";

describe("matchesTagFilter", () => {
  it("matches everything without a selection", () => {
    expect(matchesTagFilter([], new Set())).toBe(true);
    expect(matchesTagFilter(["docs"], new Set())).toBe(true);
  });

  it("matches a skill carrying any selected tag", () => {
    expect(matchesTagFilter(["docs", "office"], new Set(["office"]))).toBe(true);
    expect(matchesTagFilter(["docs"], new Set(["office"]))).toBe(false);
  });

  it("matches untagged skills only through the untagged pill", () => {
    expect(matchesTagFilter([], new Set([UNTAGGED_FILTER]))).toBe(true);
    expect(matchesTagFilter([], new Set(["office"]))).toBe(false);
    expect(matchesTagFilter(["docs"], new Set([UNTAGGED_FILTER]))).toBe(false);
  });
});

describe("replaceTagInFilters", () => {
  it("returns the same set when the tag is not selected", () => {
    const filters = new Set(["docs"]);
    expect(replaceTagInFilters(filters, "office", "work")).toBe(filters);
  });

  it("renames a selected tag in a new set", () => {
    const filters = new Set(["docs", "office"]);
    const next = replaceTagInFilters(filters, "office", "work");
    expect([...next]).toEqual(["docs", "work"]);
    expect([...filters]).toEqual(["docs", "office"]);
  });

  it("drops a deleted tag", () => {
    expect([...replaceTagInFilters(new Set(["docs", "office"]), "office")]).toEqual(["docs"]);
  });
});

describe("tagSuggestions", () => {
  const allTags = ["Docs", "office", "Work"];

  it("offers every tag the skill lacks when there is no keyword", () => {
    expect(tagSuggestions(allTags, ["office"], "  ")).toEqual(["Docs", "Work"]);
  });

  it("matches the keyword ignoring case", () => {
    expect(tagSuggestions(allTags, [], "dO")).toEqual(["Docs"]);
    expect(tagSuggestions(allTags, [], "WORK")).toEqual(["Work"]);
  });

  it("skips tags the skill already has", () => {
    expect(tagSuggestions(allTags, ["Docs"], "doc")).toEqual([]);
  });
});
