import type { VaultParticipant } from "../types";

/**
 * The Vaults a collection read asked but did not get a fresh answer from —
 * `stale` or `unavailable` participant state, in the order the envelope
 * listed them. Used to tell the truth about a `partial` read without a
 * banner (#141): the trailing line and the error-block replacement both name
 * only these.
 */
export function missingVaultNames(participants: VaultParticipant[]): string[] {
  return participants
    .filter((participant) => participant.state !== "fresh")
    .map((participant) => participant.vault_name);
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

function joinWithAnd(names: string[]): string {
  if (names.length <= 1) {
    return names[0] ?? "";
  }
  if (names.length === 2) {
    return `${names[0]} and ${names[1]}`;
  }
  return `${names.slice(0, -1).join(", ")}, and ${names[names.length - 1]}`;
}
