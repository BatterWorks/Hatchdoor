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

function joinWithAnd(names: string[]): string {
  if (names.length <= 1) {
    return names[0] ?? "";
  }
  if (names.length === 2) {
    return `${names[0]} and ${names[1]}`;
  }
  return `${names.slice(0, -1).join(", ")}, and ${names[names.length - 1]}`;
}
