import { deriveVaultAggregate, deriveVaultSlot } from "./vaultSlotLogic";
import type { VaultId, VaultSummary } from "../types";

/** One Vault row's trailing slot. */
export function VaultSlot({
  vault,
  noteCount,
}: {
  vault: VaultSummary;
  noteCount: number | undefined;
}) {
  const state = deriveVaultSlot(vault, noteCount);
  if (state.kind === "count") {
    return <span className="side-count">{state.count}</span>;
  }
  if (state.kind === "indexing") {
    return (
      <span className="vault-slot-indexing" role="status" aria-label="Indexing">
        <span className="vault-slot-indexing-bar" aria-hidden="true" />
      </span>
    );
  }
  return (
    <span
      className={`vault-slot-condition vault-tier-${state.tier}`}
      title={state.sentence}
      aria-label={state.sentence}
    >
      {state.word}
    </span>
  );
}

/** The `All Vaults` row's and the collapsed head's shared aggregate slot. */
export function VaultAggregateSlot({
  vaults,
  counts,
}: {
  vaults: VaultSummary[];
  counts: Record<VaultId, number | undefined>;
}) {
  const aggregate = deriveVaultAggregate(vaults, counts);
  if (aggregate.kind === "count") {
    return <span className="side-count">{aggregate.count}</span>;
  }
  return (
    <span className={`vault-slot-shortfall vault-tier-${aggregate.tier}`}>
      {aggregate.participating} of {aggregate.total}
    </span>
  );
}
