/**
 * Shared multi-Vault test fixtures (#137). One, three, and eight Vaults, plus
 * a builder for each non-healthy per-Vault condition the `/api/v1/vaults/**`
 * API can report — the count-or-condition slot vocabulary the design spec
 * (#116, amended by #117) settles: healthy, indexing, stale, sync failed,
 * sync stopped, conflict, and unavailable. #137 builds no chrome that renders
 * these; this module exists so #114-#126's own tests have realistic,
 * consistently-shaped wire data to assert against without each re-deriving
 * it. `git_error`/`search_error`/`activation_error` codes below are
 * plausible placeholders for conditions not yet exercised by any shipped
 * route (Git sync/conflict reporting is #121's own ticket) — confirm the
 * exact backend `code` string against `src/git/**` when a ticket first
 * renders that specific condition; the shape (`VaultRuntimeError`) is what
 * matters here, not the string literal.
 */
import type {
  VaultCapabilities,
  VaultDiscoveryResponse,
  VaultId,
  VaultParticipant,
  VaultParticipantState,
  VaultReadProjection,
  VaultSummary,
} from "../../types";

let nextIdSuffix = 1;

/** A deterministic, canonical-looking Vault ID, unique per call within a test
 * run. Deterministic (not random) so assertions can be written against a
 * fixture's own returned ID without an extra round-trip. */
function nextVaultId(): VaultId {
  const suffix = String(nextIdSuffix).padStart(12, "0");
  nextIdSuffix += 1;
  return `00000000-0000-4000-8000-${suffix}`;
}

/** Reset the deterministic ID counter. Call between tests that assert on
 * exact generated IDs, if isolation from other tests' fixture calls matters. */
export function resetVaultIdSequence(): void {
  nextIdSuffix = 1;
}

const FULL_CAPABILITIES: VaultCapabilities = {
  browse: true,
  search: true,
  mutate: true,
  pull: true,
  push: true,
  retry: true,
};

export function healthyVault(
  name: string,
  overrides: Partial<VaultSummary> = {},
): VaultSummary {
  return {
    vault_id: nextVaultId(),
    name,
    enabled: true,
    exclude_patterns: [],
    credential_configured: false,
    activation: "active",
    local_content: "read_write",
    search: "ready",
    git: "disabled",
    watcher: "running",
    capabilities: FULL_CAPABILITIES,
    ...overrides,
  };
}

/** `63%`, shimmering: a first index (or a reindex) in flight. Not a fault —
 * search still answers from the previous snapshot where one exists. */
export function indexingVault(name = "Indexing Vault"): VaultSummary {
  return healthyVault(name, { search: "indexing" });
}

/** `stale`: indexing failed: the snapshot is frozen behind the Markdown. */
export function staleVault(name = "Stale Vault"): VaultSummary {
  return healthyVault(name, {
    search: "stale",
    search_error: {
      code: "vault_read_unavailable",
      message: "The last index build for this Vault failed.",
      retryable: true,
    },
  });
}

/** `sync failed`: the last Git sync did not succeed; backoff retry running. */
export function syncFailedVault(name = "Sync Failed Vault"): VaultSummary {
  return healthyVault(name, {
    git: "unavailable",
    git_error: {
      code: "git_sync_failed",
      message: "The last Git sync for this Vault did not succeed.",
      retryable: true,
    },
  });
}

/** `sync stopped`: `dirty_working_copy` — local edits halted integration. */
export function syncStoppedVault(name = "Sync Stopped Vault"): VaultSummary {
  return healthyVault(name, {
    git: "unavailable",
    git_error: {
      code: "dirty_working_copy",
      message: "Local edits in this Vault halted Git integration.",
      retryable: false,
    },
  });
}

/** `conflict`: a content conflict; every local commit retained, remote
 * untouched. */
export function conflictVault(name = "Conflict Vault"): VaultSummary {
  return healthyVault(name, {
    git: "unavailable",
    git_error: {
      code: "git_content_conflict",
      message: "A content conflict is blocking Git integration.",
      retryable: false,
    },
  });
}

/** `unavailable`: should be answering and is not (e.g. a vanished directory,
 * or a Vault that never published a first snapshot). */
export function unavailableVault(name = "Unavailable Vault"): VaultSummary {
  return healthyVault(name, {
    activation: "unavailable",
    local_content: "unavailable",
    search: "unavailable",
    activation_error: {
      code: "vault_unavailable",
      message: "This Vault has no active runtime.",
      retryable: true,
    },
  });
}

/** Read-only: a Pull-only Git Vault. The slot carries its note count like any
 * healthy Vault — read-only-ness surfaces as the absence of Edit/New note,
 * never as a condition word. */
export function readOnlyVault(name = "Read Only Vault"): VaultSummary {
  return healthyVault(name, {
    local_content: "read_only",
    capabilities: { ...FULL_CAPABILITIES, mutate: false, push: false },
  });
}

/** A paused (disabled) Vault: absent from the sidebar and from `"all"`,
 * reachable only in Vault management (#120). */
export function pausedVault(name = "Paused Vault"): VaultSummary {
  return healthyVault(name, { enabled: false, activation: "disabled" });
}

export function discoveryResponse(
  vaults: VaultSummary[],
  demoMode = false,
): VaultDiscoveryResponse {
  return {
    registry_revision: vaults.length,
    collection_revision: vaults.length,
    vaults,
    demo_mode: demoMode,
  };
}

/** The sizing case named in #137's acceptance criteria. */
export const ONE_VAULT: VaultSummary[] = [healthyVault("Solo Vault")];

/** The common case named in #137's acceptance criteria. */
export const THREE_VAULTS: VaultSummary[] = [
  healthyVault("Alpha"),
  indexingVault("Beta"),
  staleVault("Gamma"),
];

/** The sizing case named in #137's acceptance criteria: every non-healthy
 * condition represented once. */
export const EIGHT_VAULTS: VaultSummary[] = [
  healthyVault("Vault One"),
  healthyVault("Vault Two"),
  indexingVault("Vault Three"),
  staleVault("Vault Four"),
  syncFailedVault("Vault Five"),
  syncStoppedVault("Vault Six"),
  conflictVault("Vault Seven"),
  unavailableVault("Vault Eight"),
];

/** A collection-read participant entry for one Vault, at the given
 * freshness. `state: "unavailable"` carries a `VaultReadError`, matching the
 * envelope `docs/migrations/vault-scoped-clients.md` documents. */
export function participantFor(
  vault: VaultSummary,
  state: VaultParticipantState = "fresh",
): VaultParticipant {
  const participant: VaultParticipant = {
    vault_id: vault.vault_id,
    vault_name: vault.name,
    state,
  };
  if (state === "unavailable") {
    participant.error = {
      code: "vault_unavailable",
      message: `${vault.name} did not answer.`,
      vault_id: vault.vault_id,
      retryable: true,
    };
  }
  return participant;
}

/** Wraps `data` in the shared one-or-all envelope every `{scope}` route uses. */
export function collectionEnvelope<T>(
  scope: string,
  data: T,
  participants: VaultParticipant[],
): VaultReadProjection<T> {
  return {
    scope,
    collection_revision: participants.length,
    partial: participants.some((entry) => entry.state !== "fresh"),
    participants,
    data,
  };
}
