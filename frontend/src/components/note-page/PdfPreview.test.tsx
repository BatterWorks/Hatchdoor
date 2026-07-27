import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { PdfPreview } from "./PdfPreview";

const mocks = vi.hoisted(() => ({
  getDocument: vi.fn(),
  workerOptions: { workerSrc: "" },
}));

vi.mock("pdfjs-dist", () => ({
  getDocument: mocks.getDocument,
  GlobalWorkerOptions: mocks.workerOptions,
}));

describe("PdfPreview", () => {
  beforeEach(() => {
    mocks.getDocument.mockReset();
    mocks.workerOptions.workerSrc = "";
  });

  it("shows an accessible loading state and retains a direct-open fallback", () => {
    mocks.getDocument.mockReturnValue({
      promise: new Promise(() => {}),
      destroy: vi.fn(),
    });

    render(
      <PdfPreview src="/vault-assets/report.pdf" label="Quarterly report" />,
    );

    expect(screen.getByRole("status")).toHaveTextContent("Loading PDF preview");
    expect(screen.getByRole("link", { name: "Open PDF" })).toHaveAttribute(
      "href",
      "/vault-assets/report.pdf",
    );
  });

  it("offers a direct-open fallback when PDF.js cannot load the file", async () => {
    mocks.getDocument.mockReturnValue({
      promise: Promise.reject(new Error("bad PDF")),
      destroy: vi.fn(),
    });

    render(<PdfPreview src="/vault-assets/broken.pdf" label="Broken report" />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "This PDF could not be previewed.",
    );
    expect(
      screen.getByRole("link", { name: "Open the PDF instead." }),
    ).toHaveAttribute("href", "/vault-assets/broken.pdf");
  });

  it("cancels an in-flight PDF request when the source changes", async () => {
    const firstTask = {
      promise: new Promise(() => {}),
      destroy: vi.fn(),
    };
    const secondTask = {
      promise: new Promise(() => {}),
      destroy: vi.fn(),
    };
    mocks.getDocument
      .mockReturnValueOnce(firstTask)
      .mockReturnValueOnce(secondTask);

    const { rerender } = render(
      <PdfPreview src="/vault-assets/first.pdf" label="First report" />,
    );
    await waitFor(() => expect(mocks.getDocument).toHaveBeenCalledTimes(1));

    rerender(
      <PdfPreview src="/vault-assets/second.pdf" label="Second report" />,
    );

    expect(firstTask.destroy).toHaveBeenCalledOnce();
  });
});
