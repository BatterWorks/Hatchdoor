import type { ExplorerNote } from "../../types";

export type WikilinkTrigger = {
  /** Text typed after the opening `[[`, up to the caret. */
  query: string;
  /** Index of the first `[` of the opening `[[`. */
  start: number;
};

/**
 * Detect whether the caret sits inside an open `[[` wikilink token and, if so,
 * return the query typed so far. Returns null when there is no active token
 * (already closed with `]]`, interrupted by `[`/`]`/newline, or no `[[`).
 */
export function getWikilinkTrigger(
  text: string,
  caret: number,
): WikilinkTrigger | null {
  const before = text.slice(0, caret);
  const open = before.lastIndexOf("[[");
  if (open === -1) {
    return null;
  }
  const between = before.slice(open + 2);
  if (
    between.includes("[") ||
    between.includes("]") ||
    between.includes("\n")
  ) {
    return null;
  }
  return { query: between, start: open };
}

/**
 * Replace the open wikilink token with `[[title]]` and report the new caret
 * position (just after the inserted token).
 */
export function applyWikilinkSelection(
  text: string,
  caret: number,
  start: number,
  title: string,
): { text: string; caret: number } {
  const inserted = `[[${title}]]`;
  return {
    text: text.slice(0, start) + inserted + text.slice(caret),
    caret: start + inserted.length,
  };
}

/** Case-insensitive substring match on title, capped at `limit` results. */
export function matchNoteCandidates(
  candidates: ExplorerNote[],
  query: string,
  limit = 8,
): ExplorerNote[] {
  const needle = query.trim().toLowerCase();
  const matches = needle
    ? candidates.filter((note) => note.title.toLowerCase().includes(needle))
    : candidates;
  return matches.slice(0, limit);
}
