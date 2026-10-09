import { deriveVaultAggregate, deriveVaultSlot } from "./vaultSlotLogic";
import type { VaultId, VaultSummary } from "../types";

const UNKNOWN_COUNT_MARK = "–";
const UNKNOWN_COUNT_LABEL = "Note count not known";

/** A count the stats read has not supplied (#333): a neutral dash in the
 * count's own ink, so it is never mistaken for an empty Vault's 0. `inline`
 * drops the slot's own type so the dash can sit inside running text (the
 * Settings Vault detail's blurb) and take that text's ink instead. */
export function UnknownCount({ inline = false }: { inline?: boolean }) {
  return (
    <span
      className={inline ? undefined : "side-count"}
      title={UNKNOWN_COUNT_LABEL}
      aria-label={UNKNOWN_COUNT_LABEL}
    >
      {UNKNOWN_COUNT_MARK}
    </span>
  );
}

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
    if (state.count === null) {
      return <UnknownCount />;
    }
    return <span className="side-count">{state.count}</span>;
  }
  if (state.kind === "indexing") {
    return (
      <span className="vault-slot-indexing" role="status" aria-label="Indexing">
        <span className="vault-slot-indexing-bar" aria-hidden="true" />
      </span>
    );
  }
  // Still, and in muted ink: the Vault is queued, not failing, and nothing
  // is moving until its turn comes round (ADR-35).
  if (state.kind === "waiting") {
    return (
      <span
        className="vault-slot-waiting"
        role="status"
        title={state.sentence}
        aria-label={state.sentence}
      >
        waiting
      </span>
    );
  }
  // The count is the whole slot, and the shimmer runs through it rather than
  // beside it: a Vault moving from indexing to browsable to ready reads as
  // one thing settling — a bar, then a moving number, then a still one — not
  // as a count that has grown a second marker next to it.
  if (state.kind === "count-pending-search") {
    return (
      <span
        className="vault-slot-pending-search"
        role="status"
        title={state.sentence}
        aria-label={
          state.count === null
            ? `${UNKNOWN_COUNT_LABEL}. ${state.sentence}`
            : `${state.count} notes. ${state.sentence}`
        }
      >
        <span className="side-count slot-shimmer-reading">
          {state.count ?? UNKNOWN_COUNT_MARK}
        </span>
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
  compact = false,
}: {
  vaults: VaultSummary[];
  counts: Record<VaultId, number | undefined>;
  demoMode?: boolean;
  /** The folded Scope head's form (#530): `5 of 8` beside the scope name,
   * which the head has no room to follow with a word; the sentence rides on
   * the title. Every other place says what the count counts. */
  compact?: boolean;
}) {
  const aggregate = deriveVaultAggregate(vaults, counts, demoMode);
  if (aggregate.kind === "count") {
    return <span className="side-count">{aggregate.count}</span>;
  }
  const reading = `${aggregate.participating} of ${aggregate.total}`;
  return (
    <span
      className={`vault-slot-shortfall vault-tier-${aggregate.tier}`}
      title={compact ? `${reading} answering` : undefined}
    >
      {reading}
      {/* A plain span's aria-label is not reliably read, so the word stays
          in the text and is only hidden from sight. */}
      <span className={compact ? "visually-hidden" : undefined}>
        {" "}
        answering
      </span>
    </span>
  );
}
