import { afterEach, describe, expect, it } from "vitest";

import {
  clearNoteDraft,
  loadNoteDraft,
  noteDraftKey,
  pruneNoteDrafts,
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

    expect(window.localStorage.getItem("hatchdoor:draft:note:home")).toContain(
      '"slug":"home"',
    );
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

  it("prunes drafts older than the max age and malformed entries", () => {
    const now = 2_000_000_000_000;
    saveNoteDraft("fresh", {
      slug: "fresh",
      content: "keep",
      baseContentHash: "h",
      savedAt: now - 1000,
    });
    saveNoteDraft("stale", {
      slug: "stale",
      content: "drop",
      baseContentHash: "h",
      savedAt: now - 10 * 24 * 60 * 60 * 1000,
    });
    window.localStorage.setItem("hatchdoor:draft:note:broken", "{");
    window.localStorage.setItem("unrelated:key", "keep");

    const removed = pruneNoteDrafts(7 * 24 * 60 * 60 * 1000, now);

    expect(removed).toBe(2);
    expect(loadNoteDraft("fresh")).not.toBeNull();
    expect(loadNoteDraft("stale")).toBeNull();
    expect(
      window.localStorage.getItem("hatchdoor:draft:note:broken"),
    ).toBeNull();
    expect(window.localStorage.getItem("unrelated:key")).toBe("keep");
  });
});
