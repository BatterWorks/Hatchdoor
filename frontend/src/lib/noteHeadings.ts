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

export function slugifyHeading(input: string): string {
  const compact = input
    .trim()
    .toLowerCase()
    .replace(/[`*_~]/g, "")
    .replace(/\[(.*?)\]\((.*?)\)/g, "$1")
    .replace(/[^a-z0-9\s-]/g, "")
    .replace(/\s+/g, "-")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");

  return compact || "section";
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
