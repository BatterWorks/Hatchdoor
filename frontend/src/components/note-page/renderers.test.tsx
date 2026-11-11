import { render, screen } from "@testing-library/react";
import ReactMarkdown from "react-markdown";
import { describe, expect, it, vi } from "vitest";

import { NOTE_SYNTAX_PLUGINS } from "../../lib/markdownSyntax";
import {
  createManualMarkdownComponents,
  createNoteMarkdownComponents,
} from "./renderers";

vi.mock("./PdfPreview", () => ({
  PdfPreview: ({ src, label }: { src: string; label: string }) => (
    <div data-testid="pdf-preview" data-src={src}>
      {label}
    </div>
  ),
}));

const VAULT_ID = "vault-1";

function renderMarkdown(markdown: string) {
  render(
    <ReactMarkdown
      components={createNoteMarkdownComponents(
        VAULT_ID,
        "Notes/Entry",
        new Map(),
      )}
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
      "/api/v1/vaults/vault-1/assets/Notes/Attachments/report.PDF#page=2",
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
      "/api/v1/vaults/vault-1/assets/Notes/Attachments/diagram.png",
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
        components={createNoteMarkdownComponents(
          VAULT_ID,
          "Notes/Entry",
          new Map(),
        )}
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
        components={createNoteMarkdownComponents(
          VAULT_ID,
          "Notes/Entry",
          new Map(),
        )}
      >
        {"Just prose here."}
      </ReactMarkdown>,
    );

    expect(container.querySelector("p")?.textContent).toBe("Just prose here.");
  });

  it("keeps a paragraph that mixes text with an inline image", () => {
    const { container } = render(
      <ReactMarkdown
        components={createNoteMarkdownComponents(
          VAULT_ID,
          "Notes/Entry",
          new Map(),
        )}
      >
        {"Before ![Diagram](Attachments/diagram.png) after"}
      </ReactMarkdown>,
    );

    expect(container.querySelector("p")).not.toBeNull();
    expect(container.querySelector("p img")).not.toBeNull();
  });
});

describe("block anchors in the note renderers (#578)", () => {
  function renderNote(markdown: string): HTMLElement {
    return render(
      <ReactMarkdown
        remarkPlugins={NOTE_SYNTAX_PLUGINS}
        components={createNoteMarkdownComponents(
          VAULT_ID,
          "Notes/Entry",
          new Map(),
        )}
      >
        {markdown}
      </ReactMarkdown>,
    ).container;
  }

  it("keeps a heading and a block that share a name as two elements", () => {
    renderNote("## Notes\n\nA paragraph. ^notes\n");

    expect(document.getElementById("notes")?.tagName).toBe("H2");
    const block = document.getElementById("^notes");
    expect(block?.tagName).toBe("P");
    expect(block?.textContent).toBe("A paragraph.");
  });

  it.each([
    ["a list item", "- one\n- two ^abc", "LI"],
    ["a plain quote", "> Quoted. ^abc", "BLOCKQUOTE"],
    ["a callout", "> [!note] Title\n> Body. ^abc", "DIV"],
    ["a folding callout", "> [!tip]- Title\n> Body. ^abc", "DETAILS"],
    ["a pull quote", "> [!quote] Someone\n> Said. ^abc", "FIGURE"],
    ["a table", "| a |\n| - |\n| b |\n\n^abc", "DIV"],
    [
      "a line of a callout's first paragraph",
      "> [!note] Title\n> Step. ^abc\n>\n> More.",
      "P",
    ],
    [
      "a callout that is only a title",
      "> [!note] Title ^abc\n>\n> More.",
      "DIV",
    ],
  ])("anchors %s on the element it draws", (_, markdown, tag) => {
    const container = renderNote(markdown);

    const anchored = container.querySelectorAll("[id='^abc']");
    expect(anchored).toHaveLength(1);
    expect(anchored[0].tagName).toBe(tag);
    expect(container.textContent).not.toContain("^abc");
  });

  it("lets the manual choose the ids its blocks get", () => {
    const { container } = render(
      <ReactMarkdown
        remarkPlugins={NOTE_SYNTAX_PLUGINS}
        components={createManualMarkdownComponents(
          new Map(),
          (href, children) => (
            <a href={href}>{children}</a>
          ),
          (id) => (id ? `help-${id}` : undefined),
        )}
      >
        {"A paragraph. ^abc\n\n- item ^def\n\nPlain.\n"}
      </ReactMarkdown>,
    );

    expect(
      Array.from(container.querySelectorAll("[id]")).map((el) => el.id),
    ).toEqual(["help-^abc", "help-^def"]);
  });
});
