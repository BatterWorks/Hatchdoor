/**
 * The leading source characters on a line that render as nothing: its
 * indentation, then any list marker, task box, heading hashes, or quote arrows
 * behind it.
 *
 * This is the part of the source that has no rendered counterpart, so it is
 * what has to hang into the gutter for the visible text to stay put when a
 * block is entered, and what a caret has to step past before the first
 * character it can land on.
 *
 * Indentation counts on its own, with no marker required. A wrapped list item
 * is addressed one source line at a time (D25a), so its continuation line
 * arrives as a block of its own: indent at the front, no marker to hang it on,
 * and none of it on screen either way. Reading that indent as visible text was
 * #286.
 */
const LINE_PREFIX =
  /^[ \t]*(?:(?:[-*+]|\d+[.)])[ \t]+(?:\[[ xX]\][ \t]+)?|#{1,6}[ \t]+|(?:>[ \t]?)+)?/;

export function linePrefix(line: string): string {
  return LINE_PREFIX.exec(line)?.[0] ?? "";
}
