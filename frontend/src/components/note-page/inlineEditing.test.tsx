import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { afterEach, describe, expect, it, vi } from "vitest";

import { EditorView } from "@codemirror/view";

import { frontmatterLineOffset } from "../../lib/sourceMap";
import { InlineEditorProvider } from "./InlineEditorProvider";
import { createNoteMarkdownComponents } from "./renderers";

afterEach(() => {
  cleanup();
});

vi.mock("./PdfPreview", () => ({
  PdfPreview: ({ label }: { label: string }) => (
    <div data-testid="pdf-preview">{label}</div>
  ),
}));

// The open block is a CodeMirror editor, so its text lives in editor state
// rather than in a DOM value. These reach through to the same three things a
// textarea exposed directly: the element, its text, and the caret.
function editorEl(): HTMLElement {
  return screen.getByRole("textbox");
}

function editorView(): EditorView {
  const view = EditorView.findFromDOM(editorEl() as HTMLElement);
  if (!view) {
    throw new Error("no CodeMirror view is mounted on the active block");
  }
  return view;
}

function editorValue(): string {
  return editorView().state.doc.toString();
}

function caretPos(): number {
  return editorView().state.selection.main.head;
}

function setCaret(at: number): void {
  const view = editorView();
  view.dispatch({ selection: { anchor: at } });
}

/** Replaces the block's whole text, as typing over a selection would. */
function setEditorValue(text: string): void {
  const view = editorView();
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: text },
  });
}

/**
 * Renders the note body the way NotePage does: the rendered markdown is the
 * file minus its frontmatter, and every block carries the offset needed to map
 * back to a line in the whole file.
 */
function NoteHarness({
  initialContent,
  onContentChange,
  onInProgressEdit,
  writeEnabled = true,
}: {
  initialContent: string;
  onContentChange?: (next: string) => void;
  onInProgressEdit?: (next: string) => void;
  writeEnabled?: boolean;
}) {
  const [content, setContent] = useState(initialContent);
  const body = content
    .split("\n")
    .slice(frontmatterLineOffset(content))
    .join("\n");

  return (
    <InlineEditorProvider
      content={content}
      frontmatterOffset={frontmatterLineOffset(content)}
      writeEnabled={writeEnabled}
      onInProgressChange={onInProgressEdit}
      onChange={(next) => {
        setContent(next);
        onContentChange?.(next);
      }}
    >
      <div className="note-body">
        <ReactMarkdown
          remarkPlugins={[remarkGfm]}
          components={createNoteMarkdownComponents("Home.md", new Map(), {
            editable: true,
          })}
        >
          {body}
        </ReactMarkdown>
      </div>
    </InlineEditorProvider>
  );
}

const WITH_FRONTMATTER = `---
title: Home
---
First paragraph.

Second *paragraph* here.
`;

describe("entering a block", () => {
  it("shows the block's own markdown source, syntax and all", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);

    fireEvent.click(screen.getByText(/Second/));

    expect(editorValue()).toBe("Second *paragraph* here.");
  });

  it("maps through the frontmatter offset rather than the rendered line", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);

    fireEvent.click(screen.getByText("First paragraph."));

    expect(editorValue()).toBe("First paragraph.");
  });

  it("reveals the heading marker when a heading is entered", () => {
    render(<NoteHarness initialContent={"## A heading\n\nBody.\n"} />);

    fireEvent.click(screen.getByRole("heading", { name: "A heading" }));

    expect(editorValue()).toBe("## A heading");
  });

  it("edits one list item rather than the whole list", () => {
    render(<NoteHarness initialContent={"- one\n- two\n- three\n"} />);

    fireEvent.click(screen.getByText("two"));

    expect(editorValue()).toBe("- two");
  });

  it("keeps only one block active at a time", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);

    fireEvent.click(screen.getByText("First paragraph."));
    fireEvent.click(screen.getByText(/Second/));

    expect(screen.getAllByRole("textbox")).toHaveLength(1);
    expect(editorValue()).toBe("Second *paragraph* here.");
  });

  it("does not open a block in a read-only vault", () => {
    render(
      <NoteHarness initialContent={WITH_FRONTMATTER} writeEnabled={false} />,
    );

    fireEvent.click(screen.getByText("First paragraph."));

    expect(screen.queryByRole("textbox")).toBeNull();
  });
});

describe("committing a block", () => {
  it("writes the edited lines back into the whole document", () => {
    const onChange = vi.fn();
    render(
      <NoteHarness
        initialContent={WITH_FRONTMATTER}
        onContentChange={onChange}
      />,
    );

    fireEvent.click(screen.getByText(/Second/));
    setEditorValue("Second **paragraph** here.");
    fireEvent.blur(screen.getByRole("textbox"));

    expect(onChange).toHaveBeenCalledWith(
      `---
title: Home
---
First paragraph.

Second **paragraph** here.
`,
    );
  });

  it("leaves the rest of the file untouched when a heading changes", () => {
    const onChange = vi.fn();
    render(
      <NoteHarness
        initialContent={"## A heading\n\nBody.\n"}
        onContentChange={onChange}
      />,
    );

    fireEvent.click(screen.getByRole("heading", { name: "A heading" }));
    setEditorValue("### A heading");
    fireEvent.blur(screen.getByRole("textbox"));

    expect(onChange).toHaveBeenCalledWith("### A heading\n\nBody.\n");
  });

  it("returns to the rendered view after committing", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);

    fireEvent.click(screen.getByText(/Second/));
    fireEvent.blur(screen.getByRole("textbox"));

    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.getByText(/Second/)).toBeInTheDocument();
  });
});

describe("touch entry", () => {
  // Reading is the dominant mode on a phone, and entering on a single tap would
  // raise the keyboard on every stray touch. Entry is a deliberate double tap,
  // which unlike a hold does not race the OS text-selection gesture.
  function tap(el: Element, x = 10, y = 10) {
    fireEvent.pointerDown(el, {
      pointerType: "touch",
      clientX: x,
      clientY: y,
      bubbles: true,
    });
    fireEvent.pointerUp(el, {
      pointerType: "touch",
      clientX: x,
      clientY: y,
      bubbles: true,
    });
    fireEvent.click(el, { clientX: x, clientY: y, bubbles: true });
  }

  it("does not enter a block on a single tap", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);
    const target = screen.getByText("First paragraph.");

    tap(target);

    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("enters a block on a double tap", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);
    const target = screen.getByText("First paragraph.");

    tap(target);
    tap(target);

    expect(editorValue()).toBe("First paragraph.");
  });

  it("does not enter when the second tap comes too late", () => {
    vi.useFakeTimers();
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);
    const target = screen.getByText("First paragraph.");

    tap(target);
    act(() => {
      vi.advanceTimersByTime(500);
    });
    tap(target);

    expect(screen.queryByRole("textbox")).toBeNull();
    vi.useRealTimers();
  });

  it("does not enter when the second tap lands too far away", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);
    const target = screen.getByText("First paragraph.");

    tap(target, 10, 10);
    tap(target, 10, 90);

    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("treats a third tap as the start of a new pair, not another entry", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);
    const target = screen.getByText("First paragraph.");

    tap(target);
    tap(target);
    fireEvent.blur(screen.getByRole("textbox"));
    tap(screen.getByText("First paragraph."));

    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("still enters immediately on a mouse click", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);
    const target = screen.getByText("First paragraph.");

    fireEvent.pointerDown(target, { pointerType: "mouse", bubbles: true });
    fireEvent.click(target);

    expect(editorValue()).toBe("First paragraph.");
  });

  // A screen reader activating the focused block synthesizes a bare click with
  // no pointer sequence in front of it. Requiring two of those would put touch
  // users of VoiceOver and TalkBack behind a gesture they cannot make.
  it("enters on a single activation with no pointer event, as a screen reader sends", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);

    fireEvent.click(screen.getByText("First paragraph."));

    expect(editorValue()).toBe("First paragraph.");
  });
});

describe("keyboard entry", () => {
  it("puts every editable block in the tab order", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);

    expect(screen.getByText("First paragraph.")).toHaveAttribute(
      "tabindex",
      "0",
    );
  });

  it("opens the focused block on Enter", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);

    fireEvent.keyDown(screen.getByText("First paragraph."), { key: "Enter" });

    expect(editorValue()).toBe("First paragraph.");
  });

  it("does not open on other keys", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);

    fireEvent.keyDown(screen.getByText("First paragraph."), { key: "a" });

    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("returns focus to the block on Escape so a keyboard user is never trapped", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);
    fireEvent.keyDown(screen.getByText("First paragraph."), { key: "Enter" });

    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Escape" });

    expect(screen.queryByRole("textbox")).toBeNull();
    expect(document.activeElement).toBe(screen.getByText("First paragraph."));
  });

  it("does not enter a block while an IME composition is active", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);

    fireEvent.keyDown(screen.getByText("First paragraph."), {
      key: "Enter",
      isComposing: true,
    });

    expect(screen.queryByRole("textbox")).toBeNull();
  });
});

describe("more unit types", () => {
  it("edits a fenced code block including its fences", () => {
    render(
      <NoteHarness initialContent={"```js\nconst x = 1\n```\n\nAfter.\n"} />,
    );

    fireEvent.click(screen.getByText(/const x = 1/));

    expect(editorValue()).toBe("```js\nconst x = 1\n```");
  });

  it("edits a single table row", () => {
    render(
      <NoteHarness
        initialContent={"| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n"}
      />,
    );

    fireEvent.click(screen.getByText("3"));

    expect(editorValue()).toBe("| 3 | 4 |");
  });
});

describe("structural keys", () => {
  function openBlock(text: string | RegExp) {
    fireEvent.click(screen.getByText(text));
    return editorEl();
  }

  it("Enter at the end of a list item starts the next one", () => {
    const onChange = vi.fn();
    render(
      <NoteHarness
        initialContent={"- one\n- two\n"}
        onContentChange={onChange}
      />,
    );
    const ta = openBlock("two");
    setCaret(5);

    fireEvent.keyDown(ta, { key: "Enter" });

    expect(onChange).toHaveBeenCalledWith("- one\n- two\n- \n");
  });

  it("Enter in a paragraph starts a new paragraph", () => {
    const onChange = vi.fn();
    render(
      <NoteHarness initialContent={"hello\n"} onContentChange={onChange} />,
    );
    const ta = openBlock("hello");
    setCaret(5);

    fireEvent.keyDown(ta, { key: "Enter" });

    expect(onChange).toHaveBeenCalledWith("hello\n\n\n");
  });

  it("Shift+Enter inserts a line break instead of splitting", () => {
    const onChange = vi.fn();
    render(
      <NoteHarness initialContent={"hello\n"} onContentChange={onChange} />,
    );
    const ta = openBlock("hello");
    setCaret(5);

    fireEvent.keyDown(ta, { key: "Enter", shiftKey: true });

    expect(onChange).not.toHaveBeenCalled();
  });

  it("Enter inside a fenced code block does not split the block", () => {
    const onChange = vi.fn();
    render(
      <NoteHarness
        initialContent={"```js\nconst x = 1\n```\n"}
        onContentChange={onChange}
      />,
    );
    const ta = openBlock(/const x = 1/);
    setCaret(5);

    fireEvent.keyDown(ta, { key: "Enter" });

    expect(onChange).not.toHaveBeenCalled();
  });

  it("Backspace at offset 0 merges into the previous unit", () => {
    const onChange = vi.fn();
    render(
      <NoteHarness
        initialContent={"- one\n- two\n"}
        onContentChange={onChange}
      />,
    );
    const ta = openBlock("two");
    setCaret(0);

    fireEvent.keyDown(ta, { key: "Backspace" });

    expect(onChange).toHaveBeenCalledWith("- onetwo\n");
  });

  it("Backspace anywhere else deletes normally", () => {
    const onChange = vi.fn();
    render(
      <NoteHarness
        initialContent={"- one\n- two\n"}
        onContentChange={onChange}
      />,
    );
    const ta = openBlock("two");
    setCaret(3);

    fireEvent.keyDown(ta, { key: "Backspace" });

    expect(onChange).not.toHaveBeenCalled();
  });

  it("Tab indents a list item", () => {
    const onChange = vi.fn();
    render(
      <NoteHarness
        initialContent={"- one\n- two\n"}
        onContentChange={onChange}
      />,
    );
    const ta = openBlock("two");

    fireEvent.keyDown(ta, { key: "Tab" });

    expect(onChange).toHaveBeenCalledWith("- one\n  - two\n");
  });

  it("Shift+Tab outdents a list item", () => {
    const onChange = vi.fn();
    render(
      <NoteHarness
        initialContent={"- one\n  - two\n"}
        onContentChange={onChange}
      />,
    );
    const ta = openBlock("two");

    fireEvent.keyDown(ta, { key: "Tab", shiftKey: true });

    expect(onChange).toHaveBeenCalledWith("- one\n- two\n");
  });

  it("does not act on any structural key while an IME is composing", () => {
    const onChange = vi.fn();
    render(
      <NoteHarness
        initialContent={"- one\n- two\n"}
        onContentChange={onChange}
      />,
    );
    const ta = openBlock("two");
    setCaret(5);

    fireEvent.keyDown(ta, { key: "Enter", isComposing: true });
    setCaret(0);
    fireEvent.keyDown(ta, { key: "Backspace", isComposing: true });
    fireEvent.keyDown(ta, { key: "Tab", isComposing: true });

    expect(onChange).not.toHaveBeenCalled();
  });
});

describe("committing edge cases", () => {
  it("Escape keeps what was typed, it does not discard it", () => {
    const onChange = vi.fn();
    render(
      <NoteHarness initialContent={"hello\n"} onContentChange={onChange} />,
    );
    fireEvent.click(screen.getByText("hello"));
    setEditorValue("hello there");

    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Escape" });

    expect(onChange).toHaveBeenCalledWith("hello there\n");
  });

  // After a split the next block is at a different range, and if React reuses
  // the same input instance it keeps the old guard state and the typed text is
  // silently dropped on blur.
  it("saves text typed into the block created by a split", () => {
    const onChange = vi.fn();
    render(
      <NoteHarness initialContent={"- one\n"} onContentChange={onChange} />,
    );
    const ta = screen.getByText("one").closest("li")!;
    fireEvent.click(ta);
    const input = editorEl();
    setCaret(5);
    fireEvent.keyDown(input, { key: "Enter" });

    const created = editorEl();
    setEditorValue("- two");
    fireEvent.blur(created);

    expect(onChange).toHaveBeenLastCalledWith("- one\n- two\n");
  });
});

describe("stale rendered tree", () => {
  // Wikilink resolution awaits a network round-trip, so between a content
  // change and the response every block range on screen describes the previous
  // document. Entering one then edits the wrong lines.
  it("refuses to enter a block while the tree is settling", () => {
    render(
      <InlineEditorProvider
        content={"one\n\ntwo\n"}
        frontmatterOffset={0}
        writeEnabled
        settling
        onChange={() => {}}
      >
        <div className="note-body">
          <ReactMarkdown
            remarkPlugins={[remarkGfm]}
            components={createNoteMarkdownComponents("Home.md", new Map(), {
              editable: true,
            })}
          >
            {"one\n\ntwo\n"}
          </ReactMarkdown>
        </div>
      </InlineEditorProvider>,
    );

    fireEvent.click(screen.getByText("one"));

    expect(screen.queryByRole("textbox")).toBeNull();
  });
});

describe("arrow navigation between units", () => {
  it("ArrowUp on the first line moves to the previous unit", () => {
    render(<NoteHarness initialContent={"- one\n- two\n- three\n"} />);
    fireEvent.click(screen.getByText("two"));
    const ta = editorEl();
    setCaret(0);

    fireEvent.keyDown(ta, { key: "ArrowUp" });

    expect(editorValue()).toBe("- one");
  });

  it("ArrowDown on the last line moves to the next unit", () => {
    render(<NoteHarness initialContent={"- one\n- two\n- three\n"} />);
    fireEvent.click(screen.getByText("two"));
    const ta = editorEl();
    setCaret(5);

    fireEvent.keyDown(ta, { key: "ArrowDown" });

    expect(editorValue()).toBe("- three");
  });

  it("preserves the column when moving between units", () => {
    render(<NoteHarness initialContent={"- alpha\n- bravo\n"} />);
    fireEvent.click(screen.getByText("bravo"));
    const ta = editorEl();
    setCaret(4);

    fireEvent.keyDown(ta, { key: "ArrowUp" });

    expect(caretPos()).toBe(4);
  });

  it("stays put at the first unit", () => {
    render(<NoteHarness initialContent={"- one\n- two\n"} />);
    fireEvent.click(screen.getByText("one"));
    const ta = editorEl();
    setCaret(0);

    fireEvent.keyDown(ta, { key: "ArrowUp" });

    expect(editorValue()).toBe("- one");
  });

  it("leaves a multi-line block when the caret is not on its edge line", () => {
    // A hard-wrapped paragraph is one block spanning two source lines.
    render(
      <NoteHarness initialContent={"first\n\nwrapped one\nwrapped two\n"} />,
    );
    fireEvent.click(screen.getByText(/wrapped one/));
    const ta = editorEl();
    expect(editorValue()).toBe("wrapped one\nwrapped two");
    // Caret on the second source line: within-block motion is the browser's.
    setCaret(15);

    fireEvent.keyDown(ta, { key: "ArrowUp" });

    expect(editorValue()).toBe("wrapped one\nwrapped two");
  });

  it("does not navigate while an IME is composing", () => {
    render(<NoteHarness initialContent={"- one\n- two\n"} />);
    fireEvent.click(screen.getByText("two"));
    const ta = editorEl();
    setCaret(0);

    fireEvent.keyDown(ta, { key: "ArrowUp", isComposing: true });

    expect(editorValue()).toBe("- two");
  });
});

describe("task list checkboxes", () => {
  const TASKS = "- [ ] first\n- [x] second\n";

  it("toggles a task without opening the block for editing", () => {
    const onChange = vi.fn();
    render(<NoteHarness initialContent={TASKS} onContentChange={onChange} />);

    fireEvent.click(screen.getAllByRole("checkbox")[0]);

    expect(onChange).toHaveBeenCalledWith("- [x] first\n- [x] second\n");
    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("unchecks a checked task", () => {
    const onChange = vi.fn();
    render(<NoteHarness initialContent={TASKS} onContentChange={onChange} />);

    fireEvent.click(screen.getAllByRole("checkbox")[1]);

    expect(onChange).toHaveBeenCalledWith("- [ ] first\n- [ ] second\n");
  });

  it("does not toggle in a read-only vault", () => {
    const onChange = vi.fn();
    render(
      <NoteHarness
        initialContent={TASKS}
        onContentChange={onChange}
        writeEnabled={false}
      />,
    );

    fireEvent.click(screen.getAllByRole("checkbox")[0]);

    expect(onChange).not.toHaveBeenCalled();
  });

  it("clicking the item's text still opens it for editing", () => {
    render(<NoteHarness initialContent={TASKS} />);

    fireEvent.click(screen.getByText("first"));

    expect(editorValue()).toBe("- [ ] first");
  });
});

describe("callouts", () => {
  const CALLOUT = `---
title: Home
---
> [!warning] Heads up
> Mind the gap.

After.
`;

  it("edits the callout title line, offset by the frontmatter", () => {
    render(<NoteHarness initialContent={CALLOUT} />);

    fireEvent.click(screen.getByText("Heads up"));

    expect(editorValue()).toBe("> [!warning] Heads up");
  });

  it("edits the callout body, keeping its quote prefix", () => {
    render(<NoteHarness initialContent={CALLOUT} />);

    fireEvent.click(screen.getByText(/Mind the gap/));

    expect(editorValue()).toBe("> Mind the gap.");
  });

  it("writes an edited callout body back to the right lines", () => {
    const onChange = vi.fn();
    render(<NoteHarness initialContent={CALLOUT} onContentChange={onChange} />);
    fireEvent.click(screen.getByText(/Mind the gap/));
    setEditorValue("> Mind the step.");
    fireEvent.blur(screen.getByRole("textbox"));

    expect(onChange).toHaveBeenCalledWith(
      "---\ntitle: Home\n---\n> [!warning] Heads up\n> Mind the step.\n\nAfter.\n",
    );
  });
});

describe("in-progress text is not stranded", () => {
  // A block holds its text locally until blur, so without a signal on every
  // keystroke the idle flush has nothing to flush and closing the tab
  // mid-paragraph loses everything typed since the block was opened.
  it("reports each keystroke to the page, not just the final commit", () => {
    const onEdit = vi.fn();
    render(
      <NoteHarness initialContent={"hello\n"} onInProgressEdit={onEdit} />,
    );
    fireEvent.click(screen.getByText("hello"));

    setEditorValue("hello t");
    setEditorValue("hello th");

    expect(onEdit).toHaveBeenCalledWith("hello th\n");
  });
});

describe("callouts are addressed per line (D25a)", () => {
  const CALLOUT = `> [!warning] Heads up
> First body line.
> Second body line.
> Third body line.

After.
`;

  it("opens only the clicked body line, not the whole run", () => {
    render(<NoteHarness initialContent={CALLOUT} />);

    fireEvent.click(screen.getByText(/Second body line/));

    expect(editorValue()).toBe("> Second body line.");
  });

  it("opens the first body line on its own", () => {
    render(<NoteHarness initialContent={CALLOUT} />);

    fireEvent.click(screen.getByText(/First body line/));

    expect(editorValue()).toBe("> First body line.");
  });

  it("writes one line back, leaving the rest of the callout alone", () => {
    const onChange = vi.fn();
    render(<NoteHarness initialContent={CALLOUT} onContentChange={onChange} />);

    fireEvent.click(screen.getByText(/Third body line/));
    setEditorValue("> Third line, edited.");
    fireEvent.blur(screen.getByRole("textbox"));

    expect(onChange).toHaveBeenCalledWith(
      "> [!warning] Heads up\n> First body line.\n> Second body line.\n> Third line, edited.\n\nAfter.\n",
    );
  });

  it("still opens the title line on its own", () => {
    render(<NoteHarness initialContent={CALLOUT} />);

    fireEvent.click(screen.getByText("Heads up"));

    expect(editorValue()).toBe("> [!warning] Heads up");
  });
});

describe("callout lines survive being rebuilt (D25a)", () => {
  // A line whose content is a single element leaves the soft break as a
  // string child of its own. Treating that break as stray whitespace merges
  // two source lines into one block, so the second becomes unreachable and
  // committing the first writes the merged text over one line.
  const ELEMENT_LINES = `> [!note] Emphasis
> **alpha**
> **beta**
`;

  it("addresses a line whose whole content is one element", () => {
    render(<NoteHarness initialContent={ELEMENT_LINES} />);

    fireEvent.click(screen.getByText("beta"));

    expect(editorValue()).toBe("> **beta**");
  });

  it("keeps the line above it addressable too", () => {
    render(<NoteHarness initialContent={ELEMENT_LINES} />);

    fireEvent.click(screen.getByText("alpha"));

    expect(editorValue()).toBe("> **alpha**");
  });

  it("writes an element-only line back without touching its neighbour", () => {
    const onChange = vi.fn();
    render(
      <NoteHarness initialContent={ELEMENT_LINES} onContentChange={onChange} />,
    );

    fireEvent.click(screen.getByText("beta"));
    setEditorValue("> **gamma**");
    fireEvent.blur(screen.getByRole("textbox"));

    expect(onChange).toHaveBeenCalledWith(
      "> [!note] Emphasis\n> **alpha**\n> **gamma**\n",
    );
  });

  // A link keeps its own click behaviour, so this line is checked by the range
  // it claims rather than by entering it.
  it("gives a line that is only a link its own range", () => {
    render(
      <NoteHarness
        initialContent={"> [!note] Links\n> plain line\n> [text](x.md)\n"}
      />,
    );

    const line = screen.getByText("text").closest(".editable-block");

    expect(line).toHaveAttribute("data-start-line", "3");
    expect(line).toHaveAttribute("data-end-line", "3");
  });
});

describe("multi-line list items are addressed per line (D25a)", () => {
  const WRAPPED = `- First bullet line
  continues here
- Second bullet
`;

  it("opens only the continuation line, not the whole item", () => {
    render(<NoteHarness initialContent={WRAPPED} />);

    fireEvent.click(screen.getByText(/continues here/));

    expect(editorValue()).toBe("  continues here");
  });

  it("opens the item's first line on its own", () => {
    render(<NoteHarness initialContent={WRAPPED} />);

    fireEvent.click(screen.getByText(/First bullet line/));

    expect(editorValue()).toBe("- First bullet line");
  });

  it("writes one line back, leaving the rest of the item alone", () => {
    const onChange = vi.fn();
    render(<NoteHarness initialContent={WRAPPED} onContentChange={onChange} />);

    fireEvent.click(screen.getByText(/continues here/));
    setEditorValue("  continues differently");
    fireEvent.blur(screen.getByRole("textbox"));

    expect(onChange).toHaveBeenCalledWith(
      "- First bullet line\n  continues differently\n- Second bullet\n",
    );
  });

  it("still opens a single-line item whole", () => {
    render(<NoteHarness initialContent={WRAPPED} />);

    fireEvent.click(screen.getByText(/Second bullet/));

    expect(editorValue()).toBe("- Second bullet");
  });

  // D8: an item's own lines stop where its nested list begins, so splitting
  // must not reach into the sublist.
  it("does not take lines from a nested sublist", () => {
    const NESTED = `- Parent line one
  parent line two
  - Child item
- Sibling
`;
    render(<NoteHarness initialContent={NESTED} />);

    fireEvent.click(screen.getByText(/parent line two/));
    expect(editorValue()).toBe("  parent line two");

    fireEvent.blur(screen.getByRole("textbox"));
    fireEvent.click(screen.getByText("Child item"));
    expect(editorValue()).toBe("  - Child item");
  });

  it("addresses each line of a wrapped item in a loose list", () => {
    const LOOSE = `- First bullet line
  continues here

- Second bullet
`;
    render(<NoteHarness initialContent={LOOSE} />);

    fireEvent.click(screen.getByText(/continues here/));

    expect(editorValue()).toBe("  continues here");
  });

  it("still toggles the checkbox of a wrapped task item", () => {
    const onChange = vi.fn();
    render(
      <NoteHarness
        initialContent={"- [ ] wrapped task\n  second line\n"}
        onContentChange={onChange}
      />,
    );

    fireEvent.click(screen.getByRole("checkbox"));

    expect(onChange).toHaveBeenCalledWith(
      "- [x] wrapped task\n  second line\n",
    );
    expect(screen.queryByRole("textbox")).toBeNull();
  });
});
