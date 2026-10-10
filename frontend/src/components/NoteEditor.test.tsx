import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { useState } from "react";
import type { ComponentProps } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { fetchAttachmentMaxBytes } from "../api/writeApi";
import { NoteEditor, type UploadedAttachment } from "./NoteEditor";

vi.mock("../api/writeApi", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../api/writeApi")>()),
  fetchAttachmentMaxBytes: vi.fn(),
}));

const mockedMaxBytes = vi.mocked(fetchAttachmentMaxBytes);
const MB = 1024 * 1024;

function pdfOfSize(name: string, bytes: number) {
  const file = new File(["%PDF-1.7"], name, { type: "application/pdf" });
  Object.defineProperty(file, "size", { value: bytes });
  return file;
}

beforeEach(() => {
  mockedMaxBytes.mockResolvedValue(10 * MB);
});

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
  uploadAttachment?: (file: File) => Promise<UploadedAttachment>;
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
    const uploadAttachment = vi.fn().mockResolvedValue({
      path: "Attachments/pasted.png",
      embed: "![[Attachments/pasted.png]]",
    });
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
    const uploadAttachment = vi.fn().mockResolvedValue({
      path: "Attachments/safari-paste.png",
      embed: "![[Attachments/safari-paste.png]]",
    });
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
    const uploadAttachment = vi.fn().mockResolvedValue({
      path: "Attachments/report.pdf",
      embed: "![[Attachments/report.pdf]]",
    });

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

  it("uploads a file over the default limit when the configured limit allows it (#558)", async () => {
    mockedMaxBytes.mockResolvedValue(20 * MB);
    const uploadAttachment = vi.fn().mockResolvedValue({
      path: "Attachments/big.pdf",
      embed: "![[Attachments/big.pdf]]",
    });
    render(
      <FrontmatterHarness
        initialContent={"# Body\n"}
        onSaveContent={() => {}}
        uploadAttachment={uploadAttachment}
      />,
    );

    const textarea = screen.getByRole("textbox", { name: "Markdown content" });
    const file = pdfOfSize("big.pdf", 15 * MB);
    fireEvent.paste(textarea, { clipboardData: { files: [file] } });

    await screen.findByText("Inserted attachment: Attachments/big.pdf");
    expect(uploadAttachment).toHaveBeenCalledWith(file);
  });

  it("refuses a file over the configured limit before uploading, quoting that limit (#558)", async () => {
    mockedMaxBytes.mockResolvedValue(20 * MB);
    const uploadAttachment = vi.fn();
    render(
      <FrontmatterHarness
        initialContent={"# Body\n"}
        onSaveContent={() => {}}
        uploadAttachment={uploadAttachment}
      />,
    );

    const textarea = screen.getByRole("textbox", { name: "Markdown content" });
    fireEvent.paste(textarea, {
      clipboardData: { files: [pdfOfSize("huge.pdf", 25 * MB)] },
    });

    await screen.findByText("That file is 25 MB. The limit is 20 MB.");
    expect(uploadAttachment).not.toHaveBeenCalled();
  });

  it("sends an oversized file and shows the server's refusal when the limit is unknown (#558)", async () => {
    mockedMaxBytes.mockResolvedValue(null);
    const uploadAttachment = vi
      .fn()
      .mockRejectedValue(
        new Error("The file is larger than the upload limit."),
      );
    render(
      <FrontmatterHarness
        initialContent={"# Body\n"}
        onSaveContent={() => {}}
        uploadAttachment={uploadAttachment}
      />,
    );

    const textarea = screen.getByRole("textbox", { name: "Markdown content" });
    const file = pdfOfSize("huge.pdf", 25 * MB);
    fireEvent.paste(textarea, { clipboardData: { files: [file] } });

    await screen.findByText("The file is larger than the upload limit.");
    expect(uploadAttachment).toHaveBeenCalledWith(file);
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

    expect(
      screen.getByText("Drop an image or PDF to attach"),
    ).toBeInTheDocument();
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

  it("shows only the changed lines of a long note, with the rest folded (#331)", () => {
    const disk = Array.from({ length: 60 }, (_, index) => `line ${index + 1}`);
    const draft = ["new first line", ...disk];

    const { container } = render(
      <FrontmatterHarness
        initialContent={draft.join("\n")}
        onSaveContent={() => {}}
        conflictReview={{
          diskContent: disk.join("\n"),
          draftContent: draft.join("\n"),
          onUseDisk: vi.fn(),
          onKeepDraft: vi.fn(),
        }}
      />,
    );

    const rows = container.querySelectorAll(".note-editor-conflict-line");
    expect(rows).toHaveLength(5);
    expect(
      container.querySelectorAll(".note-editor-conflict-line.draft"),
    ).toHaveLength(1);
    expect(
      container.querySelectorAll(".note-editor-conflict-line.disk"),
    ).toHaveLength(0);
    expect(screen.getByText("57 unchanged lines")).toBeInTheDocument();
  });
});

describe("NoteEditor link style (ADR-33)", () => {
  function AutocompleteHarness({
    formatNoteLink,
  }: {
    formatNoteLink?: ComponentProps<typeof NoteEditor>["formatNoteLink"];
  }) {
    const [content, setContent] = useState("See ");
    return (
      <NoteEditor
        content={content}
        saving={false}
        error={null}
        noteCandidates={[
          { vault_id: "vault-1", slug: "project-plan", title: "Project Plan" },
        ]}
        formatNoteLink={formatNoteLink}
        onChange={setContent}
        onSave={() => {}}
        onCancel={() => {}}
      />
    );
  }

  function pickSuggestion() {
    const textarea = screen.getByRole("textbox", {
      name: "Markdown content",
    }) as HTMLTextAreaElement;
    const typed = "See [[Pro";
    fireEvent.change(textarea, { target: { value: typed } });
    textarea.setSelectionRange(typed.length, typed.length);
    fireEvent.select(textarea);
    fireEvent.mouseDown(screen.getByRole("option", { name: "Project Plan" }));
    return textarea;
  }

  it("inserts [[title]] when no formatter is given", () => {
    render(<AutocompleteHarness />);
    expect(pickSuggestion()).toHaveValue("See [[Project Plan]]");
  });

  it("inserts the link the Vault's style formats", () => {
    const formatNoteLink = vi.fn(() => "[Project Plan](Project%20Plan.md)");
    render(<AutocompleteHarness formatNoteLink={formatNoteLink} />);
    expect(pickSuggestion()).toHaveValue(
      "See [Project Plan](Project%20Plan.md)",
    );
    expect(formatNoteLink).toHaveBeenCalledWith(
      expect.objectContaining({ slug: "project-plan" }),
    );
  });

  it("inserts the embed the upload answers with, in the Vault's style", async () => {
    const uploadAttachment = vi.fn().mockResolvedValue({
      path: "Attachments/pasted.png",
      embed: "![](Attachments/pasted.png)",
    });
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
    const file = new File(["png"], "pasted.png", { type: "image/png" });
    fireEvent.paste(textarea, { clipboardData: { files: [file] } });

    await screen.findByText("Inserted attachment: Attachments/pasted.png");
    expect(textarea).toHaveValue("# Body\n![](Attachments/pasted.png)");
  });
});
