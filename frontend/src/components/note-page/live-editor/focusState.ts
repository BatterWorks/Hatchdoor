// Whether the editor has focus, as state the decoration fields can read.
//
// The inline preview reveals a line's syntax only while the editor is
// focused: leaving it renders everything, caret line included. The rendered
// blocks and callouts follow the same rule, and a state field cannot ask the
// view, so focus changes arrive as an effect on the transaction that
// records them.

import {
  StateEffect,
  StateField,
  type EditorState,
  type Extension,
  type StateEffectType,
  type Transaction,
} from "@codemirror/state";
import { ensureSyntaxTree, syntaxTree } from "@codemirror/language";
import type { Tree } from "@lezer/common";
import { EditorView, type DecorationSet } from "@codemirror/view";

export const focusChanged = StateEffect.define<boolean>();

export const focusField = StateField.define<boolean>({
  create: () => false,
  update: (value, tr) => {
    for (const effect of tr.effects) {
      if (effect.is(focusChanged)) {
        return effect.value;
      }
    }
    return value;
  },
});

export const focusTracking: Extension = [
  focusField,
  EditorView.focusChangeEffect.of((_state, focusing) =>
    focusChanged.of(focusing),
  ),
];

const treesByState = new WeakMap<EditorState, Tree>();

/**
 * The note's syntax tree for the layers that read it: the whole tree when it
 * can be parsed within 50 ms, otherwise what is parsed so far (#783). A long
 * note is not fully parsed when it opens, so a layer that reads this must
 * also look again as the parser goes on (`refindOnParse`, `treeGrew`). The
 * answer is kept per state, so the layers share one 50 ms budget per
 * transaction rather than each spending its own.
 */
export function treeSoFar(state: EditorState): Tree {
  let tree = treesByState.get(state);
  if (tree === undefined) {
    tree = ensureSyntaxTree(state, state.doc.length, 50) ?? syntaxTree(state);
    treesByState.set(state, tree);
  }
  return tree;
}

/** Whether the background parser extended the tree in this transaction. */
export function treeGrew(tr: Transaction): boolean {
  return syntaxTree(tr.state) !== syntaxTree(tr.startState);
}

/** Whether a selection range sits on `[from, to]`, boundaries included. */
export function caretTouches(
  state: EditorState,
  from: number,
  to: number,
): boolean {
  return state.selection.ranges.some(
    (range) => range.from <= to && range.to >= from,
  );
}

/**
 * A decoration field that finds its subjects on each document change and
 * redraws them on each caret move and focus change, so that what the caret
 * sits on can show its source while the editor is focused. `refindOn` names
 * an effect after which the subjects are found again without a document
 * change, for a resolver that has since learnt more. `refindOnParse` finds
 * them again each time the background parser extends the syntax tree, for a
 * `find` that reads the tree: a long note is not fully parsed when it opens.
 */
export function caretAwareField<T>(spec: {
  find: (state: EditorState) => T;
  decorate: (state: EditorState, found: T, focused: boolean) => DecorationSet;
  refindOn?: StateEffectType<void>;
  refindOnParse?: boolean;
}): Extension {
  type Value = { found: T; decorations: DecorationSet };
  return StateField.define<Value>({
    create: (state) => {
      const found = spec.find(state);
      return { found, decorations: spec.decorate(state, found, false) };
    },
    update: (value, tr) => {
      const focusMoved = tr.effects.some((effect) => effect.is(focusChanged));
      const refind =
        tr.docChanged ||
        (spec.refindOn !== undefined &&
          tr.effects.some((effect) => effect.is(spec.refindOn!))) ||
        (spec.refindOnParse === true && treeGrew(tr));
      if (!refind && !tr.selection && !focusMoved) {
        return value;
      }
      const found = refind ? spec.find(tr.state) : value.found;
      return {
        found,
        decorations: spec.decorate(tr.state, found, tr.state.field(focusField)),
      };
    },
    provide: (field) =>
      EditorView.decorations.from(field, (value) => value.decorations),
  });
}
