import {
  EXPANDED_FOLDERS_KEY,
  EXPLORER_SCROLL_TOP_KEY,
  LAST_NOTE_KEY,
  LEGACY_BROWSER_STATE_CLEARED_KEY,
  RECENT_NOTES_KEY,
  VAULT_SCOPE_KEY,
} from "../app/constants";
import type { RecentNote, VaultScope } from "../types";

export function getStoredNumber(
  key: string,
  fallback: number,
  min: number,
  max: number,
): number {
  const raw = window.localStorage.getItem(key);
  const value = raw ? Number(raw) : fallback;
  if (Number.isNaN(value)) {
    return fallback;
  }
  return clamp(value, min, max);
}

export function getStoredRecentNotes(): RecentNote[] {
  try {
    const raw = window.localStorage.getItem(RECENT_NOTES_KEY);
    if (!raw) {
      return [];
    }
    const parsed = JSON.parse(raw) as Partial<RecentNote>[];
    return parsed
      .filter(
        (item) =>
          typeof item.vaultId === "string" &&
          typeof item.slug === "string" &&
          typeof item.title === "string" &&
          typeof item.relativePath === "string" &&
          typeof item.viewedAt === "number",
      )
      .slice(0, 12)
      .map((item) => ({
        vaultId: item.vaultId as string,
        slug: item.slug as string,
        title: item.title as string,
        relativePath: item.relativePath as string,
        viewedAt: item.viewedAt as number,
      }));
  } catch {
    return [];
  }
}

/** The selected Vault scope (state/URL/storage only per #137 — no chrome
 * reads or writes this yet). Defaults to `"all"`, matching the design spec's
 * stated default; with no chrome to narrow it, every collection read runs
 * under `"all"` until a later slice adds the Scope zone. */
export function getStoredScope(): VaultScope {
  const raw = getStoredString(VAULT_SCOPE_KEY);
  return raw ?? "all";
}

export function setStoredScope(scope: VaultScope): void {
  try {
    window.localStorage.setItem(VAULT_SCOPE_KEY, scope);
  } catch {
    // Ignore storage failures (private mode, disabled storage).
  }
}

export function getStoredExpandedFolders(): Record<string, boolean> {
  try {
    const raw = window.localStorage.getItem(EXPANDED_FOLDERS_KEY);
    if (!raw) {
      return {};
    }
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    const result: Record<string, boolean> = {};
    for (const [key, value] of Object.entries(parsed)) {
      if (key.length > 0 && typeof value === "boolean") {
        result[key] = value;
      }
    }
    return result;
  } catch {
    return {};
  }
}

/** The last note viewed, if any is stored and its shape still checks out.
 * Shared by App.tsx's landing redirect and the explorer accordion's landing
 * default (#142), which both need the same Vault without waiting on each
 * other's effects. */
export function getStoredLastNote(): { vaultId: string; slug: string } | null {
  const raw = getStoredString(LAST_NOTE_KEY);
  if (!raw) {
    return null;
  }
  try {
    const last = JSON.parse(raw) as { vaultId?: unknown; slug?: unknown };
    if (typeof last.vaultId !== "string" || typeof last.slug !== "string") {
      return null;
    }
    return { vaultId: last.vaultId, slug: last.slug };
  } catch {
    return null;
  }
}

export function getStoredString(key: string): string | null {
  const raw = window.localStorage.getItem(key);
  if (!raw) {
    return null;
  }
  const trimmed = raw.trim();
  return trimmed.length > 0 ? trimmed : null;
}

export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }

  const tagName = target.tagName.toLowerCase();
  return (
    tagName === "input" ||
    tagName === "textarea" ||
    tagName === "select" ||
    Boolean(target.isContentEditable)
  );
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function clampSidebarWidth(value: number): number {
  return clamp(value, 220, 420);
}

/**
 * One-time removal of browser state that named a note or folder from before
 * Vault qualification (#137): a bare slug or folder path cannot be trusted to
 * mean the same note or folder once notes are addressed by Vault + slug
 * rather than slug alone. Six Vault-agnostic preferences — theme, sidebar
 * width, the drawer's open state, Recent notes' collapsed state, the
 * touch-edit hint, and the stored bearer token — name neither and are left
 * untouched. Guarded by a persisted marker so a returning user's freshly
 * rebuilt state is never wiped again. Returns whether this call cleared
 * anything.
 */
export function clearLegacyNoteScopedBrowserState(): boolean {
  try {
    if (window.localStorage.getItem(LEGACY_BROWSER_STATE_CLEARED_KEY) === "1") {
      return false;
    }
    window.localStorage.removeItem(RECENT_NOTES_KEY);
    window.localStorage.removeItem(LAST_NOTE_KEY);
    window.localStorage.removeItem(EXPANDED_FOLDERS_KEY);
    window.localStorage.removeItem(EXPLORER_SCROLL_TOP_KEY);
    window.localStorage.setItem(LEGACY_BROWSER_STATE_CLEARED_KEY, "1");
    return true;
  } catch {
    return false;
  }
}
