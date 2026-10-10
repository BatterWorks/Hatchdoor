// Marks the words of a `?q=` search query inside the editor, the way the
// rendered page marks them with `mark.search-hit`, so arriving from search
// lands on the same highlights whichever view the note opens in.

import { Compartment, type Extension } from "@codemirror/state";
import {
  Decoration,
  EditorView,
  MatchDecorator,
  ViewPlugin,
} from "@codemirror/view";

export type SearchHit = { from: number; to: number };

function queryRegExp(query: string): RegExp | null {
  const terms = query
    .split(/\s+/)
    .map((term) => term.trim())
    .filter((term) => term.length > 0)
    .map((term) => term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  return terms.length > 0 ? new RegExp(terms.join("|"), "giu") : null;
}

/** Every hit of `query` in `doc`, in document order. */
export function findSearchHits(doc: string, query: string): SearchHit[] {
  const regexp = queryRegExp(query);
  if (!regexp) {
    return [];
  }
  const hits: SearchHit[] = [];
  for (const match of doc.matchAll(regexp)) {
    if (match[0].length > 0) {
      hits.push({ from: match.index, to: match.index + match[0].length });
    }
  }
  return hits;
}

const hitMark = Decoration.mark({ class: "search-hit" });

function highlightPlugin(query: string): Extension {
  const regexp = queryRegExp(query);
  if (!regexp) {
    return [];
  }
  const decorator = new MatchDecorator({
    regexp,
    decoration: () => hitMark,
  });
  return ViewPlugin.define(
    (view) => ({
      decorations: decorator.createDeco(view),
      update(update) {
        this.decorations = decorator.updateDeco(update, this.decorations);
      },
    }),
    { decorations: (plugin) => plugin.decorations },
  );
}

/** The highlight lives in a compartment so a new query swaps it in place. */
export function searchHighlight(): {
  extension: Extension;
  reconfigure: (view: EditorView, query: string) => void;
} {
  const compartment = new Compartment();
  return {
    extension: compartment.of([]),
    reconfigure(view, query) {
      view.dispatch({
        effects: compartment.reconfigure(highlightPlugin(query)),
      });
    },
  };
}
