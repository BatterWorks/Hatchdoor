import { stripHighlights } from "./markdownSyntax";

export type NoteHeading = {
  level: number;
  text: string;
  id: string;
  sourceLine: number;
};

export function extractMarkdownHeadings(markdown: string): NoteHeading[] {
  const lines = markdown.split(/\r?\n/);
  const counts = new Map<string, number>();
  const headings: NoteHeading[] = [];
  let fenced = false;

  for (const [lineIndex, line] of lines.entries()) {
    if (/^\s*(```|~~~)/.test(line)) {
      fenced = !fenced;
      continue;
    }

    if (fenced) {
      continue;
    }

    const match = line.match(/^(#{1,6})\s+(.+?)\s*#*\s*$/);
    if (!match) {
      continue;
    }

    const level = match[1].length;
    // A heading that is nothing but highlight signs and a colour emoji keeps
    // them, so it stays in the outline with the anchor it had.
    const text =
      normalizeHeadingText(stripHighlights(match[2])) ||
      normalizeHeadingText(match[2]);
    if (!text) {
      continue;
    }

    const id = assignHeadingId(text, counts);
    headings.push({ level, text, id, sourceLine: lineIndex + 1 });
  }

  return headings;
}

export function assignHeadingId(
  text: string,
  counts: Map<string, number>,
): string {
  const base = slugifyHeading(text);
  const nextCount = (counts.get(base) ?? 0) + 1;
  counts.set(base, nextCount);
  return nextCount === 1 ? base : `${base}-${nextCount}`;
}

/**
 * The European letters that no decomposition reaches, each with the spelling
 * its own language uses when it has to write ASCII (ADR-24). Keep this table
 * and `fold_european_letter` in `src/vault/paths.rs` identical: they are the
 * same rule, and a heading anchor that disagrees with its note's slug is a
 * link that lands nowhere.
 */
const EUROPEAN_FOLDINGS = new Map<string, string>([
  ["ß", "ss"],
  ["ẞ", "ss"],
  ["ø", "o"],
  ["Ø", "o"],
  ["æ", "ae"],
  ["Æ", "ae"],
  ["œ", "oe"],
  ["Œ", "oe"],
  ["ł", "l"],
  ["Ł", "l"],
  ["đ", "d"],
  ["Đ", "d"],
  ["ð", "d"],
  ["Ð", "d"],
  ["þ", "th"],
  ["Þ", "th"],
]);

const ASCII_ALPHANUMERIC = /[a-z0-9]/i;
const LETTER_OR_DIGIT = /[\p{Alphabetic}\p{N}]/u;
const COMBINING_MARK = /\p{M}/u;

/**
 * Fold a heading into the anchor the browser scrolls to.
 *
 * European languages give a plain ASCII anchor and every other script keeps
 * its own letters (ADR-24), which is the rule `slugify` in
 * `src/vault/paths.rs` applies to note names. An accented Latin letter loses
 * its marks and folds to its base letter, a letter or digit from any other
 * writing system is kept as it is, and so is a mark that belongs to one.
 * Nothing is romanised. Punctuation, symbols and the marks that sit on ASCII
 * are dropped.
 *
 * The emphasis and link stripping above the loop is this path's own: a
 * heading is Markdown, a note name is not.
 */
export function slugifyHeading(input: string): string {
  const text = input
    .trim()
    .toLowerCase()
    .replace(/[`*_~]/g, "")
    .replace(/\[(.*?)\]\((.*?)\)/g, "$1");

  let out = "";
  let prevDash = false;

  for (const char of text) {
    const folded = EUROPEAN_FOLDINGS.get(char);
    if (folded !== undefined) {
      out += folded;
      prevDash = false;
      continue;
    }

    if (/\s/.test(char) || char === "-") {
      if (!prevDash && out.length > 0) {
        out += "-";
      }
      prevDash = true;
      continue;
    }

    // A mark that arrived on its own belongs to whatever it follows. On an
    // ASCII letter it is an accent this rule exists to remove; on a letter
    // kept in its own script it is part of the word, and dropping it rewrites
    // that word.
    if (COMBINING_MARK.test(char)) {
      const last = out.at(-1);
      if (last !== undefined && last.charCodeAt(0) > 127) {
        out += char;
        prevDash = false;
      }
      continue;
    }

    // The first character of the decomposition is the base letter and the
    // marks that follow it are what an accent is made of. A base that is
    // already ASCII is the whole answer; anything else keeps the character it
    // arrived as, so Devanagari and Hangul are not taken apart by a rule
    // written for European accents.
    const base = [...char.normalize("NFD")][0] ?? char;
    if (ASCII_ALPHANUMERIC.test(base)) {
      out += base.toLowerCase();
      prevDash = false;
      continue;
    }

    if (LETTER_OR_DIGIT.test(char)) {
      out += char.toLowerCase();
      prevDash = false;
    }
  }

  while (out.endsWith("-")) {
    out = out.slice(0, -1);
  }

  return out || "section";
}

/**
 * The anchor of the block that carries `^id` (#578). The caret stays in the
 * anchor, and `slugifyHeading` never emits one, so a block `^notes` and a
 * heading "Notes" in the same note are two places. A block id holds letters,
 * digits and hyphens only, so anything else a link wrote after the caret is
 * dropped: the anchor goes into a Markdown destination, which a space or a
 * bracket would end.
 */
export function blockAnchorId(id: string): string {
  return `^${id.replace(/[^a-zA-Z0-9-]/g, "")}`;
}

/**
 * The anchor a link's fragment names: the part after `#` in `[[Note#Heading]]`
 * or `[text](Note.md#^id)`. A fragment that opens with a caret names a block,
 * any other a heading.
 */
export function fragmentAnchor(fragment: string): string {
  const text = fragment.trim();
  return text.startsWith("^")
    ? blockAnchorId(text.slice(1))
    : slugifyHeading(text);
}

/** The heading as the outline shows it: links shown by their labels and
 * emphasis marks dropped. Highlight signs go before this, while code spans
 * still mark where `==` is literal (#782); the anchor slugs this text, and
 * `slugifyHeading` drops `=` and emoji itself, so that moves no anchor. */
function normalizeHeadingText(value: string): string {
  return value
    .replace(/\[\[([^[\]]+)\]\]/g, (_whole, body: string) =>
      extractWikilinkLabel(body),
    )
    .replace(/\[(.*?)\]\((.*?)\)/g, "$1")
    .replace(/[`*_~]/g, "")
    .trim();
}

function extractWikilinkLabel(body: string): string {
  const [targetRaw, aliasRaw] = body.split("|", 2);
  const target = (targetRaw || "").trim();
  const alias = (aliasRaw || "").trim();
  return alias || target;
}

const collapseSpace = (text: string) => text.replace(/\s+/g, " ").trim();

/** The body line of a first H1 that only repeats the note's title (#530),
 * or `undefined`. Only the first heading can qualify, and only when nothing
 * but blank lines stand before it: an H1 further down is a section. */
export function duplicateTitleHeadingLine(
  body: string,
  headings: NoteHeading[],
  title: string | undefined,
): number | undefined {
  const first = headings[0];
  if (!first || !title || first.level !== 1) {
    return undefined;
  }
  if (
    collapseSpace(first.text).toLowerCase() !==
    collapseSpace(title).toLowerCase()
  ) {
    return undefined;
  }
  const before = body.split(/\r?\n/).slice(0, first.sourceLine - 1);
  return before.every((line) => line.trim() === "")
    ? first.sourceLine
    : undefined;
}
