import { useEffect, useRef, useState } from "react";

type PdfDocument = {
  numPages: number;
  getPage: (pageNumber: number) => Promise<PdfPage>;
  destroy: () => void;
};

type PdfPage = {
  getViewport: (params: { scale: number }) => { width: number; height: number };
  render: (params: {
    canvas: HTMLCanvasElement;
    canvasContext: CanvasRenderingContext2D;
    viewport: { width: number; height: number };
  }) => { promise: Promise<void>; cancel: () => void };
  cleanup: () => void;
};

type PdfLoadingTask = {
  promise: Promise<PdfDocument>;
  destroy: () => Promise<void>;
};

type PdfJs = {
  GlobalWorkerOptions: { workerSrc: string };
  getDocument: (params: { url: string }) => PdfLoadingTask;
};

async function loadPdfJs(): Promise<PdfJs> {
  const pdfjs = (await import("pdfjs-dist")) as unknown as PdfJs;
  pdfjs.GlobalWorkerOptions.workerSrc = new URL(
    "pdfjs-dist/build/pdf.worker.min.mjs",
    import.meta.url,
  ).toString();
  return pdfjs;
}

export function PdfPreview({ src, label }: { src: string; label: string }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [document, setDocument] = useState<PdfDocument | null>(null);
  const [pageNumber, setPageNumber] = useState(1);
  const [width, setWidth] = useState(0);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    let task: PdfLoadingTask | null = null;

    setDocument(null);
    setPageNumber(1);
    setError(null);

    void loadPdfJs()
      .then((pdfjs) => {
        if (disposed) {
          return;
        }
        task = pdfjs.getDocument({ url: src });
        return task.promise;
      })
      .then((nextDocument) => {
        if (!nextDocument) {
          return;
        }
        if (disposed) {
          nextDocument.destroy();
          return;
        }
        setDocument(nextDocument);
      })
      .catch(() => {
        if (!disposed) {
          setError("This PDF could not be previewed.");
        }
      });

    return () => {
      disposed = true;
      if (task) {
        void task.destroy();
      }
    };
  }, [src]);

  useEffect(() => {
    const element = containerRef.current;
    if (!element) {
      return;
    }
    const updateWidth = () => setWidth(element.clientWidth);
    updateWidth();

    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", updateWidth);
      return () => window.removeEventListener("resize", updateWidth);
    }
    const observer = new ResizeObserver(updateWidth);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!document || !canvas || width <= 0) {
      return;
    }

    let cancelled = false;
    let renderTask: {
      promise: Promise<void>;
      cancel: () => void;
    } | null = null;
    void document
      .getPage(pageNumber)
      .then((page) => {
        if (cancelled) {
          page.cleanup();
          return;
        }
        const naturalViewport = page.getViewport({ scale: 1 });
        const deviceScale = window.devicePixelRatio || 1;
        const scale =
          (Math.min(width, 960) / naturalViewport.width) * deviceScale;
        const viewport = page.getViewport({ scale });
        canvas.width = Math.ceil(viewport.width);
        canvas.height = Math.ceil(viewport.height);
        canvas.style.width = `${Math.ceil(viewport.width / deviceScale)}px`;
        canvas.style.height = `${Math.ceil(viewport.height / deviceScale)}px`;
        const context = canvas.getContext("2d");
        if (!context) {
          throw new Error("Canvas rendering is unavailable");
        }
        renderTask = page.render({ canvas, canvasContext: context, viewport });
        return renderTask.promise.finally(() => page.cleanup());
      })
      .catch((renderError: unknown) => {
        if (
          !cancelled &&
          !(
            renderError instanceof Error &&
            renderError.name === "RenderingCancelledException"
          )
        ) {
          setError("This PDF could not be previewed.");
        }
      });

    return () => {
      cancelled = true;
      renderTask?.cancel();
    };
  }, [document, pageNumber, width]);

  const totalPages = document?.numPages ?? 0;

  return (
    <div className="pdf-preview" ref={containerRef}>
      <div className="pdf-preview-toolbar">
        <span className="pdf-preview-title">{label || "PDF"}</span>
        <a href={src} target="_blank" rel="noopener noreferrer">
          Open PDF
        </a>
      </div>
      {error ? (
        <p className="pdf-preview-error" role="alert">
          {error} <a href={src}>Open the PDF instead.</a>
        </p>
      ) : document ? (
        <>
          <canvas
            ref={canvasRef}
            aria-label={`${label || "PDF"}, page ${pageNumber}`}
          />
          <div className="pdf-preview-controls" aria-label="PDF page controls">
            <button
              type="button"
              onClick={() => setPageNumber((current) => current - 1)}
              disabled={pageNumber === 1}
            >
              Previous
            </button>
            <span>
              Page {pageNumber} of {totalPages}
            </span>
            <button
              type="button"
              onClick={() => setPageNumber((current) => current + 1)}
              disabled={pageNumber === totalPages}
            >
              Next
            </button>
          </div>
        </>
      ) : (
        <p className="pdf-preview-loading" role="status">
          Loading PDF preview…
        </p>
      )}
    </div>
  );
}
