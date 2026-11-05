import { afterEach, describe, expect, it, vi } from "vitest";

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
  it("persists and clears existing-note drafts by vault and path", () => {
    const key = noteDraftKey("vault-1", "Notes/Home");
    expect(key).toBe("hatchdoor:draft:note-path:vault-1:Notes/Home");

    saveNoteDraft("vault-1", "Notes/Home", {
      vaultId: "vault-1",
      relativePath: "Notes/Home",
      content: "# Home\nDraft",
      baseContentHash: "abc123",
      savedAt: 1781630000000,
    });

    expect(loadNoteDraft("vault-1", "Notes/Home")).toEqual({
      vaultId: "vault-1",
      relativePath: "Notes/Home",
      content: "# Home\nDraft",
      baseContentHash: "abc123",
      savedAt: 1781630000000,
    });

    clearNoteDraft("vault-1", "Notes/Home");
    expect(loadNoteDraft("vault-1", "Notes/Home")).toBeNull();
  });

  // #627: two notes can share a name, and a slug moves between them. A draft
  // typed into one is never offered to the other.
  it("keeps drafts for two same-named notes apart", () => {
    saveNoteDraft("vault-1", "z/Plan", {
      vaultId: "vault-1",
      relativePath: "z/Plan",
      content: "typed in z",
      baseContentHash: "same",
      savedAt: 1,
    });

    expect(loadNoteDraft("vault-1", "a/Plan")).toBeNull();
    expect(loadNoteDraft("vault-1", "z/Plan")?.content).toBe("typed in z");
  });

  it("normalizes mismatched draft vault/path when saving and rejects mismatched payloads when loading", () => {
    const key = "hatchdoor:draft:note-path:vault-1:Home";
    saveNoteDraft("vault-1", "Home", {
      vaultId: "vault-2",
      relativePath: "Other",
      content: "# Home\nDraft",
      baseContentHash: "abc123",
      savedAt: 1781630000000,
    });

    expect(window.localStorage.getItem(key)).toContain('"vaultId":"vault-1"');
    expect(window.localStorage.getItem(key)).toContain('"relativePath":"Home"');
    expect(loadNoteDraft("vault-1", "Home")).toEqual({
      vaultId: "vault-1",
      relativePath: "Home",
      content: "# Home\nDraft",
      baseContentHash: "abc123",
      savedAt: 1781630000000,
    });

    window.localStorage.setItem(
      key,
      JSON.stringify({
        vaultId: "vault-1",
        relativePath: "Other",
        content: "# Home\nDraft",
        baseContentHash: "abc123",
        savedAt: 1781630000000,
      }),
    );
    expect(loadNoteDraft("vault-1", "Home")).toBeNull();

    window.localStorage.setItem(
      key,
      JSON.stringify({
        vaultId: "vault-2",
        relativePath: "Home",
        content: "# Home\nDraft",
        baseContentHash: "abc123",
        savedAt: 1781630000000,
      }),
    );
    expect(loadNoteDraft("vault-1", "Home")).toBeNull();
  });

  it("returns null for malformed draft JSON", () => {
    window.localStorage.setItem(
      "hatchdoor:draft:note-path:vault-1:broken",
      "{",
    );
    expect(loadNoteDraft("vault-1", "broken")).toBeNull();
  });

  it("prunes drafts older than the max age and malformed entries", () => {
    const now = 2_000_000_000_000;
    saveNoteDraft("vault-1", "fresh", {
      vaultId: "vault-1",
      relativePath: "fresh",
      content: "keep",
      baseContentHash: "h",
      savedAt: now - 1000,
    });
    saveNoteDraft("vault-1", "stale", {
      vaultId: "vault-1",
      relativePath: "stale",
      content: "drop",
      baseContentHash: "h",
      savedAt: now - 10 * 24 * 60 * 60 * 1000,
    });
    window.localStorage.setItem(
      "hatchdoor:draft:note-path:vault-1:broken",
      "{",
    );
    window.localStorage.setItem("hatchdoor:draft:note:vault-1:old", "{");
    window.localStorage.setItem("unrelated:key", "keep");

    const removed = pruneNoteDrafts(7 * 24 * 60 * 60 * 1000, now);

    expect(removed).toBe(3);
    expect(loadNoteDraft("vault-1", "fresh")).not.toBeNull();
    expect(loadNoteDraft("vault-1", "stale")).toBeNull();
    expect(
      window.localStorage.getItem("hatchdoor:draft:note-path:vault-1:broken"),
    ).toBeNull();
    expect(
      window.localStorage.getItem("hatchdoor:draft:note:vault-1:old"),
    ).toBeNull();
    expect(window.localStorage.getItem("unrelated:key")).toBe("keep");
  });

  it("collectLegacyHeldDrafts holds every slug-keyed note draft and leaves a path-keyed draft alone", () => {
    window.localStorage.setItem(
      "hatchdoor:draft:note:orphaned",
      JSON.stringify({
        slug: "orphaned",
        content: "# Orphaned\nDraft",
        baseContentHash: "abc123",
        savedAt: 1000,
      }),
    );
    // #627: a Vault-qualified slug draft cannot say which file it was typed
    // into either, so it is held rather than restored into the slug's holder.
    window.localStorage.setItem(
      "hatchdoor:draft:note:vault-1:plan",
      JSON.stringify({
        vaultId: "vault-1",
        slug: "plan",
        content: "typed somewhere",
        baseContentHash: "same",
        savedAt: 1500,
      }),
    );
    saveNoteDraft("vault-1", "current", {
      vaultId: "vault-1",
      relativePath: "current",
      content: "still active",
      baseContentHash: "def456",
      savedAt: 2000,
    });

    const held = collectLegacyHeldDrafts();

    expect(held).toEqual([
      {
        id: "note:vault-1:plan",
        kind: "note",
        slug: "plan",
        content: "typed somewhere",
        baseContentHash: "same",
        savedAt: 1500,
      },
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
    expect(
      window.localStorage.getItem("hatchdoor:draft:note:vault-1:plan"),
    ).toBeNull();
    // The path-keyed draft is untouched: still readable through the ordinary
    // per-note lookup, not swept into held drafts.
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

describe("saveNoteDraft reports storage failures (#330)", () => {
  it("returns true when the draft lands and false when the store refuses it", () => {
    expect(
      saveNoteDraft("vault-1", "home", {
        vaultId: "vault-1",
        relativePath: "home",
        content: "kept",
        baseContentHash: "abc",
        savedAt: 1,
      }),
    ).toBe(true);

    const setItem = vi
      .spyOn(Storage.prototype, "setItem")
      .mockImplementation(() => {
        throw new DOMException("quota exceeded", "QuotaExceededError");
      });
    try {
      expect(
        saveNoteDraft("vault-1", "home", {
          vaultId: "vault-1",
          relativePath: "home",
          content: "lost",
          baseContentHash: "abc",
          savedAt: 2,
        }),
      ).toBe(false);
    } finally {
      setItem.mockRestore();
    }
  });
});

describe("collectLegacyHeldDrafts migrates completely (#330)", () => {
  function seedLegacy(slug: string, savedAt: number): void {
    window.localStorage.setItem(
      `hatchdoor:draft:note:${slug}`,
      JSON.stringify({
        slug,
        content: `draft for ${slug}`,
        baseContentHash: `hash-${slug}`,
        savedAt,
      }),
    );
  }

  // The regression: the migration used to call `saveHeldDraft` from inside the
  // `key(i)` loop, inserting a key into the very storage area it was
  // enumerating. A storage area is a hash map with an implementation-defined
  // `key(i)` order, so a write mid-enumeration can shift entries behind the
  // cursor and legacy drafts go unseen — and unseen means not deleted either,
  // so `listHeldDrafts()` under-reports on the one run that mattered. jsdom's
  // Storage is insertion-ordered and will not reproduce that reordering, so
  // the guarantee is asserted directly: nothing is written until the last key
  // has been read.
  it("does not write to localStorage while enumerating it", () => {
    seedLegacy("alpha", 1000);
    seedLegacy("beta", 2000);
    seedLegacy("gamma", 3000);

    const events: string[] = [];
    const originalKey = Storage.prototype.key;
    const originalSetItem = Storage.prototype.setItem;
    const key = vi.spyOn(Storage.prototype, "key").mockImplementation(function (
      this: Storage,
      index: number,
    ) {
      events.push("key");
      return originalKey.call(this, index);
    });
    const setItem = vi
      .spyOn(Storage.prototype, "setItem")
      .mockImplementation(function (
        this: Storage,
        name: string,
        value: string,
      ) {
        events.push("setItem");
        originalSetItem.call(this, name, value);
      });

    try {
      collectLegacyHeldDrafts();
    } finally {
      key.mockRestore();
      setItem.mockRestore();
    }

    // Every key of the three seeded drafts is read before the first write. The
    // unfixed version wrote after reading exactly one.
    const readsBeforeFirstWrite = events
      .slice(0, events.indexOf("setItem"))
      .filter((event) => event === "key").length;
    expect(events).toContain("setItem");
    expect(readsBeforeFirstWrite).toBeGreaterThanOrEqual(3);
  });

  it("holds every legacy draft regardless of how many there are", () => {
    seedLegacy("alpha", 1000);
    seedLegacy("beta", 2000);
    seedLegacy("gamma", 3000);

    const held = collectLegacyHeldDrafts();

    expect(held.map((draft) => draft.id).sort()).toEqual([
      "note:alpha",
      "note:beta",
      "note:gamma",
    ]);
    for (const slug of ["alpha", "beta", "gamma"]) {
      expect(
        window.localStorage.getItem(`hatchdoor:draft:note:${slug}`),
      ).toBeNull();
    }
  });
});
