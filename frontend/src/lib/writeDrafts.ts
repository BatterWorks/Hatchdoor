/** Current note drafts, keyed by Vault and path (#627). */
const NOTE_DRAFT_PREFIX = "hatchdoor:draft:note-path:";
/** Earlier note drafts, keyed by slug: `<slug>` before #137, then
 * `<vaultId>:<slug>`. Neither is read as a note's draft any more; both are
 * swept into the held-draft store by `collectLegacyHeldDrafts`. */
const SLUG_DRAFT_PREFIX = "hatchdoor:draft:note:";

export type NoteDraft = {
  vaultId: string;
  /** The Vault-relative path of the note the draft was typed into, as the
   * note read reports it. */
  relativePath: string;
  content: string;
  baseContentHash: string;
  savedAt: number;
};

/** Keyed by Vault and Vault-relative path (#627). A slug is not a note's
 * identity: it moves to another file when a same-named note appears earlier in
 * path order (ADR-24), and a draft keyed by it was restored into, and
 * autosaved into, whichever note held the slug next. A path names one file. */
export function noteDraftKey(vaultId: string, relativePath: string): string {
  return `${NOTE_DRAFT_PREFIX}${vaultId}:${relativePath}`;
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

export function loadNoteDraft(
  vaultId: string,
  relativePath: string,
): NoteDraft | null {
  try {
    const raw = window.localStorage.getItem(
      noteDraftKey(vaultId, relativePath),
    );
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<NoteDraft>;
    if (
      typeof parsed.vaultId !== "string" ||
      typeof parsed.relativePath !== "string" ||
      typeof parsed.content !== "string" ||
      typeof parsed.baseContentHash !== "string" ||
      typeof parsed.savedAt !== "number"
    ) {
      return null;
    }
    if (parsed.vaultId !== vaultId || parsed.relativePath !== relativePath) {
      return null;
    }
    return {
      vaultId: parsed.vaultId,
      relativePath: parsed.relativePath,
      content: parsed.content,
      baseContentHash: parsed.baseContentHash,
      savedAt: parsed.savedAt,
    };
  } catch {
    return null;
  }
}

/**
 * Write a note draft, reporting whether it actually landed.
 *
 * The boolean is the point (#330): with site data blocked, storage full, or a
 * browser set to clear on exit, a swallowed failure looks exactly like a
 * working store, and the editor goes on promising a safety net that does not
 * exist.
 *
 * Only `NotePage`'s debounced editor draft writer reads it today, and it is the
 * one that matters: it is the writer behind the promise the UI makes while the
 * user types. The other callers — reload-latest, the two conflict resolutions,
 * and Settings' held-draft restore — still discard the result, so a blocked
 * store is silent on those paths. Surfacing it there is unfinished work, and
 * the held-draft restore is the one to start with: it discards the held draft
 * straight after writing the per-note one.
 */
export function saveNoteDraft(
  vaultId: string,
  relativePath: string,
  draft: NoteDraft,
): boolean {
  try {
    window.localStorage.setItem(
      noteDraftKey(vaultId, relativePath),
      JSON.stringify({ ...draft, vaultId, relativePath }),
    );
    return true;
  } catch {
    // Storage can fail in private browsing or when quota is exceeded.
    return false;
  }
}

export function clearNoteDraft(vaultId: string, relativePath: string): void {
  try {
    window.localStorage.removeItem(noteDraftKey(vaultId, relativePath));
  } catch {
    // Ignore storage failures.
  }
}

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
      if (
        !key ||
        !(
          key.startsWith(NOTE_DRAFT_PREFIX) || key.startsWith(SLUG_DRAFT_PREFIX)
        )
      ) {
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
 * A note draft that cannot be trusted to mean the note it would be restored
 * into. A pre-#137 draft was keyed by slug alone, so it names no Vault; a
 * draft from #137 to #627 was keyed by Vault and slug, and a slug moves to
 * another file when a same-named note appears earlier in path order (ADR-24).
 * The standalone create draft has never carried a Vault at all. All of them
 * move here for explicit recovery rather than being silently reused. Held
 * drafts never age out; `discardHeldDraft` is the only deletion path.
 */
export type HeldNoteDraft = {
  id: string;
  kind: "note";
  slug: string;
  /** Set when the draft was held by path (#627): the Vault-relative path of
   * the file it was typed into. Restore looks that path up, never `slug`. */
  relativePath?: string;
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
            ...(typeof parsed.relativePath === "string"
              ? { relativePath: parsed.relativePath }
              : {}),
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

/**
 * Move one note's path-keyed draft into the held-draft store (#627), for a
 * note page about to open another file at the same address. The file the
 * draft was typed into may be gone or moved, and no page will look for it
 * under its old path, so it goes where Settings lists it for explicit
 * recovery. The held entry carries the old path as `relativePath`: restore
 * looks that exact path up and, finding no note there, offers to create one
 * at it. Its `slug` holds the same path, only for display. The id carries
 * the time, so holding the same path twice keeps both. Returns whether a
 * draft was held.
 */
export function holdNoteDraft(vaultId: string, relativePath: string): boolean {
  const draft = loadNoteDraft(vaultId, relativePath);
  if (!draft) {
    return false;
  }
  saveHeldDraft({
    id: `note-path:${vaultId}:${relativePath}:${draft.savedAt}`,
    kind: "note",
    slug: relativePath,
    relativePath,
    content: draft.content,
    baseContentHash: draft.baseContentHash,
    savedAt: draft.savedAt,
  });
  clearNoteDraft(vaultId, relativePath);
  return true;
}

export function discardHeldDraft(id: string): void {
  try {
    window.localStorage.removeItem(heldDraftKey(id));
  } catch {
    // Ignore storage failures.
  }
}

/**
 * Sweep slug-keyed note drafts into the held-draft store: every key under
 * `hatchdoor:draft:note:`, whether pre-#137 (`<slug>`) or Vault-qualified
 * (`<vaultId>:<slug>`). Current drafts live under `hatchdoor:draft:note-path:`
 * and are never touched. The standalone create draft has no Vault-qualified
 * form to compare against, so any existing one is legacy by definition.
 *
 * Idempotent and safe to call unconditionally: every source key this moves is
 * deleted as part of the move, so a second call finds nothing left to do.
 * Called once, synchronously, before the app ever renders (`main.tsx`), so
 * every component's first read of `listHeldDrafts` already reflects it.
 *
 * Two-phase, like `pruneNoteDrafts` above, and for the same reason (#330): a
 * storage area is enumerated by index over an implementation-defined order,
 * and writing a new key into it mid-loop can reorder or shift the entries
 * behind the cursor. Nothing is written until every key has been read, so the
 * count of legacy drafts cannot change what is found.
 */
export function collectLegacyHeldDrafts(): HeldDraft[] {
  try {
    const staleKeys: string[] = [];
    const legacyRaw: string[] = [];
    for (let i = 0; i < window.localStorage.length; i += 1) {
      const key = window.localStorage.key(i);
      if (!key || !key.startsWith(SLUG_DRAFT_PREFIX)) {
        continue;
      }
      staleKeys.push(key);
      const raw = window.localStorage.getItem(key);
      if (raw) {
        legacyRaw.push(raw);
      }
    }
    for (const raw of legacyRaw) {
      try {
        const parsed = JSON.parse(raw) as {
          vaultId?: unknown;
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
          // Two Vaults can each have held a draft under the same slug.
          const vaultPart =
            typeof parsed.vaultId === "string" ? `${parsed.vaultId}:` : "";
          saveHeldDraft({
            id: `note:${vaultPart}${parsed.slug}`,
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
