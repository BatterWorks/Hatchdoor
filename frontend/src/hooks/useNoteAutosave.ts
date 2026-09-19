import { useCallback, useEffect, useRef, useState } from "react";

/** How long typing inside one unit may pause before the document is flushed. */
const IDLE_FLUSH_MS = 2000;

export type AutosaveStatus = "idle" | "saving" | "saved" | "conflict" | "error";

export type SaveResult = { content_hash?: string | null };

/** One attempt to put the whole document, against the hash it is an edit of. */
export type Sender = (
  content: string,
  expectedHash: string,
) => Promise<SaveResult>;

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
  flushSave,
  onSaved,
}: {
  baseHash: string;
  enabled: boolean;
  save: (content: string, expectedHash: string) => Promise<SaveResult>;
  /**
   * The leaving-the-page send. `save` is awaited, and a document being torn
   * down never delivers the response, so the page supplies a send with
   * keepalive semantics for the `pagehide`/`visibilitychange` flush (#330).
   * Without one the flush falls back to `save`, which is lossy on a close.
   *
   * It returns its outcome like `save` does, and the hook books that outcome
   * in exactly the same way. A page that really is going away never resolves
   * it and nothing is lost by that; a page that was merely hidden comes back
   * with the hash moved on, instead of saving against a hash the vault has
   * already superseded and conflicting on the user's next keystroke.
   */
  flushSave?: Sender;
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
    // `sendFirst` is the unload send (#330), used for this write only: anything
    // that drains out of the queue afterwards belongs to a page that is still
    // here, so it goes by the ordinary awaited route.
    async (next: string, sendFirst?: Sender) => {
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
        let send = sendFirst ?? save;
        while (current !== null && !stoppedRef.current) {
          setStatus("saving");
          try {
            const result = await send(current, hashRef.current);
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

          send = save;
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
    // `terminating` separates the two events this listens to. `pagehide` is the
    // document stopping: nothing queued here will ever be drained by this page.
    // Hiding a tab is not — the page keeps running, so the ordinary queue is
    // both available and more correct than a second send racing the first.
    const flush = (terminating: boolean) => {
      // `pendingRef` first, then `queuedRef`: `commit` clears the pending
      // pause, so whenever both hold something the pending one is the later
      // snapshot of the whole document and already contains the queued edit.
      // Taking only `pendingRef`, as this used to, dropped an edit that
      // `write` had parked behind an in-flight save (#330).
      const value = pendingRef.current ?? queuedRef.current;
      if (value === null) {
        return;
      }
      pendingRef.current = null;
      clearTimer();

      // A save already in flight owns the hash the next one must be written
      // against, so an edit arriving now belongs behind it — as long as there
      // is a page left to drain the queue. On the way out there is not, so the
      // edit leaves as its own keepalive send and stays queued for the case
      // where the page survives after all. Whichever of the two reaches the
      // vault second is refused by the hash guard rather than overwriting, and
      // the draft written alongside covers the refused one.
      if (
        terminating &&
        inFlightRef.current &&
        flushSave &&
        enabled &&
        !stoppedRef.current
      ) {
        queuedRef.current = value;
        void flushSave(value, hashRef.current).catch(() => {
          // Nothing can be reported to a page that is already gone.
        });
        return;
      }

      queuedRef.current = null;
      // Routed through `write` rather than sent beside it, so the result is
      // booked the way every other save is: a page that was only hidden keeps
      // a current hash, a live save state, and its `onSaved` (#330). One that
      // is genuinely going away never resolves this, which costs nothing.
      void write(value, flushSave);
    };
    const onVisibility = () => {
      if (document.visibilityState === "hidden") {
        flush(false);
      }
    };
    const onPageHide = () => flush(true);
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("pagehide", onPageHide);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("pagehide", onPageHide);
      clearTimer();
    };
  }, [write, flushSave, enabled]);

  return { status, savedAt, commit, touch, isOwnWrite, resume };
}
