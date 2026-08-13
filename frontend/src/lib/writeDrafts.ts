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

export type CreateDraft = {
  folder: string;
  name: string;
  content: string;
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
      typeof parsed.content !== "string" ||
      typeof parsed.savedAt !== "number"
    ) {
      return null;
    }
    return {
      folder: parsed.folder,
      name: parsed.name,
      content: parsed.content,
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
