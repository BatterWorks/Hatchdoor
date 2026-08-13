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
  it("persists and clears existing-note drafts by vault and slug", () => {
    const key = noteDraftKey("vault-1", "home");
    expect(key).toBe("hatchdoor:draft:note:vault-1:home");

    saveNoteDraft("vault-1", "home", {
      vaultId: "vault-1",
      slug: "home",
      content: "# Home\nDraft",
      baseContentHash: "abc123",
      savedAt: 1781630000000,
    });

    expect(loadNoteDraft("vault-1", "home")).toEqual({
      vaultId: "vault-1",
      slug: "home",
      content: "# Home\nDraft",
      baseContentHash: "abc123",
      savedAt: 1781630000000,
    });

    clearNoteDraft("vault-1", "home");
    expect(loadNoteDraft("vault-1", "home")).toBeNull();
  });

  it("normalizes mismatched draft vault/slug when saving and rejects mismatched payloads when loading", () => {
    saveNoteDraft("vault-1", "home", {
      vaultId: "vault-2",
      slug: "other",
      content: "# Home\nDraft",
      baseContentHash: "abc123",
      savedAt: 1781630000000,
    });

    expect(
      window.localStorage.getItem("hatchdoor:draft:note:vault-1:home"),
    ).toContain('"vaultId":"vault-1"');
    expect(
      window.localStorage.getItem("hatchdoor:draft:note:vault-1:home"),
    ).toContain('"slug":"home"');
    expect(loadNoteDraft("vault-1", "home")).toEqual({
      vaultId: "vault-1",
      slug: "home",
      content: "# Home\nDraft",
      baseContentHash: "abc123",
      savedAt: 1781630000000,
    });

    window.localStorage.setItem(
      "hatchdoor:draft:note:vault-1:home",
      JSON.stringify({
        vaultId: "vault-1",
        slug: "other",
        content: "# Home\nDraft",
        baseContentHash: "abc123",
        savedAt: 1781630000000,
      }),
    );

    expect(loadNoteDraft("vault-1", "home")).toBeNull();

    window.localStorage.setItem(
      "hatchdoor:draft:note:vault-1:home",
      JSON.stringify({
        vaultId: "vault-2",
        slug: "home",
        content: "# Home\nDraft",
        baseContentHash: "abc123",
        savedAt: 1781630000000,
      }),
    );

    expect(loadNoteDraft("vault-1", "home")).toBeNull();
  });

  it("returns null for malformed draft JSON", () => {
    window.localStorage.setItem("hatchdoor:draft:note:vault-1:broken", "{");
    expect(loadNoteDraft("vault-1", "broken")).toBeNull();
  });

  it("prunes drafts older than the max age and malformed entries", () => {
    const now = 2_000_000_000_000;
    saveNoteDraft("vault-1", "fresh", {
      vaultId: "vault-1",
      slug: "fresh",
      content: "keep",
      baseContentHash: "h",
      savedAt: now - 1000,
    });
    saveNoteDraft("vault-1", "stale", {
      vaultId: "vault-1",
      slug: "stale",
      content: "drop",
      baseContentHash: "h",
      savedAt: now - 10 * 24 * 60 * 60 * 1000,
    });
    window.localStorage.setItem("hatchdoor:draft:note:vault-1:broken", "{");
    window.localStorage.setItem("unrelated:key", "keep");

    const removed = pruneNoteDrafts(7 * 24 * 60 * 60 * 1000, now);

    expect(removed).toBe(2);
    expect(loadNoteDraft("vault-1", "fresh")).not.toBeNull();
    expect(loadNoteDraft("vault-1", "stale")).toBeNull();
    expect(
      window.localStorage.getItem("hatchdoor:draft:note:vault-1:broken"),
    ).toBeNull();
    expect(window.localStorage.getItem("unrelated:key")).toBe("keep");
  });
});
