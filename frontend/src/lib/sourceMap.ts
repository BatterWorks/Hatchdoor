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
  /** hast elements carry their kind here; mdast nodes use `type`. */
  tagName?: string;
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
  if (!isListItem(node)) {
    return fallbackEnd;
  }

  for (const child of node?.children ?? []) {
    const childNode = child as PositionedNode;
    if (!isList(childNode)) {
      continue;
    }
    const childStart = childNode.position?.start?.line;
    if (typeof childStart === "number") {
      return childStart - 1;
    }
  }

  return fallbackEnd;
}

// Accepts both shapes: react-markdown passes hast (tagName "li"), while mdast
// uses type "listItem". Checking only one silently disables the rule.
function isListItem(node: PositionedNode | undefined): boolean {
  return node?.type === "listItem" || node?.tagName === "li";
}

function isList(node: PositionedNode | undefined): boolean {
  return (
    node?.type === "list" || node?.tagName === "ul" || node?.tagName === "ol"
  );
}

/**
 * Stands in for a blank line so the renderer emits a paragraph node there.
 *
 * A zero-width space, because CommonMark does not count it as whitespace: a
 * line holding one is a paragraph, where a line holding a space is nothing at
 * all. It is never seen. An active block renders its input in place of its
 * children, and this substitution only ever happens on the active range.
 */
export const BLANK_LINE_PLACEHOLDER = "​";

/**
 * Give the active range something to render, when the line the caret is on is
 * blank.
 *
 * Splitting a paragraph, leaving a list, and clicking between two blocks all
 * put the caret on a blank line. A blank line parses to no node, a node is
 * what carries the position an editable block is addressed by, so without this
 * the input has nowhere to live and the user is silently dropped out of
 * editing mid-keystroke.
 *
 * The substitution is made only in the text handed to the renderer. The file
 * keeps its blank line, and sourceOf still slices the real content, so the
 * input opens empty rather than holding a character the user never typed.
 */
export function placeholderForBlankRange(
  content: string,
  range: LineRange | null,
): string {
  // Only ever one line: a blank block is a single line by construction, and
  // widening this would blank out real content on a range that merely starts
  // with an empty line.
  if (!range || range.startLine !== range.endLine) {
    return content;
  }

  const lines = splitLines(content);
  const at = range.startLine - 1;
  const line = lines[at];
  if (line === undefined || line.trim() !== "") {
    return content;
  }

  lines[at] = BLANK_LINE_PLACEHOLDER;
  return join(lines, content);
}

function join(lines: string[], content: string): string {
  return lines.join(detectLineEnding(content));
}

function splitLines(content: string): string[] {
  return content.split(/\r?\n/);
}

function countLines(content: string): number {
  return splitLines(content).length;
}
