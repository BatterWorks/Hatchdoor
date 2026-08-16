import { useCallback, useEffect, useState } from "react";

import { apiFetch } from "../api/api";
import { readErrorMessage } from "../api/apiError";
import { getStoredScope, setStoredScope } from "../lib/storage";
import type {
  LegacyMigrationRecovery,
  VaultDiscoveryResponse,
  VaultId,
  VaultReadProjection,
  VaultRegistryRecovery,
  VaultScope,
  VaultStatistics,
  VaultSummary,
} from "../types";

/**
 * The selected Vault scope: state and storage only. Persists per browser
 * across navigation and reloads via `lib/storage`'s
 * `getStoredScope`/`setStoredScope`. `setScope` is called by the sidebar
 * Scope zone (#138) on desktop and the mobile topbar's scope bottom sheet
 * (#145) below 920px — never both at once, since one replaces the other at
 * that breakpoint.
 */
export function useVaultScope(): [VaultScope, (next: VaultScope) => void] {
  const [scope, setScopeState] = useState<VaultScope>(() => getStoredScope());

  const setScope = useCallback((next: VaultScope) => {
    setScopeState(next);
    setStoredScope(next);
  }, []);

  return [scope, setScope];
}

/**
 * Enabled Vaults, in Vault-management order (the order `GET /api/v1/vaults`
 * returns), plus the instance's demo-mode posture. Disabled Vaults never
 * appear here and never participate in `"all"` (docs/migrations/vault-scoped-clients.md).
 *
 * `recovery` (the persisted registry file itself is unreadable) and
 * `legacyMigrationRecovery` (the registry loaded fine, empty, but a failed
 * safe legacy import still needs recovery) are mutually exclusive broken-start
 * conditions (#150): both leave `vaults` empty, but only one is ever set.
 * Neither is polled on an interval — a "Try again" action just calls
 * `loadVaults` again, same as any other refresh.
 */
export function useVaultDiscovery() {
  const [vaults, setVaults] = useState<VaultSummary[]>([]);
  const [demoMode, setDemoMode] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [recovery, setRecovery] = useState<VaultRegistryRecovery | null>(null);
  const [legacyMigrationRecovery, setLegacyMigrationRecovery] =
    useState<LegacyMigrationRecovery | null>(null);

  const loadVaults = useCallback(async () => {
    setError(null);
    try {
      const res = await apiFetch("/api/v1/vaults");
      if (!res.ok) {
        throw new Error(await readErrorMessage(res, "Failed loading Vaults"));
      }
      const json = (await res.json()) as VaultDiscoveryResponse;
      setVaults(json.vaults.filter((vault) => vault.enabled));
      setDemoMode(json.demo_mode);
      setRecovery(json.recovery ?? null);
      setLegacyMigrationRecovery(json.legacy_migration_recovery ?? null);
    } catch (err) {
      setError(
        err instanceof Error ? err.message : "Unknown Vault discovery error",
      );
    }
  }, []);

  useEffect(() => {
    void (async () => {
      setLoading(true);
      await loadVaults();
      setLoading(false);
    })();
  }, [loadVaults]);

  return {
    vaults,
    demoMode,
    loading,
    error,
    recovery,
    legacyMigrationRecovery,
    loadVaults,
  };
}

/**
 * The Vault a Vault-less action or a single-Vault page targets when there is
 * no chrome to ask: the open note's own Vault where one exists, else the
 * first enabled Vault in Vault-management order. Used both for a write with
 * no active note (creating one) and for pages that always show exactly one
 * Vault's data and have no note open to anchor on (Statistics). Superseded
 * once #114's Scope zone lets a person choose explicitly. Undefined only at
 * zero enabled Vaults.
 */
export function resolvePrimaryVaultId(
  activeNoteVaultId: VaultId | undefined,
  vaults: VaultSummary[],
): VaultId | undefined {
  return activeNoteVaultId ?? vaults[0]?.vault_id;
}

/**
 * Every enabled Vault's note count, for the sidebar Scope zone's healthy-slot
 * reading (#139). Always fetched at `"all"` scope regardless of the browsing
 * scope selected elsewhere — the Scope zone lists every enabled Vault
 * whatever is currently selected, so its counts can't depend on that
 * selection. `enabled` gates the fetch on whether the zone can even render
 * (more than one enabled Vault); `vaultRevision` (the same SSE-driven counter
 * `useVaultTree` already tracks) triggers a refetch on change rather than
 * opening a second subscription to the same event stream. A fetch failure
 * leaves prior counts in place; the slot falls back to treating a missing
 * entry as unknown.
 */
export function useVaultNoteCounts(
  enabled: boolean,
  vaultRevision: number,
): Record<VaultId, number> {
  const [counts, setCounts] = useState<Record<VaultId, number>>({});

  useEffect(() => {
    if (!enabled) {
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const res = await apiFetch("/api/v1/vaults/all/stats");
        if (!res.ok) {
          return;
        }
        const projection = (await res.json()) as VaultReadProjection<
          VaultStatistics[]
        >;
        if (cancelled) {
          return;
        }
        const next: Record<VaultId, number> = {};
        for (const entry of projection.data) {
          next[entry.vault_id] = entry.note_count;
        }
        setCounts(next);
      } catch {
        // Leave prior counts in place.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [enabled, vaultRevision]);

  return counts;
}
