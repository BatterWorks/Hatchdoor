/**
 * Hiding a note's comments from the rendered page (#573).
 *
 * Obsidian shows `%%...%%` comments only while a note is being edited, and an
 * HTML `<!-- ... -->` comment never shows. The rule for what counts as one is
 * the link preview's `strip_comments` in `src/handlers/link_preview.rs`: a
 * comment opens at `%%` or `<!--`, closes at the matching `%%` or `-->`, may
 * span lines, and one never closed runs to the end of the note.
 * `noteComments.cases.json` holds the cases both sides are tested against.
 *
 * Where this differs from the server is code and math: the page draws them,
 * so a `%%` inside inline code, a fenced block or a math span stays as
 * written. The saved query name marker, `<!-- hatchdoor-query: name -->` on a
 * line of its own, is an HTML comment too, and is left for
 * `remarkHideQueryMarkers` to read.
 *
 * This is a display rule. The text stays in the file, in search and in every
 * note read.
 */

const QUERY_MARKER_LINE = /^<!--\s*hatchdoor-query:[^\n]*-->$/;
// What can stand before a fence's opening line: blockquote markers, list
// markers and indentation. A closing line sits in the item's indentation, so
// only quote markers and indentation are stripped from it.
const OPENING_PREFIX =
  /^(?:[ \t]*(?:>|[-*+](?=[ \t])|\d{1,9}[.)](?=[ \t])))*[ \t]*/;
const CLOSING_PREFIX = /^(?:[ \t]*>)*[ \t]*/;
// The openers remark reads as a block: a backtick fence whose info string has
// no backtick, a tilde fence, and a `$$` math block with no other `$` after
// it. Anything else, `` ```npm i``` `` or `$$x$$ squared`, is an inline span.
const BLOCK_OPENERS = [/^(`{3,})[^`]*$/, /^(~{3,})/, /^(\${2,})[^$]*$/];

type CommentCloser = "%%" | "-->";

/** A fenced code block or a math block, closed by a run of the same
 * character at least as long. */
type Fence = { char: string; length: number };

/**
 * `markdown` with every comment removed. Line-count preserving, like
 * `rewriteWikilinks`: a line a comment covered whole comes back empty, so
 * every rendered block still maps to its line in the file.
 */
export function hideComments(markdown: string): string {
  let comment: CommentCloser | null = null;
  let fence: Fence | null = null;

  return markdown
    .split("\n")
    .map((rawLine) => {
      const carriage = rawLine.endsWith("\r") ? "\r" : "";
      const line = carriage ? rawLine.slice(0, -1) : rawLine;

      if (comment === null) {
        if (fence) {
          if (closesFence(line.replace(CLOSING_PREFIX, ""), fence)) {
            fence = null;
          }
          return rawLine;
        }
        if (QUERY_MARKER_LINE.test(line.trim())) {
          return rawLine;
        }
        fence = opensFence(line.replace(OPENING_PREFIX, ""));
        if (fence) {
          return rawLine;
        }
      }

      const scanned = scanLine(line, comment);
      comment = scanned.comment;
      return scanned.text + carriage;
    })
    .join("\n");
}

function opensFence(body: string): Fence | null {
  for (const opener of BLOCK_OPENERS) {
    const run = opener.exec(body.trimEnd())?.[1];
    if (run) {
      return { char: run[0], length: run.length };
    }
  }
  return null;
}

/**
 * One line without its comments, starting inside `comment` when a comment
 * opened on an earlier line, and the comment still open at its end.
 */
function scanLine(
  line: string,
  open: CommentCloser | null,
): { text: string; comment: CommentCloser | null } {
  let comment = open;
  let hadComment = open !== null;
  let text = "";
  let index = 0;

  while (index < line.length) {
    if (comment) {
      const close = line.indexOf(comment, index);
      if (close < 0) {
        break;
      }
      index = close + comment.length;
      comment = null;
      continue;
    }

    const char = line[index];
    if (char === "\\" && (line[index + 1] === "`" || line[index + 1] === "$")) {
      text += line.slice(index, index + 2);
      index += 2;
      continue;
    }
    if (char === "`" || char === "$") {
      // A code or math span, which remark opens and closes with runs of the
      // same length. Its contents are kept whole.
      const run = runLength(line, index, char);
      const close = findClosingRun(line, index + run, char, run);
      const end = close < 0 ? index + run : close + run;
      text += line.slice(index, end);
      index = end;
      continue;
    }
    if (line.startsWith("%%", index)) {
      comment = "%%";
      hadComment = true;
      index += 2;
      continue;
    }
    if (line.startsWith("<!--", index)) {
      comment = "-->";
      hadComment = true;
      index += 4;
      continue;
    }
    text += char;
    index += 1;
  }

  // Spaces left in front of a removed comment would otherwise end the line,
  // and two of them make a hard line break.
  return { text: hadComment ? text.trimEnd() : text, comment };
}

function runLength(line: string, start: number, char: string): number {
  let end = start;
  while (line[end] === char) {
    end += 1;
  }
  return end - start;
}

function findClosingRun(
  line: string,
  from: number,
  char: string,
  length: number,
): number {
  let index = from;
  while (index < line.length) {
    if (line[index] !== char) {
      index += 1;
      continue;
    }
    const run = runLength(line, index, char);
    if (run === length) {
      return index;
    }
    index += run;
  }
  return -1;
}

function closesFence(body: string, fence: Fence): boolean {
  const trimmed = body.trimEnd();
  return (
    trimmed.length >= fence.length &&
    [...trimmed].every((char) => char === fence.char)
  );
}
