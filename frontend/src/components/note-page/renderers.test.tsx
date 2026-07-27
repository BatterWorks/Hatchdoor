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
