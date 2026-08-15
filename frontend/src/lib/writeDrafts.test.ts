import { afterEach, describe, expect, it } from "vitest";

import {
  clearNoteDraft,
  collectLegacyHeldDrafts,
  createDraftKey,
  discardHeldDraft,
  listHeldDrafts,
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

  it("collectLegacyHeldDrafts moves a pre-#137 slug-only note draft and leaves a current-format draft alone", () => {
    window.localStorage.setItem(
      "hatchdoor:draft:note:orphaned",
      JSON.stringify({
        slug: "orphaned",
        content: "# Orphaned\nDraft",
        baseContentHash: "abc123",
        savedAt: 1000,
      }),
    );
    saveNoteDraft("vault-1", "current", {
      vaultId: "vault-1",
      slug: "current",
      content: "still active",
      baseContentHash: "def456",
      savedAt: 2000,
    });

    const held = collectLegacyHeldDrafts();

    expect(held).toEqual([
      {
        id: "note:orphaned",
        kind: "note",
        slug: "orphaned",
        content: "# Orphaned\nDraft",
        baseContentHash: "abc123",
        savedAt: 1000,
      },
    ]);
    expect(
      window.localStorage.getItem("hatchdoor:draft:note:orphaned"),
    ).toBeNull();
    // The current-format draft is untouched: still readable through the
    // ordinary per-note lookup, not swept into held drafts.
    expect(loadNoteDraft("vault-1", "current")).not.toBeNull();
    expect(listHeldDrafts()).toEqual(held);
  });

  it("collectLegacyHeldDrafts moves the standalone create draft and is idempotent", () => {
    window.localStorage.setItem(
      createDraftKey(),
      JSON.stringify({
        folder: "10-topics",
        name: "In Progress",
        content: "half-written",
        savedAt: 3000,
      }),
    );

    const held = collectLegacyHeldDrafts();
    expect(held).toEqual([
      {
        id: "create",
        kind: "create",
        folder: "10-topics",
        name: "In Progress",
        content: "half-written",
        savedAt: 3000,
      },
    ]);
    expect(window.localStorage.getItem(createDraftKey())).toBeNull();

    // A second call finds nothing left to migrate.
    expect(collectLegacyHeldDrafts()).toEqual(held);
  });

  it("collectLegacyHeldDrafts drops a malformed legacy entry without holding it", () => {
    window.localStorage.setItem("hatchdoor:draft:note:broken", "{");
    expect(collectLegacyHeldDrafts()).toEqual([]);
    expect(
      window.localStorage.getItem("hatchdoor:draft:note:broken"),
    ).toBeNull();
  });

  it("listHeldDrafts sorts newest first and discardHeldDraft removes exactly one", () => {
    window.localStorage.setItem(
      "hatchdoor:heldDraft:note:a",
      JSON.stringify({
        id: "note:a",
        kind: "note",
        slug: "a",
        content: "a",
        baseContentHash: "h",
        savedAt: 1000,
      }),
    );
    window.localStorage.setItem(
      "hatchdoor:heldDraft:note:b",
      JSON.stringify({
        id: "note:b",
        kind: "note",
        slug: "b",
        content: "b",
        baseContentHash: "h",
        savedAt: 2000,
      }),
    );

    expect(listHeldDrafts().map((draft) => draft.id)).toEqual([
      "note:b",
      "note:a",
    ]);

    discardHeldDraft("note:b");
    expect(listHeldDrafts().map((draft) => draft.id)).toEqual(["note:a"]);
  });
});
