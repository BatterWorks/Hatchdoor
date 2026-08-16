import { deriveVaultAggregate, deriveVaultSlot } from "./vaultSlotLogic";
import type { VaultId, VaultSummary } from "../types";

/** One Vault row's trailing slot. `demoMode` clamps a condition to the amber
 * tier with an instruction-free sentence (#152). */
export function VaultSlot({
  vault,
  noteCount,
  demoMode = false,
}: {
  vault: VaultSummary;
  noteCount: number | undefined;
  demoMode?: boolean;
}) {
  const state = deriveVaultSlot(vault, noteCount, demoMode);
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
  // The count leads, because it is the part the reader can act on. The
  // shimmer follows it at the same size as the indexing slot's, so a Vault
  // moving from indexing to browsable to ready reads as one bar resolving
  // rather than three unrelated states.
  if (state.kind === "count-pending-search") {
    return (
      <span
        className="vault-slot-pending-search"
        role="status"
        title={state.sentence}
        aria-label={`${state.count} notes. ${state.sentence}`}
      >
        <span className="side-count">{state.count}</span>
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
  demoMode = false,
}: {
  vaults: VaultSummary[];
  counts: Record<VaultId, number | undefined>;
  demoMode?: boolean;
}) {
  const aggregate = deriveVaultAggregate(vaults, counts, demoMode);
  if (aggregate.kind === "count") {
    return <span className="side-count">{aggregate.count}</span>;
  }
  return (
    <span className={`vault-slot-shortfall vault-tier-${aggregate.tier}`}>
      {aggregate.participating} of {aggregate.total}
    </span>
  );
}
