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

/**
 * The text of lines `startLine` through `endLine` inclusive, 1-indexed.
 */
export function sliceLines(
  content: string,
  startLine: number,
  endLine: number,
): string {
  return splitLines(content)
    .slice(startLine - 1, endLine)
    .join("\n");
}

/**
 * `content` with lines `startLine` through `endLine` inclusive replaced by
 * `replacement`, keeping the file's own line ending.
 */
export function replaceLines(
  content: string,
  startLine: number,
  endLine: number,
  replacement: string,
): string {
  const ending = detectLineEnding(content);
  const lines = splitLines(content);
  const next = [
    ...lines.slice(0, startLine - 1),
    ...replacement.split("\n"),
    ...lines.slice(endLine),
  ];
  return next.join(ending);
}

/**
 * Whether a transformed body still has one line per source line.
 *
 * Inline editing addresses blocks by line number, so a transform that collapses
 * lines makes every block below it write to the wrong place. Callers that get
 * false here must fall back to source mode rather than editing by line.
 */
export function linesMatch(source: string, transformed: string): boolean {
  return countLines(source) === countLines(transformed);
}

export type LineRange = { startLine: number; endLine: number };

type PositionedNode = {
  type?: string;
  children?: unknown[];
  position?: {
    start?: { line?: number };
    end?: { line?: number };
  };
};

/**
 * The file lines a rendered node owns, or null if it owns none.
 *
 * Null is a normal answer, not an error. Display math, raw HTML blocks, link
 * reference definitions, and the generated footnote section all reach the
 * renderer with no position, and callers must render them unchanged and
 * non-editable rather than guessing a range.
 */
export function blockRange(
  node: unknown,
  frontmatterOffset: number,
): LineRange | null {
  const positioned = node as PositionedNode | undefined;
  const start = positioned?.position?.start?.line;
  const end = positioned?.position?.end?.line;

  if (typeof start !== "number" || typeof end !== "number") {
    return null;
  }

  return {
    startLine: start + frontmatterOffset,
    endLine: listItemEndLine(positioned, end) + frontmatterOffset,
  };
}

/**
 * A list item's own lines stop where its first nested list begins. Without
 * this, clicking one bullet drops the entire nested list into raw markdown,
 * which is the pain inline editing exists to remove.
 */
function listItemEndLine(
  node: PositionedNode | undefined,
  fallbackEnd: number,
): number {
  if (node?.type !== "listItem") {
    return fallbackEnd;
  }

  for (const child of node.children ?? []) {
    const childNode = child as PositionedNode;
    if (childNode?.type !== "list") {
      continue;
    }
    const childStart = childNode.position?.start?.line;
    if (typeof childStart === "number") {
      return childStart - 1;
    }
  }

  return fallbackEnd;
}

function splitLines(content: string): string[] {
  return content.split(/\r?\n/);
}

function countLines(content: string): number {
  return splitLines(content).length;
}
