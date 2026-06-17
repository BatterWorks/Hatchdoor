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

  it("returns null for malformed draft JSON", () => {
    window.localStorage.setItem("hatchdoor:draft:note:broken", "{");
    expect(loadNoteDraft("broken")).toBeNull();
  });
});
