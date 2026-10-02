import { useCallback, useEffect, useRef, useState } from "react";

import { getWriteCapabilities, isDemoReadOnlyError } from "../api/writeApi";
import type { VaultId } from "../types";

/**
 * Write-capability state for one Vault: whether the server accepts writes,
 * any posture warnings to surface, and the transient notice shown after a
 * write. The setters are exposed because note-action handlers and NotePage
 * also drive the notice/warnings. `vaultId` is the note currently open where
 * one is, else the primary Vault (`resolvePrimaryVaultId`) — settings-page
 * visibility no longer comes from here (`write-capabilities` dropped
 * `settings_enabled` in #101); the shell derives it from Vault discovery's
 * `demo_mode` instead.
 *
 * Write mode is re-derived, not read once per Vault (#339). A backend that
 * restarts into demo mode is noticed by the collection (its revision stream
 * reconnects and discovery answers `demo_mode: true`), so:
 *
 * - `demoMode` true resolves `writeEnabled` to `false` in the same render,
 *   without waiting on a request: a demo instance never exposes a write
 *   affordance.
 * - `revision` (the collection revision) re-asks `write-capabilities` on every
 *   change, so a posture change the collection announces reaches every open
 *   tab without a reload.
 * - `recheck` re-asks on demand; the shell calls it when a write comes back
 *   `demo_read_only`, the one answer that proves the posture moved under it.
 *
 * On a demo instance the request itself also fails closed: the server wraps
 * `GET .../write-capabilities` in the same `demo_guard` every mutation route
 * carries, so it 403s with `demo_read_only`. A first read for a Vault fails
 * closed on any error. A re-read that fails for any other reason (a dropped
 * connection mid-edit) keeps the answer already held rather than tearing an
 * open editor down over a transient fault; a refusal still switches it off.
 */
export function useWriteMode(
  vaultId: VaultId | undefined,
  {
    demoMode = false,
    revision = null,
  }: { demoMode?: boolean; revision?: number | null } = {},
) {
  const [serverWriteEnabled, setServerWriteEnabled] = useState(false);
  const [writeWarnings, setWriteWarnings] = useState<string[]>([]);
  const [writeNotice, setWriteNotice] = useState<string | null>(null);
  const [recheckId, setRecheckId] = useState(0);
  // The Vault the held answer belongs to: a read for a different Vault is a
  // first read and fails closed, a read for the same one is a re-read.
  const answeredVaultRef = useRef<VaultId | undefined>(undefined);

  const recheck = useCallback(() => setRecheckId((id) => id + 1), []);

  useEffect(() => {
    if (!vaultId || demoMode) {
      answeredVaultRef.current = undefined;
      setServerWriteEnabled(false);
      setWriteWarnings([]);
      return;
    }

    let cancelled = false;
    const rereading = answeredVaultRef.current === vaultId;

    void (async () => {
      try {
        const capabilities = await getWriteCapabilities(vaultId);
        if (!cancelled) {
          answeredVaultRef.current = vaultId;
          setServerWriteEnabled(Boolean(capabilities.enabled));
          setWriteWarnings(
            Array.isArray(capabilities.warnings) ? capabilities.warnings : [],
          );
        }
      } catch (error) {
        if (cancelled || (rereading && !isDemoReadOnlyError(error))) {
          return;
        }
        answeredVaultRef.current = undefined;
        setServerWriteEnabled(false);
        setWriteWarnings([]);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [vaultId, demoMode, revision, recheckId]);

  return {
    writeEnabled: serverWriteEnabled && !demoMode,
    writeWarnings,
    setWriteWarnings,
    writeNotice,
    setWriteNotice,
    recheck,
  };
}
