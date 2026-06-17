import { afterEach, describe, expect, it } from "vitest";

import {
  clearNoteDraft,
  loadNoteDraft,
  noteDraftKey,
  saveNoteDraft,
} from "./writeDrafts";

afterEach(() => {
  window.localStorage.clear();
});

describe("writeDrafts", () => {
  it("persists and clears existing-note drafts by slug", () => {
    const key = noteDraftKey("home");
    expect(key).toBe("hatchdoor:draft:note:home");

    saveNoteDraft("home", {
      slug: "home",
      content: "# Home\nDraft",
      baseContentHash: "abc123",
      savedAt: 1781630000000,
    });

    expect(loadNoteDraft("home")).toEqual({
      slug: "home",
      content: "# Home\nDraft",
      baseContentHash: "abc123",
      savedAt: 1781630000000,
    });

    clearNoteDraft("home");
    expect(loadNoteDraft("home")).toBeNull();
  });

  it("normalizes mismatched draft slugs when saving and rejects mismatched payloads when loading", () => {
    saveNoteDraft("home", {
      slug: "other",
      content: "# Home\nDraft",
      baseContentHash: "abc123",
      savedAt: 1781630000000,
    });

    expect(
      window.localStorage.getItem("hatchdoor:draft:note:home"),
    ).toContain('"slug":"home"');
    expect(loadNoteDraft("home")).toEqual({
      slug: "home",
      content: "# Home\nDraft",
      baseContentHash: "abc123",
      savedAt: 1781630000000,
    });

    window.localStorage.setItem(
      "hatchdoor:draft:note:home",
      JSON.stringify({
        slug: "other",
        content: "# Home\nDraft",
        baseContentHash: "abc123",
        savedAt: 1781630000000,
      }),
    );

    expect(loadNoteDraft("home")).toBeNull();
  });

  it("returns null for malformed draft JSON", () => {
    window.localStorage.setItem("hatchdoor:draft:note:broken", "{");
    expect(loadNoteDraft("broken")).toBeNull();
  });
});
