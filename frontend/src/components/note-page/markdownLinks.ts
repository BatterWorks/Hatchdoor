/**
 * Markdown-form note links (ADR-28): `[t](p.md)`, `[t](<p.md>)` and the
 * reference definition `[r]: p.md`.
 *
 * `src/vault/markdown_links.rs` carries the same recognition for the link
 * graph and the rename rewriters. The two have to change together, or a link
 * the renderer navigates stops counting as a link, or the other way round.
 */

/** One note link's destination, located in the scanned text. */
export type MarkdownNoteLink = {
  /** Where the destination starts and ends, `<` and `>` excluded. */
  start: number;
  end: number;
  angle: boolean;
  /** The destination's path part as written, before any `#anchor`. */
  rawPath: string;
  /** The anchor as written, without its `#`; empty when there is none. */
  anchor: string;
};

/**
 * Every Markdown note link in `markdown` whose destination can be rewritten:
 * inline links and reference definitions (a reference use takes its
 * destination from the definition, so rewriting the definition moves every
 * use). Fenced code, inline code, images and wikilinks are skipped.
 */
export function findMarkdownNoteLinks(markdown: string): MarkdownNoteLink[] {
  return scanMarkdown(markdown).links;
}

/** One inline Markdown image's destination, located in the scanned text. */
export type MarkdownImage = {
  /** Where the destination starts and ends, `<` and `>` excluded. */
  start: number;
  end: number;
  angle: boolean;
  /** The destination as written. */
  raw: string;
};

/**
 * Every inline Markdown image `![alt](destination)` outside code, so its
 * destination can be pointed at the file the server resolves it to.
 */
export function findMarkdownImages(markdown: string): MarkdownImage[] {
  return scanMarkdown(markdown).images;
}

function scanMarkdown(markdown: string): {
  links: MarkdownNoteLink[];
  images: MarkdownImage[];
} {
  const links: MarkdownNoteLink[] = [];
  const images: MarkdownImage[] = [];
  let fence: { marker: string; length: number } | null = null;
  let lineStart = 0;
  for (const rawLine of markdown.split("\n")) {
    const offset = lineStart;
    lineStart += rawLine.length + 1;
    const line = rawLine.endsWith("\r") ? rawLine.slice(0, -1) : rawLine;
    const marker = fenceMarker(line.trimStart());
    if (fence) {
      if (
        marker &&
        marker.marker === fence.marker &&
        marker.length >= fence.length
      ) {
        fence = null;
      }
      continue;
    }
    if (marker) {
      fence = marker;
      continue;
    }
    const definition = parseDefinition(line);
    if (definition) {
      pushNoteLink(links, line, offset, definition);
      continue;
    }
    scanInline(line, offset, links, images);
  }
  return { links, images };
}

type Destination = { start: number; end: number; angle: boolean };

function pushNoteLink(
  links: MarkdownNoteLink[],
  line: string,
  offset: number,
  destination: Destination,
) {
  const target = noteLinkTarget(line.slice(destination.start, destination.end));
  if (!target) {
    return;
  }
  links.push({
    start: destination.start + offset,
    end: destination.end + offset,
    angle: destination.angle,
    ...target,
  });
}

function fenceMarker(
  trimmed: string,
): { marker: string; length: number } | null {
  const marker = trimmed[0];
  if (marker !== "`" && marker !== "~") {
    return null;
  }
  let length = 1;
  while (trimmed[length] === marker) {
    length += 1;
  }
  return length >= 3 ? { marker, length } : null;
}

function scanInline(
  line: string,
  offset: number,
  links: MarkdownNoteLink[],
  images: MarkdownImage[],
) {
  let idx = 0;
  while (idx < line.length) {
    const ch = line[idx];
    if (ch === "\\") {
      idx += 2;
    } else if (ch === "`") {
      idx = skipCodeSpan(line, idx);
    } else if (ch === "[" && line[idx + 1] === "[") {
      const end = line.indexOf("]]", idx + 2);
      idx = end < 0 ? idx + 2 : end + 2;
    } else if (ch === "[") {
      const isImage = idx > 0 && line[idx - 1] === "!";
      const close = matchingBracket(line, idx);
      if (close < 0 || line[idx + 1] === "^") {
        idx = close < 0 ? idx + 1 : close + 1;
        continue;
      }
      const inline =
        line[close + 1] === "("
          ? parseInlineDestination(line, close + 1)
          : null;
      if (inline) {
        if (isImage) {
          const { start, end, angle } = inline.destination;
          images.push({
            start: start + offset,
            end: end + offset,
            angle,
            raw: line.slice(start, end),
          });
        } else {
          pushNoteLink(links, line, offset, inline.destination);
        }
        idx = inline.end;
        continue;
      }
      // A full reference `[text][label]` is one construct, as the backend
      // scanner reads it; anything else may hold a real link inside.
      const labelClose =
        line[close + 1] === "[" ? line.indexOf("]", close + 2) : -1;
      idx = labelClose < 0 ? idx + 1 : labelClose + 1;
    } else {
      idx += 1;
    }
  }
}

// A backtick run nothing closes makes the rest of the line code, as the
// backend's link reader treats it.
function skipCodeSpan(line: string, start: number): number {
  const run = backtickRun(line, start);
  let idx = start + run;
  while (idx < line.length) {
    if (line[idx] === "`") {
      const close = backtickRun(line, idx);
      if (close === run) {
        return idx + close;
      }
      idx += close;
    } else {
      idx += 1;
    }
  }
  return line.length;
}

function backtickRun(line: string, start: number): number {
  let length = 0;
  while (line[start + length] === "`") {
    length += 1;
  }
  return length;
}

function matchingBracket(line: string, open: number): number {
  let depth = 0;
  let idx = open;
  while (idx < line.length) {
    const ch = line[idx];
    if (ch === "\\") {
      idx += 2;
      continue;
    }
    if (ch === "`") {
      idx = skipCodeSpan(line, idx);
      continue;
    }
    if (ch === "[") {
      depth += 1;
    } else if (ch === "]") {
      depth -= 1;
      if (depth === 0) {
        return idx;
      }
    }
    idx += 1;
  }
  return -1;
}

function parseInlineDestination(
  line: string,
  open: number,
): { destination: Destination; end: number } | null {
  const parsed = parseDestination(line, skipSpaces(line, open + 1), true);
  if (!parsed) {
    return null;
  }
  let idx = skipSpaces(line, parsed.after);
  if (idx > parsed.after) {
    const afterTitle = skipTitle(line, idx);
    if (afterTitle < 0) {
      return null;
    }
    idx = skipSpaces(line, afterTitle);
  }
  return line[idx] === ")"
    ? { destination: parsed.destination, end: idx + 1 }
    : null;
}

function parseDestination(
  line: string,
  start: number,
  inline: boolean,
): { destination: Destination; after: number } | null {
  if (line[start] === "<") {
    let idx = start + 1;
    while (idx < line.length) {
      const ch = line[idx];
      if (ch === "\\") {
        idx += 2;
      } else if (ch === ">") {
        return {
          destination: { start: start + 1, end: idx, angle: true },
          after: idx + 1,
        };
      } else if (ch === "<") {
        return null;
      } else {
        idx += 1;
      }
    }
    return null;
  }
  let idx = start;
  let depth = 0;
  while (idx < line.length) {
    const ch = line[idx];
    const code = ch.charCodeAt(0);
    if (ch === "\\" && idx + 1 < line.length) {
      idx += 2;
    } else if (code <= 0x20 || code === 0x7f) {
      break;
    } else if (ch === "(") {
      depth += 1;
      idx += 1;
    } else if (ch === ")" && inline && depth === 0) {
      break;
    } else if (ch === ")") {
      depth = Math.max(0, depth - 1);
      idx += 1;
    } else {
      idx += 1;
    }
  }
  return { destination: { start, end: idx, angle: false }, after: idx };
}

function skipTitle(line: string, start: number): number {
  const close = { '"': '"', "'": "'", "(": ")" }[line[start] ?? ""];
  if (!close) {
    return start;
  }
  let idx = start + 1;
  while (idx < line.length) {
    if (line[idx] === "\\") {
      idx += 2;
    } else if (line[idx] === close) {
      return idx + 1;
    } else {
      idx += 1;
    }
  }
  return -1;
}

function skipSpaces(line: string, start: number): number {
  let idx = start;
  while (line[idx] === " " || line[idx] === "\t") {
    idx += 1;
  }
  return idx;
}

function parseDefinition(line: string): Destination | null {
  let indent = 0;
  while (line[indent] === " ") {
    indent += 1;
  }
  if (indent > 3 || line[indent] !== "[" || line[indent + 1] === "^") {
    return null;
  }
  let idx = indent + 1;
  while (idx < line.length && line[idx] !== "]") {
    if (line[idx] === "\\") {
      idx += 2;
      continue;
    }
    if (line[idx] === "[") {
      return null;
    }
    idx += 1;
  }
  const label = line.slice(indent + 1, idx);
  if (!label.trim() || line[idx + 1] !== ":") {
    return null;
  }
  const parsed = parseDestination(line, skipSpaces(line, idx + 2), false);
  if (!parsed) {
    return null;
  }
  const { destination, after } = parsed;
  if (destination.start === destination.end && !destination.angle) {
    return null;
  }
  let end = skipSpaces(line, after);
  if (end > after) {
    const afterTitle = skipTitle(line, end);
    if (afterTitle < 0) {
      return null;
    }
    end = skipSpaces(line, afterTitle);
  }
  return end === line.length ? destination : null;
}

/**
 * Whether `raw`, a destination as written, names a note: a local path ending
 * `.md` once any `#anchor` is removed. Anything with a URL scheme, or
 * protocol-relative, is external whatever it ends in.
 */
export function noteLinkTarget(
  raw: string,
): { rawPath: string; anchor: string } | null {
  const hash = unescapedHash(raw);
  const rawPath = hash < 0 ? raw : raw.slice(0, hash);
  const anchor = hash < 0 ? "" : raw.slice(hash + 1);
  if (rawPath.startsWith("//") || /^[A-Za-z][A-Za-z0-9+.-]*:/.test(rawPath)) {
    return null;
  }
  const path = decodePercent(rawPath.replace(/\\([!-/:-@[-`{-~])/g, "$1"));
  const fileName = path.split(/[/\\]/).pop() ?? path;
  if (!fileName.endsWith(".md") || fileName.length <= ".md".length) {
    return null;
  }
  return { rawPath, anchor };
}

function unescapedHash(raw: string): number {
  let idx = 0;
  while (idx < raw.length) {
    if (raw[idx] === "\\") {
      idx += 2;
    } else if (raw[idx] === "#") {
      return idx;
    } else {
      idx += 1;
    }
  }
  return -1;
}

/**
 * Decode `%XX` escapes. A `%` not followed by two hex digits is a literal
 * percent sign, and input that does not decode to text is returned as is.
 */
export function decodePercent(raw: string): string {
  try {
    return decodeURIComponent(raw.replace(/%(?![0-9A-Fa-f]{2})/g, "%25"));
  } catch {
    return raw;
  }
}
