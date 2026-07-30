// Structural edits as pure string transforms.
//
// This is where the correctness risk of inline editing lives: an off-by-one
// here writes to the wrong line of the user's file. Nothing in here touches the
// DOM, so every case is reachable from a test.

import { detectLineEnding, type LineRange } from "./sourceMap";

export type BlockOpResult = {
  content: string;
  caretLine: number;
  /** Last line of the block the caret ends up in; equals caretLine when it is one line. */
  caretEndLine: number;
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

  // Each element must be exactly one line: a multi-line string here would keep
  // its embedded LF through a CRLF join and quietly mix line endings.
  const inserted = prefix
    ? [...before.split("\n"), ...`${prefix}${after}`.split("\n")]
    : [...before.split("\n"), "", ...after.split("\n")];

  const next = [
    ...lines.slice(0, range.startLine - 1),
    ...inserted,
    ...lines.slice(range.endLine),
  ];

  // The remainder can span several lines, and the caller builds the active
  // range from these, so reporting only its first line would open an input
  // showing part of the block.
  const remainderLines = prefix
    ? `${prefix}${after}`.split("\n").length
    : after.split("\n").length;
  const caretEndLine = range.startLine - 1 + inserted.length;
  return {
    content: join(next, content),
    caretLine: caretEndLine - remainderLines + 1,
    caretEndLine,
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

  // Anything between the two blocks that is not blank belongs to no block:
  // raw HTML, a link reference definition, or display math, none of which
  // reach the renderer as a positioned node. Absorbing one into the merge
  // would delete it, so refuse rather than guess.
  const between = lines.slice(previousRange.endLine, range.startLine - 1);
  if (between.some((line) => line.trim() !== "")) {
    return null;
  }
  const previous = lines[previousRange.endLine - 1] ?? "";
  const block = lines.slice(range.startLine - 1, range.endLine).join("\n");

  // The merged text keeps the previous block's prefix and drops its own.
  const ownPrefix = continuationPrefix(block.split("\n")[0] ?? "");
  const body = block.slice(ownPrefix.length);

  const next = [
    ...lines.slice(0, previousRange.endLine - 1),
    ...`${previous}${body}`.split("\n"),
    ...lines.slice(range.endLine),
  ];

  return {
    content: join(next, content),
    caretLine: previousRange.endLine,
    caretEndLine: previousRange.endLine,
    caretOffset: previous.length,
  };
}

/**
 * Enter on an item with nothing in it leaves the list, the way a word
 * processor does, instead of adding another empty bullet the user then has to
 * delete.
 *
 * It steps out one level at a time: a nested item lifts to its parent's level
 * and only a top-level one becomes a plain line. That makes Enter the way back
 * out of a deep list, matching what Shift-Tab does a level at a time.
 *
 * Returns null when this is not an empty item, which is the caller's signal to
 * split normally.
 */
export function exitEmptyListItem(
  content: string,
  range: LineRange,
): BlockOpResult | null {
  // An empty item is one line by construction. A wider range means the block
  // holds continuation lines, so there is something here to split.
  if (range.startLine !== range.endLine) {
    return null;
  }

  const lines = splitLines(content);
  const line = lines[range.startLine - 1];
  if (line === undefined) {
    return null;
  }

  const marker = LIST_MARKER.exec(line);
  if (!marker || line.slice(marker[0].length).trim() !== "") {
    return null;
  }

  const [, indent] = marker;
  if (indent.length >= 2) {
    // Lift one level, keeping the marker so the user carries on typing in the
    // list they are still in.
    // Sliced, not trimmed: the space after the marker is where the caret goes,
    // and trimming it would put the caret tight against the bullet.
    const lifted = line.slice(2);
    lines[range.startLine - 1] = lifted;
    return {
      content: join(lines, content),
      caretLine: range.startLine,
      caretEndLine: range.startLine,
      caretOffset: lifted.length,
    };
  }

  // Out of the list entirely, which takes two lines rather than one. The first
  // blank line is what ends the list; the caret goes on the second. Landing on
  // the first instead would put the caret on a line markdown reads as a lazy
  // continuation of the item above, so the "paragraph" the user then typed
  // would silently rejoin the bullet they just left.
  const next = [
    ...lines.slice(0, range.startLine - 1),
    "",
    "",
    ...lines.slice(range.startLine),
  ];
  return {
    content: join(next, content),
    caretLine: range.startLine + 1,
    caretEndLine: range.startLine + 1,
    caretOffset: 0,
  };
}

/**
 * Open an empty line after `afterLine`, for a click that landed between two
 * blocks rather than on one.
 *
 * The new line is padded clear of its neighbours, because an unpadded line
 * next to a paragraph or a list item is read as a continuation of it: what the
 * user typed would join the block above instead of becoming its own. Padding
 * is skipped where a blank line already does that job.
 *
 * `afterLine` is 1-indexed and may be 0, which opens a line above everything.
 */
export function insertParagraphAt(
  content: string,
  afterLine: number,
): BlockOpResult {
  const lines = splitLines(content);
  const at = Math.max(0, Math.min(afterLine, lines.length));

  const padBefore = at > 0 && lines[at - 1].trim() !== "";
  const padAfter = at < lines.length && lines[at].trim() !== "";

  const next = [
    ...lines.slice(0, at),
    ...(padBefore ? [""] : []),
    "",
    ...(padAfter ? [""] : []),
    ...lines.slice(at),
  ];
  const caretLine = at + (padBefore ? 1 : 0) + 1;

  return {
    content: join(next, content),
    caretLine,
    caretEndLine: caretLine,
    caretOffset: 0,
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
