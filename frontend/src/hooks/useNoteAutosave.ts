import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";

/** How long typing inside one unit may pause before the document is flushed. */
const IDLE_FLUSH_MS = 2000;

export type AutosaveStatus = "idle" | "saving" | "saved" | "conflict" | "error";

export type SaveResult = {
  content_hash?: string | null;
  /** The file the server wrote, as the note read spells it (#627). */
  relative_path?: string | null;
};

/** How the last send of a save loop ended, for a later visit of the same note
 * to carry on from (#803). `wroteOver` holds every hash the loop wrote
 * against, oldest first: a loop left running by an unmounted page drains
 * its queue, so a return may have read the note at any of them (#807). */
type SaveOutcome = {
  content: string;
  sentAgainst: string;
  wroteOver: string[];
} & ({ ok: true; result: SaveResult } | { ok: false; error: unknown });

/** A visit of a note, as the hook tells one from another: a token it makes
 * itself, since the page's own key can repeat on a fresh mount (#807). */
type Visit = object;

/** A note's save loop still out, and the visit that started it. */
type OutSave = {
  visit: Visit;
  settled: Promise<SaveOutcome | null>;
};

/** Every note's save still out, and every save that answered after its visit
 * was left, by `noteId` (#803). A page that can be unmounted between two
 * visits of a note, as the note page is by a trip to Settings, keeps one of
 * these outside itself and hands it to every mount, so a return carries on
 * from the save the last mount left out (#807). */
export type NoteSaves = {
  out: Map<string, OutSave>;
  left: Map<string, SaveOutcome>;
};

export const createNoteSaves = (): NoteSaves => ({
  out: new Map(),
  left: new Map(),
});

const isConflict = (error: unknown) =>
  error instanceof Error && error.name === "ConflictError";

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
 *
 * Everything it holds belongs to the one note named by `noteKey`. The page is
 * reused across notes, and a save takes about a second, so a reply for the note
 * just left routinely lands after the next one opened (#658). That reply still
 * reaches its own `onSaved`, but never this note's hash, status or queue.
 *
 * A return to the same note is a new visit with a new key, but the same
 * `noteId`. When the earlier visit's save is still out, the new visit's edits
 * wait for it and are then written against the hash it produced, so the two
 * never race each other into a false conflict (#803). A page given a `saves`
 * record that outlives it does the same across an unmount: the save loop the
 * old mount left running finishes its queue, and the next mount carries on
 * from how it ended (#807).
 */
export function useNoteAutosave({
  noteKey,
  noteId = noteKey,
  baseHash,
  enabled,
  save,
  flushSave,
  onSaved,
  saves: givenSaves,
}: {
  /** Which note the edits are for. A new key starts afresh: saving on, the
   * current `baseHash`, nothing pending, nothing in flight. */
  noteKey: string;
  /** Which note this visit is of. A later visit with the same `noteId` takes
   * over a save the earlier one left out: it waits for it, moves on to the hash
   * it produced, and does not send the same text again (#803). Change it when
   * the same address may name another file. Defaults to `noteKey`, so no two
   * visits share a save. */
  noteId?: string;
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
  /** `sentAgainst` is the hash the save was written against, which a reply
   * for the note just left needs to tell its own held edit from another
   * (#695). */
  onSaved?: (result: SaveResult, content: string, sentAgainst: string) => void;
  /** Where saves still out are kept between visits (#807). Defaults to a
   * record of this hook's own, which ends with it. */
  saves?: NoteSaves;
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
  const noteKeyRef = useRef(noteKey);
  const noteIdRef = useRef(noteId);
  noteIdRef.current = noteId;
  // The visit open now, replaced when the note changes, so a send still out
  // can tell its visit has been left. An unmount keeps it: the loop running
  // then drains its queue as before, and `unmountedRef` says it was left (#807).
  const visitRef = useRef<Visit>({});
  // Each note's save loop still out (`out`), so a save still out for the
  // note just left neither holds this one's edits back nor drains them into
  // its own send, and a return to that note can wait for it; and how a
  // note's save ended when its visit had already been left (`left`), kept
  // until the next visit of that note first writes (#803).
  const [ownSaves] = useState(createNoteSaves);
  const savesRef = useRef(givenSaves ?? ownSaves);
  savesRef.current = givenSaves ?? ownSaves;
  // The visit waiting on an earlier visit's save of the same note (#803).
  const takingOverRef = useRef<Visit | null>(null);
  // The visit whose text was restored from the note's held draft, and so
  // carries every edit an earlier visit sent (#803). Only such a visit may
  // take over the hash an earlier save produced: one showing the note as it
  // was read does not hold that save's edit, and writing it on that hash
  // would erase the edit without a word.
  const restoredRef = useRef<Visit | null>(null);
  const queuedRef = useRef<string | null>(null);
  const stoppedRef = useRef(false);
  const unsavedRef = useRef(false);
  const callbacksRef = useRef({ save, flushSave, onSaved });
  // New render closures must not tear down the idle timer. Capture them when
  // a write starts, so an awaited write also keeps its original recipient.
  useLayoutEffect(() => {
    callbacksRef.current = { save, flushSave, onSaved };
  }, [save, flushSave, onSaved]);

  // A layout effect, so the reset lands before any effect of the page's can
  // send for the new note.
  useLayoutEffect(() => {
    if (noteKeyRef.current === noteKey) {
      return;
    }
    noteKeyRef.current = noteKey;
    visitRef.current = {};
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    pendingRef.current = null;
    queuedRef.current = null;
    unsavedRef.current = false;
    stoppedRef.current = false;
    hashRef.current = baseHash;
    ownHashesRef.current = new Set([baseHash]);
    setStatus("idle");
  }, [noteKey, baseHash]);

  // Set while the page is unmounted (#807). A save loop already running goes
  // on draining its queue, as it always has, since nothing else would write
  // that edit, and books how it ended for the next mount; nothing new starts.
  const unmountedRef = useRef(false);
  useEffect(() => {
    unmountedRef.current = false;
    return () => {
      unmountedRef.current = true;
    };
  }, []);

  /** A save of this visit is out, or one of an earlier visit it waits on. */
  const savingThisVisit = () =>
    savesRef.current.out.get(noteIdRef.current)?.visit === visitRef.current ||
    takingOverRef.current === visitRef.current;

  useEffect(() => {
    if (!unsavedRef.current && !savingThisVisit()) {
      hashRef.current = baseHash;
      ownHashesRef.current.add(baseHash);
    }
  }, [baseHash]);

  const clearTimer = () => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  };

  /**
   * Carry on from the save an earlier visit of this note ended with (#803),
   * returning the text this visit still has to send, or null. When this
   * visit's text was restored from the held draft and that save's loop wrote
   * over the version this visit read, its hash is this visit's too, booked
   * through this visit's `onSaved`, so a newer edit is written
   * after it rather than beside it against the version it replaced. The same
   * text again is not sent twice: written, it is saved; refused, sending it
   * again would be refused the same way, so it stays unsaved here and in the
   * draft, as a refusal for a note you left always leaves it (#658).
   */
  const carryOn = useCallback(
    (ended: SaveOutcome | null, next: string | null): string | null => {
      if (restoredRef.current !== visitRef.current) {
        return next;
      }
      if (ended?.ok) {
        const hash = ended.result?.content_hash;
        if (!hash || !ended.wroteOver.includes(hashRef.current)) {
          return next;
        }
        hashRef.current = hash;
        ownHashesRef.current.add(hash);
        callbacksRef.current.onSaved?.(
          ended.result,
          ended.content,
          ended.sentAgainst,
        );
        if (next !== ended.content) {
          return next;
        }
        unsavedRef.current = pendingRef.current !== null;
        setStatus(unsavedRef.current ? "idle" : "saved");
        if (!unsavedRef.current) setSavedAt(new Date());
        return null;
      }
      if (
        ended &&
        isConflict(ended.error) &&
        hashRef.current === ended.sentAgainst &&
        next === ended.content
      ) {
        setStatus("idle");
        return null;
      }
      return next;
    },
    [],
  );

  const write = useCallback(
    // `sendFirst` is the unload send (#330), used for this write only: anything
    // that drains out of the queue afterwards belongs to a page that is still
    // here, so it goes by the ordinary awaited route.
    async (next: string | null, sendFirst?: Sender) => {
      if (next === null || !enabled || stoppedRef.current) {
        return;
      }
      // A write takes about a second against a real vault and every content
      // change commits, so edits arriving mid-flight are routine. Queue the
      // newest one rather than dropping it: dropping loses the edit outright
      // and still reports "Saved".
      if (savingThisVisit()) {
        queuedRef.current = next;
        return;
      }
      // An earlier visit of this note left a save out, or one that answered
      // after it was left: this visit carries on from it (#803).
      const id = noteIdRef.current;
      const saves = savesRef.current;
      const earlier = saves.out.get(id);
      if (earlier) {
        const waiting = visitRef.current;
        queuedRef.current = next;
        takingOverRef.current = waiting;
        setStatus("saving");
        const ended = await earlier.settled;
        if (takingOverRef.current === waiting) {
          takingOverRef.current = null;
        }
        if (
          visitRef.current !== waiting ||
          stoppedRef.current ||
          unmountedRef.current
        ) {
          return;
        }
        saves.left.delete(id);
        next = carryOn(ended, queuedRef.current);
        queuedRef.current = null;
        // Nothing has awaited since the wait ended, so no other send of this
        // visit can have started: this one goes out as an ordinary write.
        sendFirst = undefined;
      } else if (saves.left.has(id)) {
        const ended = saves.left.get(id)!;
        saves.left.delete(id);
        next = carryOn(ended, next);
      }
      if (next === null) {
        return;
      }

      const visit = visitRef.current;
      const wroteOver: string[] = [];
      let landed: SaveOutcome | null = null;
      let settle: (landed: SaveOutcome | null) => void = () => {};
      const mine: OutSave = {
        visit,
        settled: new Promise((resolve) => {
          settle = resolve;
        }),
      };
      saves.out.set(id, mine);
      const { save, onSaved } = callbacksRef.current;
      try {
        let current: string | null = next;
        let send = sendFirst ?? save;
        while (current !== null && !stoppedRef.current) {
          setStatus("saving");
          const sentAgainst = hashRef.current;
          wroteOver.push(sentAgainst);
          try {
            const result = await send(current, sentAgainst);
            landed = {
              ok: true,
              result,
              content: current,
              sentAgainst,
              wroteOver: [...wroteOver],
            };
            if (visitRef.current !== visit) {
              // The page has moved on. The note this was for still books it
              // (its draft, through the closure captured above); the open
              // note's hash and queue are not this send's to touch.
              onSaved?.(result, current, sentAgainst);
              return;
            }
            if (result?.content_hash) {
              hashRef.current = result.content_hash;
              ownHashesRef.current.add(result.content_hash);
            }
            onSaved?.(result, current, sentAgainst);
          } catch (error) {
            // A send that failed without a refusal leaves the last one this
            // loop wrote as the newest it knows of, so a later visit carries
            // on from that one rather than from a hash the vault has moved
            // past. Had the failed one landed after all, the hash check
            // refuses the next send rather than overwrite it (#807).
            if (!landed?.ok || isConflict(error)) {
              landed = {
                ok: false,
                error,
                content: current,
                sentAgainst,
                wroteOver: [...wroteOver],
              };
            }
            if (visitRef.current !== visit) {
              // Refused for the note just left: its draft stays held for its
              // next visit, and the open note keeps saving.
              return;
            }
            stoppedRef.current = true;
            queuedRef.current = null;
            setStatus(isConflict(error) ? "conflict" : "error");
            return;
          }

          send = save;
          current = queuedRef.current;
          queuedRef.current = null;
          if (current === null) {
            unsavedRef.current = pendingRef.current !== null;
            setStatus(unsavedRef.current ? "idle" : "saved");
            if (!unsavedRef.current) setSavedAt(new Date());
          }
        }
      } finally {
        if (saves.out.get(id) === mine) {
          saves.out.delete(id);
        }
        if (landed && (visitRef.current !== visit || unmountedRef.current)) {
          saves.left.set(id, landed);
        }
        settle(landed);
      }
    },
    [enabled, carryOn],
  );

  /** A unit was committed: write now. */
  const commit = useCallback(
    (next: string) => {
      clearTimer();
      unsavedRef.current = true;
      pendingRef.current = null;
      void write(next);
    },
    [write],
  );

  /** Commit text restored from the note's held draft (#330), which carries
   * every edit an earlier visit of this note sent, so this visit may carry on
   * from a save that visit left out (#803). */
  const commitRestored = useCallback(
    (next: string) => {
      restoredRef.current = visitRef.current;
      commit(next);
    },
    [commit],
  );

  /** Still typing inside one unit: flush after the idle pause. */
  const touch = useCallback(
    (next: string) => {
      unsavedRef.current = true;
      if (!stoppedRef.current) setStatus(savingThisVisit() ? "saving" : "idle");
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

  /** Drop every edit not yet sent: the page is about to open another file
   * at this address, and a pause or queue left over would write this one's
   * text there (#627). The page keeps that text in its draft. */
  const discard = useCallback(() => {
    clearTimer();
    pendingRef.current = null;
    queuedRef.current = null;
    unsavedRef.current = false;
  }, []);

  const hasUnsaved = useCallback(() => unsavedRef.current, []);

  useEffect(() => {
    const warn = (event: BeforeUnloadEvent) => {
      if (unsavedRef.current) {
        event.preventDefault();
        event.returnValue = "";
      }
    };
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, []);

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
      const { flushSave } = callbacksRef.current;
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
        savingThisVisit() &&
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
  }, [write, enabled]);

  return {
    status,
    savedAt,
    commit,
    commitRestored,
    touch,
    isOwnWrite,
    resume,
    discard,
    hasUnsaved,
  };
}
