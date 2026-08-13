import { getStoredLastNote, getStoredString } from "../lib/storage";
import { pathToNoteIdentity } from "../lib/notePath";
import { LAST_UNFOLDED_VAULT_KEY } from "./constants";
import type { VaultId, VaultSummary } from "../types";

/** A Vault whose activation is "unavailable" keeps its accordion head but
 * never unfolds (#142) — the same signal `deriveVaultSlot`'s first branch
 * uses for the "unavailable" condition word. */
export function isVaultUnfoldable(vault: VaultSummary): boolean {
  return vault.activation !== "unavailable";
}

/**
 * Landing default for the accordion's unfolded Vault: the open note's own
 * Vault first, else the last one persisted per browser, else nothing. A
 * candidate that no longer exists among the enabled Vaults, or that cannot
 * unfold, resolves to nothing rather than a stale or dead reference.
 */
export function resolveInitialUnfoldedVault(
  landingVaultId: VaultId | undefined,
  storedVaultId: VaultId | null,
  vaults: VaultSummary[],
): VaultId | undefined {
  const candidate = landingVaultId ?? storedVaultId ?? undefined;
  if (!candidate) {
    return undefined;
  }
  const vault = vaults.find((entry) => entry.vault_id === candidate);
  if (!vault || !isVaultUnfoldable(vault)) {
    return undefined;
  }
  return candidate;
}

/**
 * The Vault "landing with a note open" names, resolved directly from the
 * URL (a direct link to `/v/:vaultId/n/:slug`) or, at the root path, from
 * the stored last note App.tsx's own redirect effect will act on. Reading
 * the same stored source directly — rather than waiting for that redirect's
 * navigation, or for the note's own content fetch to populate `activeNote`
 * — avoids a mount-order race between this and App.tsx's effect.
 */
export function resolveLandingVaultId(
  locationPathname: string,
): VaultId | undefined {
  const direct = pathToNoteIdentity(locationPathname)?.vaultId;
  if (direct) {
    return direct;
  }
  if (locationPathname !== "/") {
    return undefined;
  }
  return getStoredLastNote()?.vaultId;
}

export function getStoredUnfoldedVault(): VaultId | null {
  return getStoredString(LAST_UNFOLDED_VAULT_KEY);
}

export function setStoredUnfoldedVault(vaultId: VaultId | null): void {
  try {
    if (vaultId) {
      window.localStorage.setItem(LAST_UNFOLDED_VAULT_KEY, vaultId);
    } else {
      window.localStorage.removeItem(LAST_UNFOLDED_VAULT_KEY);
    }
  } catch {
    // Ignore storage failures (private mode, disabled storage).
  }
}

// Vault id and folder path never collide with this separator: folder paths
// are built from `/`-joined names (see Explorer.tsx's FolderNode), and Vault
// ids are UUIDs, so U+0001 never appears in either half. Written as an escape
// rather than a raw embedded byte so it survives formatting/copy-paste
// legibly instead of vanishing into an invisible control character.
const FOLDER_KEY_SEP = "\u0001";

/**
 * The accordion's per-Vault folder-open memory (#142): the same flat
 * `expandedFolders` record App.tsx already persists, namespaced by Vault id
 * so two Vaults' identically-named folders (e.g. both have a `Journal`)
 * don't share open/closed state. Scoped to the accordion only — narrowed-
 * scope browsing keeps today's unnamespaced keys untouched.
 */
export function expandedFoldersForVault(
  expandedFolders: Record<string, boolean>,
  vaultId: VaultId,
): Record<string, boolean> {
  const prefix = `${vaultId}${FOLDER_KEY_SEP}`;
  const result: Record<string, boolean> = {};
  for (const [key, value] of Object.entries(expandedFolders)) {
    if (key.startsWith(prefix)) {
      result[key.slice(prefix.length)] = value;
    }
  }
  return result;
}

/** Merges a Vault's next folder-open state back into the shared flat record,
 * leaving every other Vault's (and narrowed-scope's unnamespaced) entries
 * untouched. */
export function withVaultFolderChange(
  expandedFolders: Record<string, boolean>,
  vaultId: VaultId,
  nextForVault: Record<string, boolean>,
): Record<string, boolean> {
  const prefix = `${vaultId}${FOLDER_KEY_SEP}`;
  const rest: Record<string, boolean> = {};
  for (const [key, value] of Object.entries(expandedFolders)) {
    if (!key.startsWith(prefix)) {
      rest[key] = value;
    }
  }
  for (const [path, value] of Object.entries(nextForVault)) {
    rest[`${prefix}${path}`] = value;
  }
  return rest;
}
