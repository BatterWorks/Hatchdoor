import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
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

  it("renders at 2x backing resolution on a 1x display", async () => {
    const renderPage = vi.fn(() => ({ promise: Promise.resolve(), cancel: vi.fn() }));
    const page = {
      getViewport: ({ scale }: { scale: number }) => ({
        width: 600 * scale,
        height: 800 * scale,
      }),
      render: renderPage,
      cleanup: vi.fn(),
    };
    mocks.getDocument.mockReturnValue({
      promise: Promise.resolve({
        numPages: 1,
        getPage: vi.fn().mockResolvedValue(page),
        destroy: vi.fn(),
      }),
      destroy: vi.fn(),
    });
    vi.stubGlobal("ResizeObserver", undefined);
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
      {} as CanvasRenderingContext2D,
    );
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get() {
        return this.classList.contains("pdf-preview") ? 300 : 0;
      },
    });
    Object.defineProperty(window, "devicePixelRatio", {
      configurable: true,
      value: 1,
    });

    const { container } = render(<PdfPreview src="/vault-assets/sample.pdf" label="Sample" />);

    await waitFor(() => expect(renderPage).toHaveBeenCalledOnce());

    const canvas = container.querySelector("canvas");
    expect(canvas).toHaveAttribute("style", expect.stringContaining("width: 300px"));
    expect(canvas).toHaveProperty("width", 600);
    expect(canvas).toHaveProperty("height", 800);
  });
});
