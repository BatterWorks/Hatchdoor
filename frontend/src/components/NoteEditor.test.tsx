import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
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
  onDemoRefusal,
}: {
  initialContent: string;
  onSaveContent: (content: string) => void;
  uploadAttachment?: (file: File) => Promise<string>;
  conflictReview?: ComponentProps<typeof NoteEditor>["conflictReview"];
  onDemoRefusal?: (error: unknown) => boolean;
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
      onDemoRefusal={onDemoRefusal}
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

  it("uploads Safari pasted images exposed only through clipboard items", async () => {
    const uploadAttachment = vi
      .fn()
      .mockResolvedValue("Attachments/safari-paste.png");
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
    const file = new File(["png-bytes"], "safari-paste.png", {
      type: "image/png",
    });
    fireEvent.paste(textarea, {
      clipboardData: {
        files: [],
        items: [
          {
            kind: "file",
            type: "image/png",
            getAsFile: () => file,
          },
        ],
      },
    });

    await screen.findByText(
      "Inserted attachment: Attachments/safari-paste.png",
    );
    expect(uploadAttachment).toHaveBeenCalledWith(file);
    expect(textarea).toHaveValue("# Body\n![[Attachments/safari-paste.png]]");
  });

  it("uploads a dropped PDF and inserts an embed", async () => {
    const uploadAttachment = vi
      .fn()
      .mockResolvedValue("Attachments/report.pdf");

    render(
      <FrontmatterHarness
        initialContent={"# Body\n"}
        onSaveContent={() => {}}
        uploadAttachment={uploadAttachment}
      />,
    );

    const textarea = screen.getByRole("textbox", {
      name: "Markdown content",
    }) as HTMLTextAreaElement;
    textarea.setSelectionRange(7, 7);
    const file = new File(["%PDF-1.7"], "report.pdf", {
      type: "application/pdf",
    });
    fireEvent.drop(textarea, { dataTransfer: { files: [file] } });

    await screen.findByText("Inserted attachment: Attachments/report.pdf");
    expect(uploadAttachment).toHaveBeenCalledWith(file);
    expect(textarea).toHaveValue("# Body\n![[Attachments/report.pdf]]");
  });

  it("defers a demo_read_only upload refusal to onDemoRefusal instead of its own inline notice (#152)", async () => {
    const demoError = new Error(
      "This is a public read-only demo instance; mutations and Vault-control operations are disabled.",
    ) as Error & { code?: string };
    demoError.code = "demo_read_only";
    const uploadAttachment = vi.fn().mockRejectedValue(demoError);
    const onDemoRefusal = vi.fn().mockReturnValue(true);

    render(
      <FrontmatterHarness
        initialContent={"# Body\n"}
        onSaveContent={() => {}}
        uploadAttachment={uploadAttachment}
        onDemoRefusal={onDemoRefusal}
      />,
    );

    const textarea = screen.getByRole("textbox", {
      name: "Markdown content",
    }) as HTMLTextAreaElement;
    textarea.setSelectionRange(7, 7);
    const file = new File(["png-bytes"], "pasted.png", { type: "image/png" });
    fireEvent.paste(textarea, { clipboardData: { files: [file] } });

    await waitFor(() => {
      expect(onDemoRefusal).toHaveBeenCalledWith(demoError);
    });
    expect(screen.queryByText(/Upload failed/)).not.toBeInTheDocument();
    expect(screen.queryByText(demoError.message)).not.toBeInTheDocument();
  });

  it("names what it accepts instead of uploading an unsupported file", async () => {
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
    }) as HTMLTextAreaElement;
    fireEvent.drop(textarea, {
      dataTransfer: {
        files: [
          new File(["doc"], "notes.docx", {
            type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
          }),
        ],
      },
    });

    await screen.findByText("Hatchdoor accepts images and PDFs.");
    expect(uploadAttachment).not.toHaveBeenCalled();
  });

  it("reports the size instead of uploading an over-limit file", async () => {
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
    }) as HTMLTextAreaElement;
    const big = new File(["x"], "big.pdf", { type: "application/pdf" });
    Object.defineProperty(big, "size", { value: 14 * 1024 * 1024 });
    fireEvent.drop(textarea, { dataTransfer: { files: [big] } });

    await screen.findByText("That file is 14 MB. The limit is 10 MB.");
    expect(uploadAttachment).not.toHaveBeenCalled();
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

  it("keeps conflict review focused by hiding generic notices", () => {
    render(
      <NoteEditor
        content={"# Home\nDraft"}
        saving={false}
        error={null}
        notice="This note changed on disk while you were editing."
        onChange={() => {}}
        onSave={() => {}}
        onCancel={() => {}}
        conflictReview={{
          diskContent: "# Home\nDisk",
          draftContent: "# Home\nDraft",
          onUseDisk: vi.fn(),
          onKeepDraft: vi.fn(),
        }}
      />,
    );

    expect(
      screen.getByRole("region", { name: "Conflict review" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("This note changed on disk while you were editing."),
    ).not.toBeInTheDocument();
  });
});
