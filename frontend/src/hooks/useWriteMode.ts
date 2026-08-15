import { useEffect, useState } from "react";

import { getWriteCapabilities } from "../api/writeApi";
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
 * In demo mode `getWriteCapabilities` itself already fails closed: the
 * server wraps `GET .../write-capabilities` in the same `demo_guard` every
 * mutation route carries, so the request 403s with `demo_read_only` before
 * this hook's `enabled` field is ever read, and the catch below already
 * resolves `writeEnabled` to `false`. No demo-mode branch belongs here.
 */
export function useWriteMode(vaultId: VaultId | undefined) {
  const [writeEnabled, setWriteEnabled] = useState(false);
  const [writeWarnings, setWriteWarnings] = useState<string[]>([]);
  const [writeNotice, setWriteNotice] = useState<string | null>(null);

  useEffect(() => {
    if (!vaultId) {
      setWriteEnabled(false);
      setWriteWarnings([]);
      return;
    }

    let cancelled = false;

    void (async () => {
      try {
        const capabilities = await getWriteCapabilities(vaultId);
        if (!cancelled) {
          setWriteEnabled(Boolean(capabilities.enabled));
          setWriteWarnings(
            Array.isArray(capabilities.warnings) ? capabilities.warnings : [],
          );
        }
      } catch {
        if (!cancelled) {
          setWriteEnabled(false);
          setWriteWarnings([]);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [vaultId]);

  return {
    writeEnabled,
    writeWarnings,
    setWriteWarnings,
    writeNotice,
    setWriteNotice,
  };
}
