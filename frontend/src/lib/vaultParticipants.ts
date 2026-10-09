import type { VaultParticipant } from "../types";

/**
 * The Vaults a collection read asked and got no answer from — `unavailable`
 * participant state, in the order the envelope listed them. Used to tell the
 * truth about a `partial` read without a banner (#141): the trailing line and
 * the error-block replacement both name only these. A `stale` participant
 * answered, from the index generation before the one being built, and its
 * rows are on screen (#530): saying it "did not answer" under its own tree
 * was the lie this sentence exists to avoid.
 */
export function missingVaultNames(participants: VaultParticipant[]): string[] {
  return participants
    .filter((participant) => participant.state === "unavailable")
    .map((participant) => participant.vault_name);
}

/**
 * The Vaults where a *semantic* search could not reach some or all of the
 * notes it selected, because those notes have no vectors: not embedded yet,
 * or on a demoted layer with layer embedding switched off (#328). Kept apart
 * from `missingVaultNames` on purpose: saying one "did not answer" is exactly
 * the confusion the `not_searchable` state exists to remove — its Notes are
 * present and browsable, and Keyword search reaches them.
 */
export function notSearchableVaultNames(
  participants: VaultParticipant[],
): string[] {
  return participants
    .filter((participant) => participant.state === "not_searchable")
    .map((participant) => participant.vault_name);
}

/** "Semantic search could not reach some notes in X. Keyword search can." —
 * true whether the notes are still being embedded or will never be. */
export function describeNotSearchableVaults(names: string[]): string {
  return `Semantic search could not reach some notes in ${joinWithAnd(names)}. Keyword search can.`;
}

/** "X did not answer." / "X and Y did not answer." / "X, Y, and Z did not
 * answer." Never a banner — just the sentence a trailing line or an
 * error-block description carries. */
export function describeMissingVaults(missing: string[]): string {
  return `${joinWithAnd(missing)} did not answer.`;
}

/** "X could not be drawn." / "X and Y could not be drawn." — the all-Vault
 * graph's own wording for a Vault that contributes no island (#118's
 * resolution, implemented by #143). Distinct from `describeMissingVaults`:
 * a Vault answering from a stale snapshot still draws an island (its
 * caption carries the condition word instead), so the graph's "did not
 * draw" set is narrower than "not fresh" and is computed by the caller from
 * which Vaults are absent from the response data, not from participant
 * state. */
export function describeVaultsNotDrawn(missing: string[]): string {
  return `${joinWithAnd(missing)} could not be drawn.`;
}

/** "X" / "X and Y" / "X, Y, and Z": the Vault-name list every sentence here
 * is built on, exported so a feature can phrase its own sentence around the
 * same list. */
export function joinWithAnd(names: string[]): string {
  if (names.length <= 1) {
    return names[0] ?? "";
  }
  if (names.length === 2) {
    return `${names[0]} and ${names[1]}`;
  }
  return `${names.slice(0, -1).join(", ")}, and ${names[names.length - 1]}`;
}
