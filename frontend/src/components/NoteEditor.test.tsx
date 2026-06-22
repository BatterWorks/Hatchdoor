import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import type { ComponentProps } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NoteEditor } from "./NoteEditor";

afterEach(() => {
  cleanup();
});

function FrontmatterHarness({
  initialContent,
  onSaveContent,
  uploadAttachment,
  conflictReview,
}: {
  initialContent: string;
  onSaveContent: (content: string) => void;
  uploadAttachment?: (file: File) => Promise<string>;
  conflictReview?: ComponentProps<typeof NoteEditor>["conflictReview"];
}) {
  const [content, setContent] = useState(initialContent);

  return (
    <NoteEditor
      content={content}
      saving={false}
      error={null}
      onChange={setContent}
      onSave={() => onSaveContent(content)}
      onCancel={() => {}}
      onUploadAttachment={uploadAttachment}
      conflictReview={conflictReview}
    />
  );
}

describe("NoteEditor frontmatter properties", () => {
  it("edits simple frontmatter separately from the markdown body", () => {
    const saveContent = vi.fn();

    render(
      <FrontmatterHarness
        initialContent={"---\ntitle: Home\ntags:\n  - work\n---\n# Body"}
        onSaveContent={saveContent}
      />,
    );

    fireEvent.change(screen.getByLabelText("Property title"), {
      target: { value: "Home Base" },
    });
    fireEvent.change(screen.getByLabelText("Property tags"), {
      target: { value: "work, planning" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(saveContent).toHaveBeenCalledWith(
      "---\ntitle: Home Base\ntags:\n  - work\n  - planning\n---\n# Body",
    );
  });
});

describe("NoteEditor attachment uploads", () => {
  it("uploads a pasted image and inserts an Obsidian image embed", async () => {
    const uploadAttachment = vi
      .fn()
      .mockResolvedValue("Attachments/pasted.png");
    const saveContent = vi.fn();

    render(
      <FrontmatterHarness
        initialContent={"# Body\n"}
        onSaveContent={saveContent}
        uploadAttachment={uploadAttachment}
      />,
    );

    const textarea = screen.getByRole("textbox", {
      name: "Markdown content",
    }) as HTMLTextAreaElement;
    textarea.setSelectionRange(7, 7);
    const file = new File(["png-bytes"], "pasted.png", { type: "image/png" });
    fireEvent.paste(textarea, {
      clipboardData: {
        files: [file],
      },
    });

    await screen.findByText("Inserted attachment: Attachments/pasted.png");
    expect(uploadAttachment).toHaveBeenCalledWith(file);
    expect(textarea).toHaveValue("# Body\n![[Attachments/pasted.png]]");
  });

  it("shows a visible drop target while dragging an image over the editor", () => {
    const uploadAttachment = vi.fn();

    render(
      <FrontmatterHarness
        initialContent={"# Body\n"}
        onSaveContent={() => {}}
        uploadAttachment={uploadAttachment}
      />,
    );

    const textarea = screen.getByRole("textbox", {
      name: "Markdown content",
    });
    fireEvent.dragEnter(textarea, {
      dataTransfer: {
        files: [new File(["png-bytes"], "pasted.png", { type: "image/png" })],
      },
    });

    expect(screen.getByText("Drop image to attach")).toBeInTheDocument();
    expect(textarea.closest(".note-editor-input")).toHaveClass("drag-active");
  });
});

describe("NoteEditor conflict review", () => {
  it("gives conflict actions distinct, explicit labels", () => {
    render(
      <FrontmatterHarness
        initialContent={"# Home\nDraft"}
        onSaveContent={() => {}}
        conflictReview={{
          diskContent: "# Home\nDisk",
          draftContent: "# Home\nDraft",
          onUseDisk: vi.fn(),
          onKeepDraft: vi.fn(),
        }}
      />,
    );

    expect(
      screen.getByRole("button", { name: "Discard draft and use disk" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Keep draft on latest" }),
    ).toBeInTheDocument();
  });
});
