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

/**
 * Renders the note body the way NotePage does: the rendered markdown is the
 * file minus its frontmatter, and every block carries the offset needed to map
 * back to a line in the whole file.
 */
function NoteHarness({
  initialContent,
  onContentChange,
  writeEnabled = true,
}: {
  initialContent: string;
  onContentChange?: (next: string) => void;
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

    expect(screen.getByRole("textbox")).toHaveValue("Second *paragraph* here.");
  });

  it("maps through the frontmatter offset rather than the rendered line", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);

    fireEvent.click(screen.getByText("First paragraph."));

    expect(screen.getByRole("textbox")).toHaveValue("First paragraph.");
  });

  it("reveals the heading marker when a heading is entered", () => {
    render(<NoteHarness initialContent={"## A heading\n\nBody.\n"} />);

    fireEvent.click(screen.getByRole("heading", { name: "A heading" }));

    expect(screen.getByRole("textbox")).toHaveValue("## A heading");
  });

  it("edits one list item rather than the whole list", () => {
    render(<NoteHarness initialContent={"- one\n- two\n- three\n"} />);

    fireEvent.click(screen.getByText("two"));

    expect(screen.getByRole("textbox")).toHaveValue("- two");
  });

  it("keeps only one block active at a time", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);

    fireEvent.click(screen.getByText("First paragraph."));
    fireEvent.click(screen.getByText(/Second/));

    expect(screen.getAllByRole("textbox")).toHaveLength(1);
    expect(screen.getByRole("textbox")).toHaveValue("Second *paragraph* here.");
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
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "Second **paragraph** here." },
    });
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
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "### A heading" },
    });
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
  // Reading is the dominant mode on a phone, and tap-to-place-caret would
  // raise the keyboard on every stray touch. Entry is a deliberate long press.
  function touch(
    el: Element,
    type: "pointerDown" | "pointerUp" | "pointerMove",
    x = 10,
    y = 10,
  ) {
    fireEvent[type](el, {
      pointerType: "touch",
      clientX: x,
      clientY: y,
      bubbles: true,
    });
  }

  it("does not enter a block on a tap", () => {
    vi.useFakeTimers();
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);
    const target = screen.getByText("First paragraph.");

    touch(target, "pointerDown");
    act(() => {
      vi.advanceTimersByTime(120);
    });
    touch(target, "pointerUp");
    fireEvent.click(target);

    expect(screen.queryByRole("textbox")).toBeNull();
    vi.useRealTimers();
  });

  it("enters a block on a long press", () => {
    vi.useFakeTimers();
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);
    const target = screen.getByText("First paragraph.");

    touch(target, "pointerDown");
    act(() => {
      vi.advanceTimersByTime(600);
    });

    expect(screen.getByRole("textbox")).toHaveValue("First paragraph.");
    vi.useRealTimers();
  });

  it("cancels the long press when the finger moves, because that is a scroll", () => {
    vi.useFakeTimers();
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);
    const target = screen.getByText("First paragraph.");

    touch(target, "pointerDown", 10, 10);
    touch(target, "pointerMove", 10, 80);
    act(() => {
      vi.advanceTimersByTime(600);
    });

    expect(screen.queryByRole("textbox")).toBeNull();
    vi.useRealTimers();
  });

  it("still enters immediately on a mouse click", () => {
    render(<NoteHarness initialContent={WITH_FRONTMATTER} />);
    const target = screen.getByText("First paragraph.");

    fireEvent.pointerDown(target, { pointerType: "mouse", bubbles: true });
    fireEvent.click(target);

    expect(screen.getByRole("textbox")).toHaveValue("First paragraph.");
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

    expect(screen.getByRole("textbox")).toHaveValue("First paragraph.");
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

    expect(screen.getByRole("textbox")).toHaveValue("```js\nconst x = 1\n```");
  });

  it("edits a single table row", () => {
    render(
      <NoteHarness
        initialContent={"| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n"}
      />,
    );

    fireEvent.click(screen.getByText("3"));

    expect(screen.getByRole("textbox")).toHaveValue("| 3 | 4 |");
  });
});
