import {
  cloneElement,
  isValidElement,
  useEffect,
  useRef,
  type MouseEvent,
  type PointerEvent,
  type ReactNode,
} from "react";

import { blockRange, type LineRange } from "../../lib/sourceMap";
import { BlockInput, type UnitType } from "./BlockInput";
import { useInlineEditor } from "./inlineEditorContext";

function sameRange(a: LineRange | null, b: LineRange): boolean {
  return a !== null && a.startLine === b.startLine && a.endLine === b.endLine;
}

/** How long a touch must be held before it counts as "edit this", not "read". */
const LONG_PRESS_MS = 500;
/** Past this much movement the gesture is a scroll, not a press. */
const LONG_PRESS_SLOP_PX = 10;

/**
 * Makes one rendered block editable in place.
 *
 * The rendered element is cloned rather than wrapped. A wrapper would sit
 * between a list and its items, breaking every `ul > li` rule (the bullet
 * markers) and putting a div directly inside a ul, and it would do the same to
 * table rows. Cloning keeps the tree exactly as the renderer built it and only
 * adds a class and a handler.
 *
 * Degrades to the untouched child whenever the block owns no lines: outside a
 * provider, in a read-only vault, or when the node carries no position at all
 * (display math, raw HTML, link reference definitions, generated footnotes).
 */
export function EditableBlock({
  node,
  unitType,
  children,
}: {
  node: unknown;
  unitType: UnitType;
  children: ReactNode;
}) {
  const editor = useInlineEditor();
  const range = editor ? blockRange(node, editor.frontmatterOffset) : null;
  const timerRef = useRef<number | null>(null);
  const originRef = useRef<{ x: number; y: number } | null>(null);
  const touchRef = useRef(false);

  const clearPress = () => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    originRef.current = null;
  };

  useEffect(
    () => () => {
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
      }
    },
    [],
  );

  if (!editor || !editor.writeEnabled || !range || !isValidElement(children)) {
    return <>{children}</>;
  }

  const child = children as React.ReactElement<{
    className?: string;
    onClick?: (event: MouseEvent) => void;
    onPointerDown?: (event: PointerEvent) => void;
    onPointerMove?: (event: PointerEvent) => void;
    onPointerUp?: () => void;
    onPointerCancel?: () => void;
    children?: ReactNode;
  }>;

  if (sameRange(editor.activeRange, range)) {
    return cloneElement(
      child,
      {
        className: joinClass(child.props.className, "editable-block is-active"),
      },
      <BlockInput
        unitType={unitType}
        initialValue={editor.sourceOf(range)}
        onCommit={(text) => editor.commitBlock(range, text)}
        onCancel={editor.exitBlock}
      />,
    );
  }

  const claimsGesture = (target: EventTarget | null) => {
    // Links, checkboxes, and callout summaries keep their own behaviour.
    const el = target as HTMLElement | null;
    return !!el?.closest?.("a, input, summary, button");
  };

  const onPointerDown = (event: PointerEvent) => {
    touchRef.current =
      event.pointerType === "touch" || event.pointerType === "pen";
    clearPress();
    if (!touchRef.current || claimsGesture(event.target)) {
      return;
    }
    // On a phone, reading is the dominant mode: tap-to-enter would raise the
    // keyboard on every stray touch. Entry is a deliberate hold.
    originRef.current = { x: event.clientX, y: event.clientY };
    timerRef.current = window.setTimeout(() => {
      timerRef.current = null;
      editor.enterBlock(range);
    }, LONG_PRESS_MS);
  };

  const onPointerMove = (event: PointerEvent) => {
    const origin = originRef.current;
    if (timerRef.current === null || !origin) {
      return;
    }
    const moved =
      Math.abs(event.clientX - origin.x) > LONG_PRESS_SLOP_PX ||
      Math.abs(event.clientY - origin.y) > LONG_PRESS_SLOP_PX;
    if (moved) {
      clearPress();
    }
  };

  const onClick = (event: MouseEvent) => {
    if (event.defaultPrevented) {
      return;
    }
    // A touch gesture already decided for itself whether to enter, so the
    // synthetic click it produces must not enter a second time.
    if (touchRef.current) {
      return;
    }
    // The innermost block wins, so an ancestor never claims a click a
    // descendant already handled.
    if (claimsGesture(event.target)) {
      return;
    }
    event.preventDefault();
    editor.enterBlock(range);
  };

  return cloneElement(child, {
    className: joinClass(child.props.className, "editable-block"),
    onClick,
    onPointerDown,
    onPointerMove,
    onPointerUp: clearPress,
    onPointerCancel: clearPress,
  });
}

function joinClass(existing: string | undefined, added: string): string {
  return existing ? `${existing} ${added}` : added;
}
