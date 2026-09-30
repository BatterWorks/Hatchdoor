import { useCallback, useEffect, useState } from "react";

import { getStoredScope, setStoredScope } from "../lib/storage";
import type { VaultId, VaultScope, VaultSummary } from "../types";
import { useVaultCollection, type VaultCollectionState } from "../vaults";

/**
 * Why a stored scope was dropped back to `"all"`: one sentence for the shared
 * notice strip. `id` changes on every fallback so a reader can show a second
 * notice that happens to read the same as the first.
 */
export type ScopeFallbackNotice = { id: number; message: string };

let nextFallbackId = 1;

/**
 * The selected Vault scope, reconciled against the live collection. Persists
 * per browser across navigation and reloads via `lib/storage`'s
 * `getStoredScope`/`setStoredScope`. `setScope` is called by the sidebar
 * Scope zone (#138) on desktop and the mobile topbar's scope bottom sheet
 * (#145) below 920px — never both at once, since one replaces the other at
 * that breakpoint.
 *
 * A stored scope naming a Vault that has left the browsing list (disconnected,
 * paused, or enabled but unavailable) is never handed on: every collection
 * read under it would be refused, and at one enabled Vault there is no scope
 * chrome left to pick another (#335). Once the collection has answered, such
 * a scope reads as `"all"` in the same render, is written back as `"all"`, and
 * the third element carries the sentence saying why. Every mounted instance
 * does this for itself, so a page holding its own copy (Graph, Statistics)
 * never keeps reading the dead id either. The collection revision stream
 * re-renders every subscriber, so a Vault paused from Settings, another tab or
 * an MCP agent is caught on the revision that reports it.
 *
 * Nothing is judged while discovery is in flight or has failed, while the
 * registry needs recovery, or at zero enabled Vaults: an empty list in any of
 * those says nothing about whether the stored Vault still exists.
 *
 * The Vault collection itself — the list, the counts, the demo-mode posture,
 * and the revision stream that invalidates them — belongs to the collection
 * client in `../vaults` (#198), not here.
 */
export function useVaultScope(): [
  VaultScope,
  (next: VaultScope) => void,
  ScopeFallbackNotice | null,
] {
  const [storedScope, setScopeState] = useState<VaultScope>(() =>
    getStoredScope(),
  );
  const [notice, setNotice] = useState<ScopeFallbackNotice | null>(null);
  const collection = useVaultCollection();
  const fallback = scopeFallbackMessage(storedScope, collection);

  const setScope = useCallback((next: VaultScope) => {
    setScopeState(next);
    setStoredScope(next);
  }, []);

  useEffect(() => {
    if (fallback === null) {
      return;
    }
    setScope("all");
    setNotice({ id: nextFallbackId++, message: fallback });
  }, [fallback, setScope]);

  return [fallback === null ? storedScope : "all", setScope, notice];
}

/**
 * The sentence explaining why `scope` cannot be browsed any more, or `null`
 * when it can (or when the collection cannot yet say). Exported for tests.
 */
export function scopeFallbackMessage(
  scope: VaultScope,
  collection: Pick<
    VaultCollectionState,
    | "readState"
    | "vaults"
    | "allVaults"
    | "recovery"
    | "legacyMigrationRecovery"
  >,
): string | null {
  if (
    scope === "all" ||
    collection.readState === "loading" ||
    collection.readState === "error" ||
    collection.recovery ||
    collection.legacyMigrationRecovery ||
    collection.vaults.length === 0
  ) {
    return null;
  }
  const browsing = collection.vaults.find((vault) => vault.vault_id === scope);
  if (browsing && browsing.activation !== "unavailable") {
    return null;
  }
  const known =
    browsing ?? collection.allVaults.find((vault) => vault.vault_id === scope);
  const fallbackTo = "so the explorer now shows All Vaults.";
  if (!known) {
    return `The Vault you were browsing has been disconnected, ${fallbackTo}`;
  }
  if (known.activation === "unavailable") {
    return `${known.name} is unavailable, ${fallbackTo}`;
  }
  return `${known.name} is paused, ${fallbackTo}`;
}

/**
 * The Vault a Vault-less action or a single-Vault page targets when there is
 * no chrome to ask: the open note's own Vault where one exists, else the
 * first enabled Vault in Vault-management order. Used both for a write with
 * no active note (creating one) and for pages that always show exactly one
 * Vault's data and have no note open to anchor on (Statistics). Superseded
 * once #114's Scope zone lets a person choose explicitly. Undefined only at
 * zero enabled Vaults.
 */
export function resolvePrimaryVaultId(
  activeNoteVaultId: VaultId | undefined,
  vaults: VaultSummary[],
): VaultId | undefined {
  return activeNoteVaultId ?? vaults[0]?.vault_id;
}
