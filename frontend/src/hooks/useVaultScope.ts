import { useCallback, useState } from "react";

import { getStoredScope, setStoredScope } from "../lib/storage";
import type { VaultId, VaultScope, VaultSummary } from "../types";

/**
 * The selected Vault scope: state and storage only. Persists per browser
 * across navigation and reloads via `lib/storage`'s
 * `getStoredScope`/`setStoredScope`. `setScope` is called by the sidebar
 * Scope zone (#138) on desktop and the mobile topbar's scope bottom sheet
 * (#145) below 920px — never both at once, since one replaces the other at
 * that breakpoint.
 *
 * The Vault collection itself — the list, the counts, the demo-mode posture,
 * and the revision stream that invalidates them — belongs to the collection
 * client in `../vaults` (#198), not here.
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
