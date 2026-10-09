// The editor's wikilink extension asks for one target at a time, as links
// come into view, and it asks them all in one synchronous pass. Folding that
// pass into a single request is what keeps a note with fifty links from
// sending fifty of them: every target asked before the next microtask goes
// out together, the way the reading view's batch does.

/** What the server knows about one target: `null` when nothing matches. */
export type NoteLinkHit = { slug: string; archived?: boolean };

export type NoteLinkResolver = (target: string) => Promise<NoteLinkHit | null>;

export function createNoteLinkResolver(
  resolveBatch: (targets: string[]) => Promise<Map<string, NoteLinkHit | null>>,
): NoteLinkResolver {
  let pending: Map<string, Array<(hit: NoteLinkHit | null) => void>> | null =
    null;

  const flush = async () => {
    const batch = pending;
    pending = null;
    if (!batch) {
      return;
    }
    const targets = [...batch.keys()];
    let answers: Map<string, NoteLinkHit | null>;
    try {
      answers = await resolveBatch(targets);
    } catch {
      answers = new Map();
    }
    for (const [target, waiting] of batch) {
      const hit = answers.get(target) ?? null;
      for (const settle of waiting) {
        settle(hit);
      }
    }
  };

  return (target) =>
    new Promise((settle) => {
      if (!pending) {
        pending = new Map();
        queueMicrotask(() => void flush());
      }
      const waiting = pending.get(target) ?? [];
      waiting.push(settle);
      pending.set(target, waiting);
    });
}

/**
 * The text a wikilink shows for its target when the link carries no alias,
 * as the reading view shows it: the target without its heading and without
 * its folders, and the path kept whole for an archived note.
 */
export function wikilinkLabel(target: string, archived: boolean): string {
  const base = target.split(/[#^]/, 1)[0].trim() || target;
  if (archived) {
    return base;
  }
  const parts = base.replace(/\\/g, "/").split("/").filter(Boolean);
  return parts[parts.length - 1] ?? base;
}
