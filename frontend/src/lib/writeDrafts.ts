export type NoteDraft = {
  vaultId: string;
  slug: string;
  content: string;
  baseContentHash: string;
  savedAt: number;
};

/** Keyed by both `vaultId` and `slug`: a slug is only unique within its own
 * Vault, and without the Vault ID an unsaved draft for one Vault's note could
 * surface — confusingly, not silently, since staleness is still hash-checked
 * — while editing a same-slug note in a different Vault. */
export function noteDraftKey(vaultId: string, slug: string): string {
  return `hatchdoor:draft:note:${vaultId}:${slug}`;
}

export function createDraftKey(): string {
  return "hatchdoor:draft:create";
}

/** A create dialog left half-filled. It carries no body: a note is created
 * empty and written in place, so the only thing a reload can lose is where the
 * note was going to go. `vaultId` is optional because a draft written before
 * the dialog picked a Vault has none. */
export type CreateDraft = {
  vaultId?: string;
  folder: string;
  name: string;
  savedAt: number;
};

export function loadCreateDraft(): CreateDraft | null {
  try {
    const raw = window.localStorage.getItem(createDraftKey());
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<CreateDraft>;
    if (
      typeof parsed.folder !== "string" ||
      typeof parsed.name !== "string" ||
      typeof parsed.savedAt !== "number"
    ) {
      return null;
    }
    return {
      vaultId: typeof parsed.vaultId === "string" ? parsed.vaultId : undefined,
      folder: parsed.folder,
      name: parsed.name,
      savedAt: parsed.savedAt,
    };
  } catch {
    return null;
  }
}

export function saveCreateDraft(draft: CreateDraft): void {
  try {
    window.localStorage.setItem(createDraftKey(), JSON.stringify(draft));
  } catch {
    // Storage can fail in private browsing or when quota is exceeded.
  }
}

export function clearCreateDraft(): void {
  try {
    window.localStorage.removeItem(createDraftKey());
  } catch {
    // Ignore storage failures.
  }
}

export function loadNoteDraft(vaultId: string, slug: string): NoteDraft | null {
  try {
    const raw = window.localStorage.getItem(noteDraftKey(vaultId, slug));
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<NoteDraft>;
    if (
      typeof parsed.vaultId !== "string" ||
      typeof parsed.slug !== "string" ||
      typeof parsed.content !== "string" ||
      typeof parsed.baseContentHash !== "string" ||
      typeof parsed.savedAt !== "number"
    ) {
      return null;
    }
    if (parsed.vaultId !== vaultId || parsed.slug !== slug) {
      return null;
    }
    return {
      vaultId: parsed.vaultId,
      slug: parsed.slug,
      content: parsed.content,
      baseContentHash: parsed.baseContentHash,
      savedAt: parsed.savedAt,
    };
  } catch {
    return null;
  }
}

export function saveNoteDraft(
  vaultId: string,
  slug: string,
  draft: NoteDraft,
): void {
  try {
    window.localStorage.setItem(
      noteDraftKey(vaultId, slug),
      JSON.stringify({ ...draft, vaultId, slug }),
    );
  } catch {
    // Storage can fail in private browsing or when quota is exceeded.
  }
}

export function clearNoteDraft(vaultId: string, slug: string): void {
  try {
    window.localStorage.removeItem(noteDraftKey(vaultId, slug));
  } catch {
    // Ignore storage failures.
  }
}

const NOTE_DRAFT_PREFIX = "hatchdoor:draft:note:";

/**
 * Remove note drafts older than `maxAgeMs`. Drafts are only meant to bridge an
 * interrupted edit; without pruning they accumulate in localStorage forever.
 * Returns the number of drafts removed.
 */
export function pruneNoteDrafts(
  maxAgeMs: number,
  now: number = Date.now(),
): number {
  let removed = 0;
  try {
    const staleKeys: string[] = [];
    for (let i = 0; i < window.localStorage.length; i += 1) {
      const key = window.localStorage.key(i);
      if (!key || !key.startsWith(NOTE_DRAFT_PREFIX)) {
        continue;
      }
      const raw = window.localStorage.getItem(key);
      let savedAt: unknown;
      try {
        savedAt = raw ? (JSON.parse(raw) as Partial<NoteDraft>).savedAt : null;
      } catch {
        savedAt = null;
      }
      if (typeof savedAt !== "number" || now - savedAt > maxAgeMs) {
        staleKeys.push(key);
      }
    }
    for (const key of staleKeys) {
      window.localStorage.removeItem(key);
      removed += 1;
    }
  } catch {
    // Ignore storage failures.
  }
  return removed;
}

/**
 * A draft recovered from before Vault qualification (#137). A pre-#137 note
 * draft was keyed by slug alone, and the standalone create draft has never
 * carried a Vault at all — neither can be trusted to mean the same note (or
 * placed in the same Vault) once notes are addressed by Vault + slug, so both
 * move here for explicit recovery rather than being silently reused. Held
 * drafts never age out; `discardHeldDraft` is the only deletion path.
 */
export type HeldNoteDraft = {
  id: string;
  kind: "note";
  slug: string;
  content: string;
  baseContentHash: string;
  savedAt: number;
};

export type HeldCreateDraft = {
  id: string;
  kind: "create";
  folder: string;
  name: string;
  content: string;
  savedAt: number;
};

export type HeldDraft = HeldNoteDraft | HeldCreateDraft;

const HELD_DRAFT_PREFIX = "hatchdoor:heldDraft:";

function heldDraftKey(id: string): string {
  return `${HELD_DRAFT_PREFIX}${id}`;
}

function saveHeldDraft(draft: HeldDraft): void {
  try {
    window.localStorage.setItem(heldDraftKey(draft.id), JSON.stringify(draft));
  } catch {
    // Storage can fail in private browsing or when quota is exceeded.
  }
}

/** Every held draft, most recently typed first. Malformed entries are skipped. */
export function listHeldDrafts(): HeldDraft[] {
  const drafts: HeldDraft[] = [];
  try {
    for (let i = 0; i < window.localStorage.length; i += 1) {
      const key = window.localStorage.key(i);
      if (!key || !key.startsWith(HELD_DRAFT_PREFIX)) {
        continue;
      }
      const raw = window.localStorage.getItem(key);
      if (!raw) {
        continue;
      }
      try {
        const parsed = JSON.parse(raw) as Partial<HeldDraft>;
        if (
          parsed.kind === "note" &&
          typeof parsed.id === "string" &&
          typeof parsed.slug === "string" &&
          typeof parsed.content === "string" &&
          typeof parsed.baseContentHash === "string" &&
          typeof parsed.savedAt === "number"
        ) {
          drafts.push({
            id: parsed.id,
            kind: "note",
            slug: parsed.slug,
            content: parsed.content,
            baseContentHash: parsed.baseContentHash,
            savedAt: parsed.savedAt,
          });
        } else if (
          parsed.kind === "create" &&
          typeof parsed.id === "string" &&
          typeof parsed.folder === "string" &&
          typeof parsed.name === "string" &&
          typeof parsed.content === "string" &&
          typeof parsed.savedAt === "number"
        ) {
          drafts.push({
            id: parsed.id,
            kind: "create",
            folder: parsed.folder,
            name: parsed.name,
            content: parsed.content,
            savedAt: parsed.savedAt,
          });
        }
      } catch {
        // Skip malformed entries.
      }
    }
  } catch {
    return [];
  }
  return drafts.sort((a, b) => b.savedAt - a.savedAt);
}

export function discardHeldDraft(id: string): void {
  try {
    window.localStorage.removeItem(heldDraftKey(id));
  } catch {
    // Ignore storage failures.
  }
}

/**
 * Sweep pre-#137 drafts into the held-draft store. A legacy note draft's key
 * has no `:` after the shared `hatchdoor:draft:note:` prefix (the current
 * format is `...note:<vaultId>:<slug>`, and a Vault ID never contains `:`),
 * which distinguishes it from an ordinary current-format draft without
 * inspecting its content. The standalone create draft has no Vault-qualified
 * form to compare against — any existing one is legacy by definition.
 *
 * Idempotent and safe to call unconditionally: every source key this moves is
 * deleted as part of the move, so a second call finds nothing left to do.
 * Called once, synchronously, before the app ever renders (`main.tsx`), so
 * every component's first read of `listHeldDrafts` already reflects it.
 */
export function collectLegacyHeldDrafts(): HeldDraft[] {
  try {
    const staleKeys: string[] = [];
    for (let i = 0; i < window.localStorage.length; i += 1) {
      const key = window.localStorage.key(i);
      if (!key || !key.startsWith(NOTE_DRAFT_PREFIX)) {
        continue;
      }
      const remainder = key.slice(NOTE_DRAFT_PREFIX.length);
      if (remainder.includes(":")) {
        continue; // current `<vaultId>:<slug>` shape — not legacy.
      }
      staleKeys.push(key);
      const raw = window.localStorage.getItem(key);
      if (!raw) {
        continue;
      }
      try {
        const parsed = JSON.parse(raw) as {
          slug?: unknown;
          content?: unknown;
          baseContentHash?: unknown;
          savedAt?: unknown;
        };
        if (
          typeof parsed.slug === "string" &&
          typeof parsed.content === "string" &&
          typeof parsed.baseContentHash === "string" &&
          typeof parsed.savedAt === "number"
        ) {
          saveHeldDraft({
            id: `note:${parsed.slug}`,
            kind: "note",
            slug: parsed.slug,
            content: parsed.content,
            baseContentHash: parsed.baseContentHash,
            savedAt: parsed.savedAt,
          });
        }
      } catch {
        // Drop malformed legacy entries without holding them.
      }
    }
    for (const key of staleKeys) {
      window.localStorage.removeItem(key);
    }

    const legacyCreateDraft = loadCreateDraft();
    if (legacyCreateDraft) {
      // A create draft written before the dialog dropped its content box may
      // still hold a typed body. `loadCreateDraft` no longer returns one, so
      // it is read straight off the stored entry rather than lost on the way
      // into the held store.
      let content = "";
      try {
        const raw = window.localStorage.getItem(createDraftKey());
        const parsed = raw ? (JSON.parse(raw) as { content?: unknown }) : null;
        if (typeof parsed?.content === "string") {
          content = parsed.content;
        }
      } catch {
        // Keep the folder and name; an unreadable body holds as empty.
      }
      saveHeldDraft({
        id: "create",
        kind: "create",
        folder: legacyCreateDraft.folder,
        name: legacyCreateDraft.name,
        content,
        savedAt: legacyCreateDraft.savedAt,
      });
      window.localStorage.removeItem(createDraftKey());
    }
  } catch {
    // Ignore storage failures.
  }
  return listHeldDrafts();
}
