import type { VaultId, VaultScope, VaultSummary } from "../types";

export type VaultSlotState =
  | { kind: "count"; count: number }
  | { kind: "indexing" }
  | { kind: "condition"; word: string; tier: "warn" | "error"; sentence: string };

export type VaultAggregate =
  | { kind: "count"; count: number }
  | {
      kind: "shortfall";
      participating: number;
      total: number;
      tier: "warn" | "error";
    };

/** The browsing scope's display name: `All Vaults`, or the narrowed Vault's
 * own name — shared by the desktop Scope zone (#138) and the mobile scope
 * row (#145), the two chrome surfaces that name the current scope. */
export function scopeName(scope: VaultScope, vaults: VaultSummary[]): string {
  if (scope === "all") {
    return "All Vaults";
  }
  return vaults.find((vault) => vault.vault_id === scope)?.name ?? "All Vaults";
}

/**
 * Each Vault's single trailing slot: its note count when healthy, a
 * shimmering placeholder while indexing, or one condition word otherwise —
 * never a count and a condition together (#116, amended by #117).
 *
 * Priority (worst first): `unavailable` outranks every Git condition, which
 * outranks `stale`, which outranks indexing. A Vault that has never
 * published a snapshot reports the same `search: "unavailable"` the API uses
 * for a vanished directory, but only the latter also turns `activation`
 * `"unavailable"` (`activation_snapshot` in `src/vault_runtime.rs`: a Vault
 * whose directory stats fine is `"active"` the moment it is enabled,
 * regardless of whether it has indexed yet). Checking `activation` first is
 * what tells "first index" apart from "lost it" without reopening the
 * frozen contract, exactly as #116's resolution describes.
 */
export function deriveVaultSlot(
  vault: VaultSummary,
  noteCount: number | undefined,
): VaultSlotState {
  if (vault.activation === "unavailable") {
    return {
      kind: "condition",
      word: "unavailable",
      tier: "error",
      sentence:
        vault.activation_error?.message ??
        "This Vault should be answering and is not.",
    };
  }
  if (vault.git === "unavailable") {
    if (vault.git_error?.code === "git_content_conflict") {
      return {
        kind: "condition",
        word: "conflict",
        tier: "error",
        sentence:
          vault.git_error.message ??
          "A content conflict is blocking Git sync for this Vault.",
      };
    }
    if (vault.git_error?.code === "dirty_working_copy") {
      return {
        kind: "condition",
        word: "sync stopped",
        tier: "error",
        sentence:
          vault.git_error.message ??
          "Local edits in this Vault halted Git sync.",
      };
    }
    return {
      kind: "condition",
      word: "sync failed",
      tier: "warn",
      sentence:
        vault.git_error?.message ??
        "The last Git sync for this Vault did not succeed.",
    };
  }
  if (vault.search === "stale") {
    return {
      kind: "condition",
      word: "stale",
      tier: "warn",
      sentence:
        vault.search_error?.message ??
        "The last index build for this Vault failed.",
    };
  }
  if (vault.search === "indexing" || vault.search === "unavailable") {
    return { kind: "indexing" };
  }
  return { kind: "count", count: noteCount ?? 0 };
}

/**
 * The `All Vaults` row's and the collapsed Scope head's shared aggregate:
 * the enabled-Vault count when every Vault is healthy or indexing (both
 * "participating" — an indexing Vault keeps answering from its previous
 * snapshot), or the shortfall against the worst tier present when some are
 * not (#116, amended by #117).
 */
export function deriveVaultAggregate(
  vaults: VaultSummary[],
  counts: Record<VaultId, number | undefined>,
): VaultAggregate {
  let worstTier: "warn" | "error" | null = null;
  let participating = 0;
  for (const vault of vaults) {
    const slot = deriveVaultSlot(vault, counts[vault.vault_id]);
    if (slot.kind === "condition") {
      worstTier = slot.tier === "error" ? "error" : (worstTier ?? "warn");
    } else {
      participating += 1;
    }
  }
  if (worstTier === null) {
    return { kind: "count", count: vaults.length };
  }
  return {
    kind: "shortfall",
    participating,
    total: vaults.length,
    tier: worstTier,
  };
}
