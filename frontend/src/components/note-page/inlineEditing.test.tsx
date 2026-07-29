import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import ReactMarkdown from "react-markdown";
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
