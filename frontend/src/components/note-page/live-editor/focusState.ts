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
} from "@codemirror/state";
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
 * change, for a resolver that has since learnt more.
 */
export function caretAwareField<T>(spec: {
  find: (state: EditorState) => T;
  decorate: (state: EditorState, found: T, focused: boolean) => DecorationSet;
  refindOn?: StateEffectType<void>;
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
          tr.effects.some((effect) => effect.is(spec.refindOn!)));
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
