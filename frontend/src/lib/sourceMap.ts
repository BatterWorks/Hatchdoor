// Maps between the markdown handed to the renderer and the lines of the file
// on disk.
//
// The renderer is fed a transform of the file:
//
//   file -> parseFrontmatter -> body -> stripBlockIds -> resolveWikilinks
//
// A rendered node's position gives line numbers in that transformed text, so
// slicing the right lines out of the file means adding back the frontmatter
// offset. That only holds while every step of the transform preserves line
// counts, which linesMatch checks at runtime.

import { parseFrontmatter } from "./markdown";

export type LineEnding = "\n" | "\r\n";

/**
 * How many lines parseFrontmatter removed from the front of `content`.
 *
 * Derived from parseFrontmatter's own output rather than by finding the
 * closing fence. parseFrontmatter returns the input unchanged in three
 * separate cases (too short, no closing fence, header that is not key: value),
 * and each of those means an offset of zero. Re-implementing the boundary here
 * would drift from looksLikeFrontmatterHeader and misaddress every block in
 * notes that merely open with a --- rule.
 */
export function frontmatterLineOffset(content: string): number {
  const { body } = parseFrontmatter(content);
  return countLines(content) - body.split("\n").length;
}

/**
 * The dominant line ending in `content`, reproduced on write so a CRLF file
 * does not silently become an LF file on the first block edit.
 */
export function detectLineEnding(content: string): LineEnding {
  const crlf = (content.match(/\r\n/g) ?? []).length;
  const lf = (content.match(/(?<!\r)\n/g) ?? []).length;
  return crlf > lf ? "\r\n" : "\n";
}

function countLines(content: string): number {
  return content.split(/\r?\n/).length;
}
