import {
  Children,
  cloneElement,
  isValidElement,
  useEffect,
  useRef,
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent,
  type ReactNode,
} from "react";

import { sourceOffsetForRenderedOffset } from "../../lib/caretMap";
import { blockRange, type LineRange } from "../../lib/sourceMap";
import { BlockInput, type UnitType } from "./BlockInput";
import { useInlineEditor } from "./inlineEditorContext";

function sameRange(a: LineRange | null, b: LineRange): boolean {
  return a !== null && a.startLine === b.startLine && a.endLine === b.endLine;
}

/**
 * The source offset under a pointer, or null when the browser cannot say.
 *
 * Approximate by design (D9): markdown syntax has no rendered counterpart, so
 * landing a few characters out is fine. Landing always at 0, or always at the
 * end, is not.
 */
function caretOffsetAtPoint(
  clientX: number,
  clientY: number,
  source: string,
): number | null {
  const doc = document as Document & {
    caretPositionFromPoint?: (
      x: number,
      y: number,
    ) => { offset: number } | null;
    caretRangeFromPoint?: (
      x: number,
      y: number,
    ) => { startOffset: number } | null;
  };

  const position = doc.caretPositionFromPoint?.(clientX, clientY);
  if (position) {
    return sourceOffsetForRenderedOffset(source, position.offset);
  }
  // WebKit spells it differently.
  const range = doc.caretRangeFromPoint?.(clientX, clientY);
  if (range) {
    return sourceOffsetForRenderedOffset(source, range.startOffset);
  }
  return null;
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
  const elementRef = useRef<HTMLElement | null>(null);
  const timerRef = useRef<number | null>(null);
  const originRef = useRef<{ x: number; y: number } | null>(null);
  const touchRef = useRef(false);

  const enterAt = (clientX: number, clientY: number) => {
    if (!editor || !range) {
      return;
    }
    editor.enterBlock(
      range,
      caretOffsetAtPoint(clientX, clientY, editor.sourceOf(range)),
    );
  };

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

  const registerBlock = editor?.registerBlock;
  const rangeKey = range ? `${range.startLine}:${range.endLine}` : null;
  useEffect(() => {
    if (!registerBlock || !rangeKey) {
      return;
    }
    const [startLine, endLine] = rangeKey.split(":").map(Number);
    return registerBlock({ startLine, endLine });
  }, [registerBlock, rangeKey]);

  const isActive = sameRange(
    editor?.activeRange ?? null,
    range ?? { startLine: -1, endLine: -1 },
  );
  const wasActiveRef = useRef(false);
  useEffect(() => {
    // Leaving a block must hand focus back to it, or Escape strands a keyboard
    // user with nothing focused at all.
    // Only when nothing else took over: clicking straight into another block
    // must not have this one steal focus back and blur the new input.
    if (wasActiveRef.current && !isActive && editor?.activeRange == null) {
      elementRef.current?.focus();
    }
    wasActiveRef.current = isActive;
  }, [isActive, editor?.activeRange]);

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
    onKeyDown?: (event: KeyboardEvent) => void;
    tabIndex?: number;
    ref?: unknown;
    children?: ReactNode;
  }>;

  if (isActive) {
    const input = (
      <BlockInput
        unitType={unitType}
        initialValue={editor.sourceOf(range)}
        initialCaret={editor.activeCaret}
        onCommit={(text) => editor.commitBlock(range, text)}
        onSplit={(text, caret) => {
          // The op reads the document, so the in-progress text has to be part
          // of it first, or the split would run against the stale line.
          editor.commitBlock(range, text, { keepActive: true });
          editor.splitAt(range, caret);
        }}
        onMergeUp={(text) => {
          editor.commitBlock(range, text, { keepActive: true });
          return editor.mergeUp(range);
        }}
        onIndent={(text) => {
          editor.commitBlock(range, text, { keepActive: true });
          return editor.indent(range);
        }}
        onOutdent={(text) => {
          editor.commitBlock(range, text, { keepActive: true });
          return editor.outdent(range);
        }}
      />
    );
    const activeClass = joinClass(
      child.props.className,
      "editable-block is-active",
    );
    // A tr may not contain a textarea, and swapping its cells for one would
    // let every column resize on entry and again on exit, because widths are
    // derived from content. So the cells stay exactly as they are, holding the
    // grid still, and the input is overlaid on the row from inside its first
    // cell, which is somewhere a textarea is allowed to live.
    if (child.type === "tr") {
      const cells = Children.toArray(child.props.children);
      const overlaid = cells.map((cell, index) =>
        index === 0 && isValidElement<{ children?: ReactNode }>(cell)
          ? cloneElement(
              cell,
              {},
              <>
                {cell.props.children}
                <span className="block-input-overlay">{input}</span>
              </>,
            )
          : cell,
      );
      return cloneElement(child, { className: activeClass }, overlaid);
    }

    // Same reason as below: a component child cannot receive replacement
    // children, so it gets a wrapper rather than being cloned into.
    return typeof child.type === "string" ? (
      cloneElement(child, { className: activeClass }, input)
    ) : (
      <div className={activeClass}>{input}</div>
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
    const { clientX, clientY } = event;
    timerRef.current = window.setTimeout(() => {
      timerRef.current = null;
      enterAt(clientX, clientY);
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
    enterAt(event.clientX, event.clientY);
  };

  const onKeyDown = (event: KeyboardEvent) => {
    // Composition owns Enter while an IME candidate window is open.
    if (event.nativeEvent.isComposing || event.key !== "Enter") {
      return;
    }
    if (event.target !== event.currentTarget) {
      return;
    }
    event.preventDefault();
    editor.enterBlock(range, null);
  };

  const handlers = {
    className: joinClass(child.props.className, "editable-block"),
    onClick,
    onPointerDown,
    onPointerMove,
    onPointerUp: clearPress,
    onPointerCancel: clearPress,
    onKeyDown,
    ref: elementRef,
    // Every editable unit is reachable by Tab, so there is a keyboard-only
    // path to editing rather than a mouse-only one.
    tabIndex: 0,
  };

  // Props only reach the DOM when the child is an intrinsic element. A
  // component child (the fenced code block renders its own chrome) would
  // silently swallow them, so it gets a wrapper instead. That is safe here
  // precisely because a code block is not a list item or a table row, where a
  // wrapper would sit between a parent and children it must be adjacent to.
  if (typeof child.type !== "string") {
    const { ref, ...rest } = handlers;
    return (
      <div {...rest} ref={ref as React.RefObject<HTMLDivElement | null>}>
        {child}
      </div>
    );
  }

  return cloneElement(child, handlers);
}

function joinClass(existing: string | undefined, added: string): string {
  return existing ? `${existing} ${added}` : added;
}
