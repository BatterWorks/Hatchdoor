export type NoteDraft = {
  slug: string;
  content: string;
  baseContentHash: string;
  savedAt: number;
};

export function noteDraftKey(slug: string): string {
  return `hatchdoor:draft:note:${slug}`;
}

export function createDraftKey(): string {
  return "hatchdoor:draft:create";
}

export function loadNoteDraft(slug: string): NoteDraft | null {
  try {
    const raw = window.localStorage.getItem(noteDraftKey(slug));
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<NoteDraft>;
    if (
      typeof parsed.slug !== "string" ||
      typeof parsed.content !== "string" ||
      typeof parsed.baseContentHash !== "string" ||
      typeof parsed.savedAt !== "number"
    ) {
      return null;
    }
    if (parsed.slug !== slug) {
      return null;
    }
    return {
      slug: parsed.slug,
      content: parsed.content,
      baseContentHash: parsed.baseContentHash,
      savedAt: parsed.savedAt,
    };
  } catch {
    return null;
  }
}

export function saveNoteDraft(slug: string, draft: NoteDraft): void {
  try {
    window.localStorage.setItem(
      noteDraftKey(slug),
      JSON.stringify({ ...draft, slug }),
    );
  } catch {
    // Storage can fail in private browsing or when quota is exceeded.
  }
}

export function clearNoteDraft(slug: string): void {
  try {
    window.localStorage.removeItem(noteDraftKey(slug));
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
export function pruneNoteDrafts(maxAgeMs: number, now: number = Date.now()): number {
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
