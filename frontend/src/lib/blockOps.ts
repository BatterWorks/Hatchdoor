// Structural edits as pure string transforms.
//
// This is where the correctness risk of inline editing lives: an off-by-one
// here writes to the wrong line of the user's file. Nothing in here touches the
// DOM, so every case is reachable from a test.

import { detectLineEnding, type LineRange } from "./sourceMap";

export type BlockOpResult = {
  content: string;
  caretLine: number;
  caretOffset: number;
};

/** The marker that opens a list item, task, or quote line. */
const LIST_MARKER = /^(\s*)([-*+]|\d+[.)])(\s+)(\[[ xX]\]\s+)?/;
const QUOTE_MARKER = /^((?:>\s?)+)/;

function splitLines(content: string): string[] {
  return content.split(/\r?\n/);
}

function join(lines: string[], content: string): string {
  return lines.join(detectLineEnding(content));
}

/**
 * The prefix a continuation line should carry, or "" when the next line is a
 * plain paragraph.
 *
 * Ordered lists are deliberately not renumbered: renderers ignore the literal
 * numbers, and rewriting them would touch lines outside the edited range.
 */
function continuationPrefix(line: string): string {
  const list = LIST_MARKER.exec(line);
  if (list) {
    const [, indent, marker, gap, task] = list;
    // A new task always starts unchecked, whatever the source line was.
    return `${indent}${marker}${gap}${task ? "[ ] " : ""}`;
  }

  const quote = QUOTE_MARKER.exec(line);
  if (quote) {
    return quote[1];
  }

  return "";
}

/**
 * Split the block at `caret`, an offset into the block's own source text.
 *
 * A paragraph or heading splits into two blocks separated by a blank line,
 * because markdown needs one. A list item or quote line continues its prefix.
 */
export function splitBlock(
  content: string,
  range: LineRange,
  caret: number,
): BlockOpResult {
  const lines = splitLines(content);
  const blockLines = lines.slice(range.startLine - 1, range.endLine);
  const text = blockLines.join("\n");
  const at = Math.max(0, Math.min(caret, text.length));
  const before = text.slice(0, at);
  const after = text.slice(at);

  const firstLine = blockLines[0] ?? "";
  const prefix = continuationPrefix(firstLine);

  const inserted = prefix
    ? [...before.split("\n"), `${prefix}${after}`]
    : [...before.split("\n"), "", after];

  const next = [
    ...lines.slice(0, range.startLine - 1),
    ...inserted,
    ...lines.slice(range.endLine),
  ];

  const caretLine = range.startLine - 1 + inserted.length;
  return {
    content: join(next, content),
    caretLine,
    caretOffset: prefix.length,
  };
}

/**
 * Join the block into the one above it, as Backspace at offset 0 does.
 *
 * Returns null when there is no previous unit. The caller supplies that range,
 * because only it knows which lines a block owns: merging across a range no
 * block owns would silently absorb or delete it.
 */
export function mergeBlockUp(
  content: string,
  range: LineRange,
  previousRange: LineRange | null,
): BlockOpResult | null {
  if (!previousRange || previousRange.endLine >= range.startLine) {
    return null;
  }

  const lines = splitLines(content);
  const previous = lines[previousRange.endLine - 1] ?? "";
  const block = lines.slice(range.startLine - 1, range.endLine).join("\n");

  // The merged text keeps the previous block's prefix and drops its own.
  const ownPrefix = continuationPrefix(block.split("\n")[0] ?? "");
  const body = block.slice(ownPrefix.length);

  const merged = `${previous}${body}`;
  const next = [
    ...lines.slice(0, previousRange.endLine - 1),
    merged,
    ...lines.slice(range.endLine),
  ];

  return {
    content: join(next, content),
    caretLine: previousRange.endLine,
    caretOffset: previous.length,
  };
}

/** Nest a list item one level deeper. Null when that is not meaningful. */
export function indentListItem(
  content: string,
  range: LineRange,
): { content: string } | null {
  const lines = splitLines(content);
  const line = lines[range.startLine - 1];
  if (line === undefined || !LIST_MARKER.test(line)) {
    return null;
  }

  // The first item of a list has nothing to nest under; markdown would read
  // the indented line as a new list rather than a child.
  const above = lines[range.startLine - 2];
  if (above === undefined || !LIST_MARKER.test(above)) {
    return null;
  }

  lines[range.startLine - 1] = `  ${line}`;
  return { content: join(lines, content) };
}

/** Lift a list item one level out. Null when it is already at the margin. */
export function outdentListItem(
  content: string,
  range: LineRange,
): { content: string } | null {
  const lines = splitLines(content);
  const line = lines[range.startLine - 1];
  if (line === undefined || !LIST_MARKER.test(line)) {
    return null;
  }
  if (!line.startsWith("  ")) {
    return null;
  }

  lines[range.startLine - 1] = line.slice(2);
  return { content: join(lines, content) };
}

/** Flip a task list checkbox on `line` (1-indexed). */
export function toggleCheckbox(content: string, line: number): string {
  const lines = splitLines(content);
  const target = lines[line - 1];
  if (target === undefined) {
    return content;
  }

  const match = /^(\s*(?:[-*+]|\d+[.)])\s+)\[([ xX])\]/.exec(target);
  if (!match) {
    return content;
  }

  const checked = match[2] !== " ";
  lines[line - 1] =
    `${match[1]}[${checked ? " " : "x"}]${target.slice(match[0].length)}`;
  return join(lines, content);
}
