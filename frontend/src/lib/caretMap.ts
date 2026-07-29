// Maps a caret position in rendered text back to an offset in the markdown
// source that produced it.
//
// The mapping is approximate by nature: markdown syntax characters do not exist
// in the rendered text, so there is no exact answer. Landing a few characters
// off is acceptable; always landing at offset 0, or always at the end, is not.

/** Line prefixes that produce no rendered text at all. */
const LINE_PREFIX =
  /^(\s*)(?:(?:[-*+]|\d+[.)])\s+(?:\[[ xX]\]\s+)?|#{1,6}\s+|>\s?)/;

/**
 * The source offset corresponding to `renderedOffset` characters of rendered
 * text in the single source line `source`.
 */
export function sourceOffsetForRenderedOffset(
  source: string,
  renderedOffset: number,
): number {
  const target = Math.max(0, renderedOffset);

  let index = 0;
  const prefix = LINE_PREFIX.exec(source);
  if (prefix) {
    index = prefix[0].length;
  }

  let rendered = 0;
  while (index < source.length) {
    const rest = source.slice(index);

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
        return index + shownStart + (target - rendered);
      }
      rendered += shown.length;
      index += wikilink[0].length;
      continue;
    }

    // A link renders its label and drops its target.
    const link = /^!?\[([^\]]*)\]\([^)]*\)/.exec(rest);

    if (rendered >= target) {
      return link ? index + 1 : index;
    }

    if (link) {
      const label = link[1];
      if (target - rendered < label.length) {
        return index + 1 + (target - rendered);
      }
      rendered += label.length;
      index += link[0].length;
      continue;
    }

    rendered += 1;
    index += 1;
  }

  return source.length;
}
