// The formatting bar above a phone's keyboard. Touch gets no floating toolbar
// (#540): the OS draws its own Cut/Copy/Paste callout over a selection, in
// the same place, and the two fight. Everything the desktop toolbar offers
// lives here instead, as in Obsidian mobile.
//
// iOS lays the keyboard over the page and only shrinks the visual viewport,
// so a fixed bar at bottom: 0 ends up under the keyboard. The bar is lifted by
// however much of the layout viewport the keyboard hides; on Android, where
// the layout viewport itself shrinks, that amount is zero.

import { useEffect, useState, type MouseEvent } from "react";

import { redo, undo } from "@codemirror/commands";
import type { EditorView } from "@codemirror/view";

import {
  ChecklistIcon,
  CodeIcon,
  FormatBoldIcon,
  FormatH2Icon,
  FormatIndentDecreaseIcon,
  FormatIndentIncreaseIcon,
  FormatInkHighlighterIcon,
  FormatItalicIcon,
  FormatListBulletedIcon,
  FormatStrikethroughIcon,
  LinkIcon,
  RedoIcon,
  UndoIcon,
} from "../../icons";
import {
  indentLines,
  insertLink,
  setLinePrefix,
  wrapSelection,
} from "./commands";

function useKeyboardOffset(): number {
  const [offset, setOffset] = useState(0);
  useEffect(() => {
    const viewport = window.visualViewport;
    if (!viewport) {
      return;
    }
    const update = () => {
      const hidden =
        window.innerHeight - (viewport.height + viewport.offsetTop);
      setOffset(Math.max(0, Math.round(hidden)));
    };
    update();
    viewport.addEventListener("resize", update);
    viewport.addEventListener("scroll", update);
    return () => {
      viewport.removeEventListener("resize", update);
      viewport.removeEventListener("scroll", update);
    };
  }, []);
  return offset;
}

type Item = {
  label: React.ReactNode;
  title: string;
  run: (view: EditorView) => boolean | void;
};

const ITEMS: Array<Item | "gap"> = [
  {
    label: <FormatIndentDecreaseIcon />,
    title: "Outdent",
    run: (v) => indentLines(v, false),
  },
  {
    label: <FormatIndentIncreaseIcon />,
    title: "Indent",
    run: (v) => indentLines(v, true),
  },
  {
    label: <FormatListBulletedIcon />,
    title: "Bullet",
    run: (v) => setLinePrefix(v, "- "),
  },
  {
    label: <ChecklistIcon />,
    title: "To-do",
    run: (v) => setLinePrefix(v, "- [ ] "),
  },
  {
    label: <FormatH2Icon />,
    title: "Heading",
    run: (v) => setLinePrefix(v, "## "),
  },
  "gap",
  {
    label: <FormatBoldIcon />,
    title: "Bold",
    run: (v) => wrapSelection(v, "**"),
  },
  {
    label: <FormatItalicIcon />,
    title: "Italic",
    run: (v) => wrapSelection(v, "*"),
  },
  {
    label: <FormatStrikethroughIcon />,
    title: "Strikethrough",
    run: (v) => wrapSelection(v, "~~"),
  },
  { label: <CodeIcon />, title: "Code", run: (v) => wrapSelection(v, "`") },
  {
    label: <FormatInkHighlighterIcon />,
    title: "Highlight",
    run: (v) => wrapSelection(v, "=="),
  },
  { label: <LinkIcon />, title: "Link", run: insertLink },
  "gap",
  { label: <UndoIcon />, title: "Undo", run: undo },
  { label: <RedoIcon />, title: "Redo", run: redo },
];

export function KeyboardBar({ getView }: { getView: () => EditorView | null }) {
  const offset = useKeyboardOffset();
  const press =
    (run: (view: EditorView) => boolean | void) =>
    (event: MouseEvent<HTMLButtonElement>) => {
      // mousedown, and prevented: a click would first move focus off the
      // editor, which drops the keyboard and commits the edit.
      event.preventDefault();
      const view = getView();
      if (view) {
        run(view);
      }
    };
  return (
    <div
      className="live-editor-keyboard-bar"
      style={{ bottom: offset }}
      role="toolbar"
      aria-label="Formatting"
    >
      {ITEMS.map((item, index) =>
        item === "gap" ? (
          <span key={`gap-${index}`} className="live-editor-keyboard-gap" />
        ) : (
          <button
            key={item.title}
            type="button"
            title={item.title}
            aria-label={item.title}
            onMouseDown={press(item.run)}
          >
            {item.label}
          </button>
        ),
      )}
      <button
        type="button"
        className="live-editor-keyboard-done"
        onMouseDown={press((view) => view.contentDOM.blur())}
      >
        Done
      </button>
    </div>
  );
}
