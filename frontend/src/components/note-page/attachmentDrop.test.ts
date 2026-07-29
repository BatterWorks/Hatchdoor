import { describe, expect, it, vi } from "vitest";

import {
  ATTACHMENT_MAX_BYTES,
  attachmentEmbedPath,
  attachmentRejection,
  uploadNoteAttachment,
} from "./attachmentDrop";

describe("attachmentEmbedPath", () => {
  it("leaves the path alone for a note at the vault root", () => {
    expect(attachmentEmbedPath("Attachments/report.pdf", "Home.md")).toBe(
      "Attachments/report.pdf",
    );
  });

  it("walks back out of the note's folder so the embed resolves to the vault root", () => {
    // Rendering resolves an embed relative to the note's directory, so a bare
    // "Attachments/report.pdf" inside Projects/Foo.md would look for
    // Projects/Attachments/report.pdf and 404.
    expect(
      attachmentEmbedPath("Attachments/report.pdf", "Projects/Foo.md"),
    ).toBe("../Attachments/report.pdf");
  });

  it("walks back out of every level of nesting", () => {
    expect(
      attachmentEmbedPath("Attachments/report.pdf", "Projects/2026/Q3/Foo.md"),
    ).toBe("../../../Attachments/report.pdf");
  });
});

function pdfFile(name = "report.pdf") {
  return new File(["%PDF-1.7"], name, { type: "application/pdf" });
}

function fileOfSize(name: string, bytes: number, type = "application/pdf") {
  const file = new File(["x"], name, { type });
  Object.defineProperty(file, "size", { value: bytes });
  return file;
}

describe("attachmentRejection", () => {
  it("accepts every extension the vault accepts", () => {
    for (const ext of [
      "png",
      "jpg",
      "jpeg",
      "gif",
      "webp",
      "avif",
      "bmp",
      "pdf",
    ]) {
      expect(attachmentRejection(pdfFile(`file.${ext}`))).toBeNull();
    }
  });

  it("accepts an uppercase extension", () => {
    expect(attachmentRejection(pdfFile("REPORT.PDF"))).toBeNull();
  });

  it("names what it accepts when the extension is not on the list", () => {
    expect(attachmentRejection(pdfFile("notes.docx"))).toBe(
      "Hatchdoor accepts images and PDFs.",
    );
  });

  it("rejects a file with no extension", () => {
    expect(attachmentRejection(pdfFile("report"))).toBe(
      "Hatchdoor accepts images and PDFs.",
    );
  });

  it("reports both sizes when the file is over the limit", () => {
    const file = fileOfSize("big.pdf", 14 * 1024 * 1024);

    expect(attachmentRejection(file)).toBe(
      "That file is 14 MB. The limit is 10 MB.",
    );
  });

  it("accepts a file exactly at the limit", () => {
    expect(
      attachmentRejection(fileOfSize("edge.pdf", ATTACHMENT_MAX_BYTES)),
    ).toBeNull();
  });
});

describe("uploadNoteAttachment", () => {
  it("uploads to the vault-root Attachments folder", async () => {
    const upload = vi.fn().mockResolvedValue({
      attachment: { relative_path: "Attachments/report.pdf" },
    });

    await uploadNoteAttachment(pdfFile(), "Projects/Foo.md", upload);

    expect(upload).toHaveBeenCalledWith(
      expect.any(File),
      "Attachments/report.pdf",
    );
  });

  it("returns an embed path that resolves from a note in a subfolder", async () => {
    const upload = vi.fn().mockResolvedValue({
      attachment: { relative_path: "Attachments/report.pdf" },
    });

    const result = await uploadNoteAttachment(
      pdfFile(),
      "Projects/Foo.md",
      upload,
    );

    expect(result.embedPath).toBe("../Attachments/report.pdf");
  });

  it("strips characters the vault will not accept from the filename", async () => {
    const upload = vi.fn().mockResolvedValue({
      attachment: { relative_path: "Attachments/my-report.pdf" },
    });

    await uploadNoteAttachment(pdfFile("my:report.pdf"), "Home.md", upload);

    expect(upload).toHaveBeenCalledWith(
      expect.any(File),
      "Attachments/my-report.pdf",
    );
  });

  it("retries with a numbered filename when the name is already taken", async () => {
    const conflict = new Error("attachment already exists");
    conflict.name = "ConflictError";
    const upload = vi
      .fn()
      .mockRejectedValueOnce(conflict)
      .mockResolvedValue({
        attachment: { relative_path: "Attachments/report-1.pdf" },
      });

    const result = await uploadNoteAttachment(pdfFile(), "Home.md", upload);

    expect(upload).toHaveBeenNthCalledWith(
      1,
      expect.any(File),
      "Attachments/report.pdf",
    );
    expect(upload).toHaveBeenNthCalledWith(
      2,
      expect.any(File),
      "Attachments/report-1.pdf",
    );
    expect(result.embedPath).toBe("Attachments/report-1.pdf");
  });

  it("keeps counting up while names stay taken", async () => {
    const conflict = new Error("attachment already exists");
    conflict.name = "ConflictError";
    const upload = vi
      .fn()
      .mockRejectedValueOnce(conflict)
      .mockRejectedValueOnce(conflict)
      .mockResolvedValue({
        attachment: { relative_path: "Attachments/report-2.pdf" },
      });

    await uploadNoteAttachment(pdfFile(), "Home.md", upload);

    expect(upload).toHaveBeenNthCalledWith(
      3,
      expect.any(File),
      "Attachments/report-2.pdf",
    );
  });

  it("does not retry an error that is not a conflict", async () => {
    const failure = new Error("vault is read-only");
    failure.name = "WriteApiError";
    const upload = vi.fn().mockRejectedValue(failure);

    await expect(
      uploadNoteAttachment(pdfFile(), "Home.md", upload),
    ).rejects.toThrow("vault is read-only");
    expect(upload).toHaveBeenCalledTimes(1);
  });

  it("surfaces the git sync warning when the server sends one", async () => {
    const upload = vi.fn().mockResolvedValue({
      attachment: { relative_path: "Attachments/report.pdf" },
      git_sync_warning: "remote rejected",
    });

    const result = await uploadNoteAttachment(pdfFile(), "Home.md", upload);

    expect(result.gitSyncWarning).toBe("remote rejected");
  });
});
