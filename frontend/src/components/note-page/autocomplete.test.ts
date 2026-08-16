import { describe, expect, it } from "vitest";

import {
  applyWikilinkSelection,
  getWikilinkTrigger,
  matchNoteCandidates,
} from "./autocomplete";

describe("getWikilinkTrigger", () => {
  it("detects an open token and returns the query", () => {
    const text = "see [[Pro";
    expect(getWikilinkTrigger(text, text.length)).toEqual({
      query: "Pro",
      start: 4,
    });
  });

  it("returns an empty query right after the brackets", () => {
    const text = "see [[";
    expect(getWikilinkTrigger(text, text.length)).toEqual({
      query: "",
      start: 4,
    });
  });

  it("returns null when there is no open token", () => {
    expect(getWikilinkTrigger("plain text", 10)).toBeNull();
    expect(getWikilinkTrigger("done [[Note]] more", 18)).toBeNull();
    expect(getWikilinkTrigger("multi\n[[A]] [[B", 5)).toBeNull();
  });

  it("ignores tokens interrupted by brackets or newlines", () => {
    expect(getWikilinkTrigger("[[a]b", 5)).toBeNull();
    expect(getWikilinkTrigger("[[a\nb", 5)).toBeNull();
  });
});

describe("applyWikilinkSelection", () => {
  it("replaces the open token with [[title]] and moves the caret", () => {
    const text = "see [[Pro";
    const result = applyWikilinkSelection(text, text.length, 4, "Project Plan");
    expect(result.text).toBe("see [[Project Plan]]");
    expect(result.caret).toBe(result.text.length);
  });

  it("preserves trailing text after the caret", () => {
    const text = "a [[Pr tail";
    // caret right after "Pr" (index 6)
    const result = applyWikilinkSelection(text, 6, 2, "Project");
    expect(result.text).toBe("a [[Project]] tail");
  });
});

describe("matchNoteCandidates", () => {
  const candidates = [
    { vault_id: "vault-1", title: "Project Plan", slug: "project-plan" },
    { vault_id: "vault-1", title: "Projection", slug: "projection" },
    { vault_id: "vault-1", title: "Recipes", slug: "recipes" },
  ];

  it("filters case-insensitively by substring", () => {
    expect(matchNoteCandidates(candidates, "proj").map((n) => n.slug)).toEqual([
      "project-plan",
      "projection",
    ]);
  });

  it("returns all candidates for an empty query, capped by limit", () => {
    expect(matchNoteCandidates(candidates, "", 2)).toHaveLength(2);
  });
});
