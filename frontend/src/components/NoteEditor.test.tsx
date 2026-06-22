import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NoteEditor } from "./NoteEditor";

afterEach(() => {
  cleanup();
});

function FrontmatterHarness({
  initialContent,
  onSaveContent,
  uploadAttachment,
}: {
  initialContent: string;
  onSaveContent: (content: string) => void;
  uploadAttachment?: (file: File) => Promise<string>;
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
});
