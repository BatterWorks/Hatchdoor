import { render, screen } from "@testing-library/react";
import ReactMarkdown from "react-markdown";
import { describe, expect, it, vi } from "vitest";

import { createNoteMarkdownComponents } from "./renderers";

vi.mock("./PdfPreview", () => ({
  PdfPreview: ({ src, label }: { src: string; label: string }) => (
    <div data-testid="pdf-preview" data-src={src}>
      {label}
    </div>
  ),
}));

function renderMarkdown(markdown: string) {
  render(
    <ReactMarkdown
      components={createNoteMarkdownComponents("Notes/Entry", new Map())}
    >
      {markdown}
    </ReactMarkdown>,
  );
}

describe("note markdown asset renderer", () => {
  it("renders embedded PDFs with the cross-platform PDF preview", () => {
    renderMarkdown("![Quarterly report](Attachments/report.PDF#page=2)");

    expect(screen.getByTestId("pdf-preview")).toHaveAttribute(
      "data-src",
      "/vault-assets/Notes/Attachments/report.PDF#page=2",
    );
    expect(screen.getByTestId("pdf-preview")).toHaveTextContent(
      "Quarterly report",
    );
  });

  it("keeps non-PDF embeds as lazy images", () => {
    renderMarkdown("![Diagram](Attachments/diagram.png)");

    const image = screen.getByRole("img", { name: "Diagram" });
    expect(image).toHaveAttribute(
      "src",
      "/vault-assets/Notes/Attachments/diagram.png",
    );
    expect(image).toHaveAttribute("loading", "lazy");
  });
});

describe("block embeds inside paragraphs", () => {
  // A PDF embed on its own line is parsed as a paragraph containing an image.
  // PdfPreview renders a div (and its loading state renders a p), so leaving
  // the paragraph in place produces invalid nesting: React logs
  // "<p> cannot contain a nested <p>" and the browser silently splits the
  // paragraph, detaching the preview from its own container.
  it("does not wrap a PDF preview in a paragraph", () => {
    const { container } = render(
      <ReactMarkdown
        components={createNoteMarkdownComponents("Notes/Entry", new Map())}
      >
        {"![Report](Attachments/report.pdf)"}
      </ReactMarkdown>,
    );

    expect(container.querySelector("p [data-testid='pdf-preview']")).toBeNull();
    expect(
      container.querySelector("[data-testid='pdf-preview']"),
    ).not.toBeNull();
  });

  it("keeps an ordinary paragraph wrapping its text", () => {
    const { container } = render(
      <ReactMarkdown
        components={createNoteMarkdownComponents("Notes/Entry", new Map())}
      >
        {"Just prose here."}
      </ReactMarkdown>,
    );

    expect(container.querySelector("p")?.textContent).toBe("Just prose here.");
  });

  it("keeps a paragraph that mixes text with an inline image", () => {
    const { container } = render(
      <ReactMarkdown
        components={createNoteMarkdownComponents("Notes/Entry", new Map())}
      >
        {"Before ![Diagram](Attachments/diagram.png) after"}
      </ReactMarkdown>,
    );

    expect(container.querySelector("p")).not.toBeNull();
    expect(container.querySelector("p img")).not.toBeNull();
  });
});
