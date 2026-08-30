import {
  deriveVaultSlot,
  describeScopeSlot,
  type VaultSlotState,
} from "../app/vaultSlotLogic";
import type { VaultId, VaultScope, VaultSummary } from "../types";

/** The three fields the projection reads. A whole `VaultCollectionState`
 * satisfies it, and naming them keeps a consumer from re-deriving the
 * projection every time some unrelated field (the revision, say) moves. */
export type VaultProjectionInputs = {
  vaults: VaultSummary[];
  noteCounts: Record<VaultId, number>;
  demoMode: boolean;
};

/**
 * The collection's demo-aware projection: `app/vaultSlotLogic.ts`'s pure
 * vocabulary with the two inputs every call site used to decide for itself —
 * which count source applies, and whether demo mode applies — already bound to
 * the collection snapshot.
 *
 * A surface with its own count for a Vault (the graph's island node count)
 * passes it as `countOverride`; everything else gets the collection's counts.
 */
export type VaultProjection = {
  slotFor(vault: VaultSummary, countOverride?: number): VaultSlotState;
  describeScope(scope: VaultScope): string | null;
};

export function createVaultProjection(
  snapshot: VaultProjectionInputs,
): VaultProjection {
  return {
    slotFor: (vault, countOverride) =>
      deriveVaultSlot(
        vault,
        countOverride ?? snapshot.noteCounts[vault.vault_id],
        snapshot.demoMode,
      ),
    describeScope: (scope) =>
      describeScopeSlot(
        scope,
        snapshot.vaults,
        snapshot.noteCounts,
        snapshot.demoMode,
      ),
  };
}
