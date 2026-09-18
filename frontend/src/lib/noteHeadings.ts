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
    const text = normalizeHeadingText(match[2]);
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
