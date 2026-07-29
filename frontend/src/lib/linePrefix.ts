/**
 * The leading markdown syntax on a line: list marker, task box, heading hashes,
 * or quote arrows.
 *
 * This is the part of the source that has no rendered counterpart, so it is
 * what has to hang into the gutter for the visible text to stay put when a
 * block is entered.
 */
const LINE_PREFIX =
  /^(?:\s*(?:[-*+]|\d+[.)])\s+(?:\[[ xX]\]\s+)?|#{1,6}\s+|(?:>\s?)+)/;

export function linePrefix(line: string): string {
  return LINE_PREFIX.exec(line)?.[0] ?? "";
}
