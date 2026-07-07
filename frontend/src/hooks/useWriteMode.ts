import { useEffect, useState } from "react";

import { getWriteCapabilities } from "../api/writeApi";

/**
 * Write-capability state: whether the server accepts writes, any posture
 * warnings to surface, and the transient notice shown after a write. The
 * setters are exposed because note-action handlers and NotePage also drive the
 * notice/warnings.
 */
export function useWriteMode() {
  const [writeEnabled, setWriteEnabled] = useState(false);
  const [writeWarnings, setWriteWarnings] = useState<string[]>([]);
  const [writeNotice, setWriteNotice] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      try {
        const capabilities = await getWriteCapabilities();
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
  }, []);

  return {
    writeEnabled,
    writeWarnings,
    setWriteWarnings,
    writeNotice,
    setWriteNotice,
  };
}
