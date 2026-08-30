import { useMemo, useSyncExternalStore } from "react";

import {
  getVaultCollectionSnapshot,
  refreshVaultCollection,
  subscribeVaultCollection,
  type VaultCollectionState,
} from "./vaultCollectionStore";
import { createVaultProjection, type VaultProjection } from "./vaultProjection";

/**
 * The Vault collection, as every surface sees it: one list, one set of counts,
 * one demo-mode posture, one revision stream. Mounting this hook anywhere joins
 * the shared subscription rather than opening a second one, so no two views can
 * disagree about the same Vault.
 *
 * `refresh` re-reads the collection immediately. It exists for the caller that
 * has just mutated and does not want to wait for the server's revision event;
 * the event itself already refreshes every subscriber without anyone asking.
 */
export function useVaultCollection(): VaultCollectionState & {
  refresh: () => Promise<void>;
} {
  const snapshot = useSyncExternalStore(
    subscribeVaultCollection,
    getVaultCollectionSnapshot,
  );
  return useMemo(
    () => ({ ...snapshot, refresh: refreshVaultCollection }),
    [snapshot],
  );
}

/**
 * The collection's demo-aware slot projection, bound to the live snapshot.
 * Keyed on the three fields the projection actually reads rather than the whole
 * snapshot, so a bare revision bump does not hand every consumer a new
 * projection to re-run its effects on.
 */
export function useVaultProjection(): VaultProjection {
  const snapshot = useSyncExternalStore(
    subscribeVaultCollection,
    getVaultCollectionSnapshot,
  );
  const { vaults, noteCounts, demoMode } = snapshot;
  return useMemo(
    () => createVaultProjection({ vaults, noteCounts, demoMode }),
    [vaults, noteCounts, demoMode],
  );
}
