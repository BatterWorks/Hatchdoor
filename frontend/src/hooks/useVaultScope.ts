import { useCallback, useEffect, useState } from "react";

import { apiFetch } from "../api/api";
import { readErrorMessage } from "../api/apiError";
import { getStoredScope, setStoredScope } from "../lib/storage";
import type {
  VaultDiscoveryResponse,
  VaultId,
  VaultScope,
  VaultSummary,
} from "../types";

/**
 * The selected Vault scope: state and storage only (#137) — no Scope zone or
 * other chrome reads or writes this yet. Persists per browser across
 * navigation and reloads via `lib/storage`'s `getStoredScope`/`setStoredScope`.
 * `setScope` exists for the Scope zone (#114) to call; nothing in this ticket
 * calls it yet, since narrowing scope is a deliberate act taken in chrome that
 * does not exist here.
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
 */
export function useVaultDiscovery() {
  const [vaults, setVaults] = useState<VaultSummary[]>([]);
  const [demoMode, setDemoMode] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

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

  return { vaults, demoMode, loading, error, loadVaults };
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
