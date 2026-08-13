import {
  EXPANDED_FOLDERS_KEY,
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
