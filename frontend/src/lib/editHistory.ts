// Document-level undo.
//
// Native textarea undo is deliberately not mixed in: a textarea's stack dies
// when it unmounts, which happens every time a block is left, it cannot cover
// changes made to the document above it, and it cannot be queried for remaining
// history. Owning the stack produces undo that spans the whole note.

/** How long a run of typing may pause before it becomes a separate undo step. */
const COALESCE_MS = 500;

export type HistoryEntry = { content: string };

export function createEditHistory(initial: string) {
  const past: string[] = [];
  let present = initial;
  const future: string[] = [];
  let lastRecordedAt = 0;
  let runOpen = false;

  return {
    /**
     * Record a new document state. Consecutive edits within the pause window
     * collapse into the run already in progress, so continuous typing is one
     * undo rather than one per keystroke.
     */
    record(content: string, at: number) {
      if (content === present) {
        return;
      }
      const continuesRun = runOpen && at - lastRecordedAt < COALESCE_MS;
      if (!continuesRun) {
        past.push(present);
      }
      present = content;
      lastRecordedAt = at;
      runOpen = true;
      future.length = 0;
    },

    /**
     * End the current run, so the next edit starts a new undo step. Called on
     * structural operations and when moving between units.
     */
    breakRun() {
      runOpen = false;
    },

    undo(): HistoryEntry | null {
      const previous = past.pop();
      if (previous === undefined) {
        return null;
      }
      future.push(present);
      present = previous;
      runOpen = false;
      return { content: present };
    },

    redo(): HistoryEntry | null {
      const next = future.pop();
      if (next === undefined) {
        return null;
      }
      past.push(present);
      present = next;
      runOpen = false;
      return { content: present };
    },

    current(): string {
      return present;
    },

    /** Start again from `content`, discarding all history. */
    reset(content: string) {
      past.length = 0;
      future.length = 0;
      present = content;
      lastRecordedAt = 0;
      runOpen = false;
    },
  };
}
