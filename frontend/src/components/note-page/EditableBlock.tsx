import {
  cloneElement,
  isValidElement,
  type MouseEvent,
  type ReactNode,
} from "react";

import { blockRange, type LineRange } from "../../lib/sourceMap";
import { BlockInput, type UnitType } from "./BlockInput";
import { useInlineEditor } from "./inlineEditorContext";

function sameRange(a: LineRange | null, b: LineRange): boolean {
  return a !== null && a.startLine === b.startLine && a.endLine === b.endLine;
}

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

  if (!editor || !editor.writeEnabled || !range || !isValidElement(children)) {
    return <>{children}</>;
  }

  const child = children as React.ReactElement<{
    className?: string;
    onClick?: (event: MouseEvent) => void;
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

  const onClick = (event: MouseEvent) => {
    if (event.defaultPrevented) {
      return;
    }
    // Links, checkboxes, and callout summaries keep their own behaviour. The
    // innermost block wins, so an ancestor never claims a click a descendant
    // already handled.
    const target = event.target as HTMLElement;
    if (target.closest("a, input, summary, button")) {
      return;
    }
    event.preventDefault();
    editor.enterBlock(range);
  };

  return cloneElement(child, {
    className: joinClass(child.props.className, "editable-block"),
    onClick,
  });
}

function joinClass(existing: string | undefined, added: string): string {
  return existing ? `${existing} ${added}` : added;
}
