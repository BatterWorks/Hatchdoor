import { describe, expect, it, vi } from "vitest";

import {
  ATTACHMENT_MAX_BYTES,
  attachmentEmbedPath,
  attachmentRejection,
  insertEmbedAt,
  insertionLineForDrop,
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
      vault_id: "vault-1",
      attachment: { relative_path: "Attachments/report.pdf", layer: null },
    });

    await uploadNoteAttachment(pdfFile(), "Projects/Foo.md", upload);

    expect(upload).toHaveBeenCalledWith(
      expect.any(File),
      "Attachments/report.pdf",
    );
  });

  it("returns an embed path that resolves from a note in a subfolder", async () => {
    const upload = vi.fn().mockResolvedValue({
      vault_id: "vault-1",
      attachment: { relative_path: "Attachments/report.pdf", layer: null },
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
      vault_id: "vault-1",
      attachment: { relative_path: "Attachments/my-report.pdf", layer: null },
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
        vault_id: "vault-1",
        attachment: { relative_path: "Attachments/report-1.pdf", layer: null },
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
        vault_id: "vault-1",
        attachment: { relative_path: "Attachments/report-2.pdf", layer: null },
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
});

describe("insertionLineForDrop", () => {
  const blocks = [
    { startLine: 1, endLine: 1, top: 0, bottom: 20 },
    { startLine: 3, endLine: 5, top: 20, bottom: 80 },
    { startLine: 7, endLine: 7, top: 80, bottom: 100 },
  ];

  it("inserts after the block the drop lands on", () => {
    expect(insertionLineForDrop(blocks, 50)).toBe(5);
  });

  it("inserts after the first block when dropped on it", () => {
    expect(insertionLineForDrop(blocks, 10)).toBe(1);
  });

  it("inserts after the last block when dropped below everything", () => {
    expect(insertionLineForDrop(blocks, 500)).toBe(7);
  });

  it("inserts before the first block when dropped above everything", () => {
    expect(insertionLineForDrop(blocks, -50)).toBe(0);
  });

  // DOM order and source order diverge: a footnote definition renders in a
  // generated section at the end while carrying the position where it was
  // written.
  it("uses source order, not the order the blocks were given in", () => {
    const outOfOrder = [
      { startLine: 9, endLine: 9, top: 0, bottom: 20 },
      { startLine: 2, endLine: 2, top: 20, bottom: 40 },
    ];

    expect(insertionLineForDrop(outOfOrder, 30)).toBe(2);
  });

  it("returns 0 when there are no blocks at all", () => {
    expect(insertionLineForDrop([], 10)).toBe(0);
  });
});

describe("insertEmbedAt", () => {
  it("inserts an embed on its own line after the given line", () => {
    expect(insertEmbedAt("one\ntwo", 1, "Attachments/a.png")).toBe(
      "one\n\n![[Attachments/a.png]]\n\ntwo",
    );
  });

  it("inserts at the top when the line is 0", () => {
    expect(insertEmbedAt("one", 0, "a.png")).toBe("![[a.png]]\n\none");
  });

  it("appends at the end of the document", () => {
    expect(insertEmbedAt("one\ntwo", 2, "a.png")).toBe(
      "one\ntwo\n\n![[a.png]]",
    );
  });

  it("preserves CRLF line endings", () => {
    expect(insertEmbedAt("one\r\ntwo", 1, "a.png")).toBe(
      "one\r\n\r\n![[a.png]]\r\n\r\ntwo",
    );
  });

  it("does not double up a blank line that is already there", () => {
    expect(insertEmbedAt("one\n\ntwo", 1, "a.png")).toBe(
      "one\n\n![[a.png]]\n\ntwo",
    );
  });

  it("keeps the file's trailing newline", () => {
    expect(insertEmbedAt("one\n", 1, "a.png")).toBe("one\n\n![[a.png]]\n");
  });
});
