import { useCallback, useEffect, useRef, useState } from "react";

/** How long typing inside one unit may pause before the document is flushed. */
const IDLE_FLUSH_MS = 2000;

export type AutosaveStatus = "idle" | "saving" | "saved" | "conflict" | "error";

export type SaveResult = { content_hash?: string | null };

/**
 * Writes the whole document against the last hash the server confirmed.
 *
 * Every write is a coherent document rather than a patch, so a long paragraph
 * is never left half-saved. Writes fire on unit commit and after an idle pause
 * while typing inside one unit; a conflict stops autosaving rather than
 * retrying into a losing race.
 */
export function useNoteAutosave({
  baseHash,
  enabled,
  save,
  onSaved,
}: {
  baseHash: string;
  enabled: boolean;
  save: (content: string, expectedHash: string) => Promise<SaveResult>;
  onSaved?: (result: SaveResult) => void;
}) {
  const [status, setStatus] = useState<AutosaveStatus>("idle");
  const [savedAt, setSavedAt] = useState<Date | null>(null);

  const hashRef = useRef(baseHash);
  // Each write bumps the vault revision twice, and a bump from write N can land
  // after write N+1 is confirmed, so recognising only the newest hash reports
  // divergence that never happened.
  const ownHashesRef = useRef<Set<string>>(new Set([baseHash]));
  const timerRef = useRef<number | null>(null);
  const pendingRef = useRef<string | null>(null);
  const inFlightRef = useRef(false);
  const queuedRef = useRef<string | null>(null);
  const stoppedRef = useRef(false);

  useEffect(() => {
    hashRef.current = baseHash;
    ownHashesRef.current.add(baseHash);
  }, [baseHash]);

  const clearTimer = () => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  };

  const write = useCallback(
    async (next: string) => {
      if (!enabled || stoppedRef.current) {
        return;
      }
      // A write takes about a second against a real vault and every content
      // change commits, so edits arriving mid-flight are routine. Queue the
      // newest one rather than dropping it: dropping loses the edit outright
      // and still reports "Saved".
      if (inFlightRef.current) {
        queuedRef.current = next;
        return;
      }

      inFlightRef.current = true;
      try {
        let current: string | null = next;
        while (current !== null && !stoppedRef.current) {
          setStatus("saving");
          try {
            const result = await save(current, hashRef.current);
            if (result?.content_hash) {
              hashRef.current = result.content_hash;
              ownHashesRef.current.add(result.content_hash);
            }
            onSaved?.(result);
          } catch (error) {
            stoppedRef.current = true;
            queuedRef.current = null;
            setStatus(
              error instanceof Error && error.name === "ConflictError"
                ? "conflict"
                : "error",
            );
            return;
          }

          current = queuedRef.current;
          queuedRef.current = null;
          if (current === null) {
            setStatus("saved");
            setSavedAt(new Date());
          }
        }
      } finally {
        inFlightRef.current = false;
      }
    },
    [enabled, save, onSaved],
  );

  /** A unit was committed: write now. */
  const commit = useCallback(
    (next: string) => {
      clearTimer();
      pendingRef.current = null;
      void write(next);
    },
    [write],
  );

  /** Still typing inside one unit: flush after the idle pause. */
  const touch = useCallback(
    (next: string) => {
      pendingRef.current = next;
      clearTimer();
      timerRef.current = window.setTimeout(() => {
        timerRef.current = null;
        const value = pendingRef.current;
        pendingRef.current = null;
        if (value !== null) {
          void write(value);
        }
      }, IDLE_FLUSH_MS);
    },
    [write],
  );

  const isOwnWrite = useCallback(
    (hash: string) => ownHashesRef.current.has(hash),
    [],
  );

  const resume = useCallback((hash: string) => {
    stoppedRef.current = false;
    hashRef.current = hash;
    ownHashesRef.current.add(hash);
    setStatus("idle");
  }, []);

  // Leaving the page must not drop an unflushed pause.
  useEffect(() => {
    const flush = () => {
      const value = pendingRef.current;
      if (value !== null) {
        pendingRef.current = null;
        clearTimer();
        void write(value);
      }
    };
    const onVisibility = () => {
      if (document.visibilityState === "hidden") {
        flush();
      }
    };
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("pagehide", flush);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("pagehide", flush);
      clearTimer();
    };
  }, [write]);

  return { status, savedAt, commit, touch, isOwnWrite, resume };
}
