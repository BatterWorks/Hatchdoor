import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { linePrefix } from "../../lib/linePrefix";
import type { KeyboardEvent } from "react";

export type UnitType =
  | "paragraph"
  | "heading"
  | "list item"
  | "quote"
  | "callout"
  | "code block"
  | "table row";

/**
 * The textarea that replaces one rendered block while it is being edited.
 *
 * Styling matches the metrics of the block it replaces but never its
 * transforms: `.block-input` resets text-transform, because three rendered
 * styles in this app uppercase their text and would otherwise show the typist a
 * different string than the one being saved.
 */
/**
 * Hangs the line's markdown prefix into the gutter so the visible text does not
 * move when a block is entered.
 *
 * The prefix has to be *measured*: "- " is about 9px while the bullet marker it
 * replaces is inset 22.4px, so a fixed hang leaves the text 13px out. The
 * offset is clamped to the space the scroll pane actually has, which is 56px on
 * desktop but only 16px on a phone, so a long prefix never gets clipped.
 */
function hangPrefix(el: HTMLTextAreaElement) {
  const prefix = linePrefix(el.value);
  if (!prefix) {
    el.style.marginLeft = "";
    el.style.width = "";
    return;
  }

  const style = getComputedStyle(el);
  const width = measureText(prefix, style.font);
  if (width === null) {
    return;
  }

  const pane = el.closest("main");
  const available = pane
    ? parseFloat(getComputedStyle(pane).paddingLeft) || 0
    : 0;
  const existingInset = parseFloat(style.paddingLeft) || 0;
  const hang = Math.min(width, available + existingInset);

  el.style.marginLeft = `${-hang}px`;
  el.style.width = `calc(100% + ${hang}px)`;
}

let measureContext: CanvasRenderingContext2D | null | undefined;

function measureText(text: string, font: string): number | null {
  if (measureContext === undefined) {
    measureContext = document.createElement("canvas").getContext("2d");
  }
  if (!measureContext) {
    return null;
  }
  measureContext.font = font;
  return measureContext.measureText(text).width;
}

export function BlockInput({
  unitType,
  initialValue,
  initialCaret = null,
  onCommit,
  onSplit,
  onMergeUp,
  onIndent,
  onOutdent,
  onMove,
}: {
  unitType: UnitType;
  initialValue: string;
  initialCaret?: number | null;
  onCommit: (text: string) => void;
  onSplit?: (text: string, caret: number) => void;
  onMergeUp?: (text: string) => boolean;
  onIndent?: (text: string) => boolean;
  onOutdent?: (text: string) => boolean;
  onMove?: (text: string, direction: -1 | 1, column: number) => boolean;
}) {
  const [value, setValue] = useState(initialValue);
  const ref = useRef<HTMLTextAreaElement | null>(null);
  const committedRef = useRef(false);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) {
      return;
    }
    el.focus();
    const caret =
      initialCaret === null
        ? el.value.length
        : Math.min(Math.max(initialCaret, 0), el.value.length);
    el.setSelectionRange(caret, caret);
    // Keeps the active line above the virtual keyboard on a phone.
    el.scrollIntoView?.({ block: "nearest" });
    hangPrefix(el);
    // Deliberately mount-only: the caret is placed once on entry, and rerunning
    // this would yank the caret back mid-edit.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const el = ref.current;
    if (!el) {
      return;
    }
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [value]);

  const commit = () => {
    if (committedRef.current) {
      return;
    }
    committedRef.current = true;
    onCommit(value);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    // Every intercepted key is gated on composition: with an IME active, Enter
    // and Escape belong to the candidate window, not to us.
    if (event.nativeEvent.isComposing) {
      return;
    }
    // Escape commits and leaves. Under autosave there is nothing to cancel
    // back to, so discarding here would throw away work with no way to get it
    // back. It is a deliberate break from Escape elsewhere in the app.
    if (event.key === "Escape") {
      event.preventDefault();
      commit();
      return;
    }

    const el = event.currentTarget;

    // Enter splits a unit, except inside a code block or a table row, where a
    // generic split would insert a line before the closing fence or before the
    // delimiter row and destroy the block (D27).
    if (event.key === "Enter" && !event.shiftKey) {
      if (unitType === "code block" || unitType === "table row" || !onSplit) {
        return;
      }
      event.preventDefault();
      committedRef.current = true;
      onSplit(el.value, el.selectionStart);
      return;
    }

    if (
      event.key === "Backspace" &&
      el.selectionStart === 0 &&
      el.selectionEnd === 0
    ) {
      if (unitType === "table row" || !onMergeUp) {
        return;
      }
      if (onMergeUp(el.value)) {
        event.preventDefault();
        committedRef.current = true;
      }
      return;
    }

    // Arrow keys only leave the unit from its edge lines; motion inside the
    // block is the browser's job.
    if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      const caret = el.selectionStart;
      const upToCaret = el.value.slice(0, caret);
      const lastBreak = upToCaret.lastIndexOf("\n");
      const onFirstLine = lastBreak === -1;
      const onLastLine = el.value.indexOf("\n", caret) === -1;
      const leaving = event.key === "ArrowUp" ? onFirstLine : onLastLine;
      if (!leaving || !onMove) {
        return;
      }
      const column = caret - (lastBreak + 1);
      if (onMove(el.value, event.key === "ArrowUp" ? -1 : 1, column)) {
        event.preventDefault();
        committedRef.current = true;
      }
      return;
    }

    if (event.key === "Tab" && unitType === "list item") {
      const handler = event.shiftKey ? onOutdent : onIndent;
      if (handler?.(el.value)) {
        event.preventDefault();
        committedRef.current = true;
      }
    }
  };

  return (
    <textarea
      ref={ref}
      className={`block-input block-input-${unitType.replace(/\s+/g, "-")}`}
      aria-label={`Editing ${unitType}`}
      value={value}
      rows={1}
      spellCheck
      onChange={(event) => setValue(event.target.value)}
      onBlur={commit}
      onKeyDown={onKeyDown}
    />
  );
}
