// Maps a caret position in rendered text back to an offset in the markdown
// source that produced it.
//
// The mapping is approximate by nature: markdown syntax characters do not exist
// in the rendered text, so there is no exact answer. Landing a few characters
// off is acceptable; always landing at offset 0, or always at the end, is not.

import { linePrefix } from "./linePrefix";

/** Opens or closes a fenced code block. The whole line renders as nothing. */
const FENCE = /^\s*(?:```|~~~)/;

/**
 * The source offset corresponding to `renderedOffset` characters of rendered
 * text in the block `source`, which may span more than one line.
 *
 * Every line is measured against its own leading syntax. Indentation renders as
 * nothing wherever it sits, and counting it as text put the caret short of the
 * clicked character by the width of the indent, worse the deeper a list nests
 * (#284).
 */
export function sourceOffsetForRenderedOffset(
  source: string,
  renderedOffset: number,
): number {
  const target = Math.max(0, renderedOffset);
  const lines = source.split("\n");

  if (FENCE.test(lines[0])) {
    return codeBlockOffset(lines, target);
  }

  let rendered = 0;
  let lineStart = 0;
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    const walk = walkLine(line, target - rendered);
    if (walk.hit !== null) {
      return lineStart + walk.hit;
    }
    rendered += walk.rendered;
    lineStart += line.length + 1;

    if (i === lines.length - 1) {
      break;
    }
    // The soft break between two lines of one block is one character on each
    // side, so a caret that lands on it belongs at the end of the line it
    // closes rather than at the front of the next one.
    if (rendered >= target) {
      return lineStart - 1;
    }
    rendered += 1;
  }

  return source.length;
}

type LineWalk = {
  /** The offset within the line, or null when the target lies past its end. */
  hit: number | null;
  /** How many rendered characters the line carries, once `hit` is null. */
  rendered: number;
};

/** Where `target` rendered characters land within the single line `line`. */
function walkLine(line: string, target: number): LineWalk {
  let index = linePrefix(line).length;
  let rendered = 0;

  while (index < line.length) {
    const rest = line.slice(index);

    // Emphasis, bold, and inline code carry no rendered characters, and are
    // consumed before the position check so a caret that lands exactly on a
    // marker steps past it rather than in front of it.
    const marker = /^(\*\*|__|\*|_|`)/.exec(rest);
    if (marker) {
      index += marker[0].length;
      continue;
    }

    // An embed renders nothing inline; a wikilink renders its alias when it
    // has one and its target otherwise. Both are the dominant syntax in an
    // Obsidian vault, so leaving them out put the caret inside the brackets.
    const embed = /^!\[\[[^\]\r\n]*\]\]/.exec(rest);
    if (embed) {
      index += embed[0].length;
      continue;
    }

    const wikilink = /^\[\[([^\]\r\n]*)\]\]/.exec(rest);
    if (wikilink) {
      const body = wikilink[1];
      const pipe = body.indexOf("|");
      const shown = pipe >= 0 ? body.slice(pipe + 1) : body;
      const shownStart = pipe >= 0 ? 2 + pipe + 1 : 2;
      if (target - rendered < shown.length) {
        return { hit: index + shownStart + (target - rendered), rendered };
      }
      rendered += shown.length;
      index += wikilink[0].length;
      continue;
    }

    // A link renders its label and drops its target.
    const link = /^!?\[([^\]]*)\]\([^)]*\)/.exec(rest);

    if (rendered >= target) {
      return { hit: link ? index + 1 : index, rendered };
    }

    if (link) {
      const label = link[1];
      if (target - rendered < label.length) {
        return { hit: index + 1 + (target - rendered), rendered };
      }
      rendered += label.length;
      index += link[0].length;
      continue;
    }

    rendered += 1;
    index += 1;
  }

  return { hit: null, rendered };
}

/**
 * The source offset in a fenced block for `target` characters of code.
 *
 * The fences carry the delimiter and the language name and render nothing at
 * all, so each is skipped whole, its newline with it. What is left is the code,
 * which renders literally: its own backticks and brackets are not markdown and
 * none of the line rules apply to it.
 *
 * The one thing stripped from a code line is the block's own indentation, up to
 * the width the opening fence sits at. That is how the fence is written inside
 * a list item or a callout, and those spaces belong to the container rather
 * than to the code, so they never reach the screen.
 */
function codeBlockOffset(lines: string[], target: number): number {
  const closing =
    lines.length > 1 && FENCE.test(lines[lines.length - 1])
      ? lines.length - 1
      : lines.length;
  const container = indentWidth(lines[0]);

  let start = lines[0].length + 1;
  // With no code at all, the spot between the two fences.
  let end = start;
  let rendered = 0;
  for (let i = 1; i < closing; i += 1) {
    const line = lines[i];
    const stripped = Math.min(container, indentWidth(line));
    const shown = line.length - stripped;
    if (target - rendered <= shown) {
      return start + stripped + (target - rendered);
    }
    rendered += shown + 1;
    end = start + line.length;
    start += line.length + 1;
  }

  // Past the last character of the code: the end of its final line.
  return end;
}

/** How many characters of leading indentation `line` opens with. */
function indentWidth(line: string): number {
  return /^[ \t]*/.exec(line)?.[0].length ?? 0;
}
