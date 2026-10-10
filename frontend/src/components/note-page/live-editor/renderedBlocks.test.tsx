import { act, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { EditorView } from "@codemirror/view";

import { SavedQueryProvider } from "../SavedQueryBlock";
import { LiveEditor } from "./LiveEditor";

const mocks = vi.hoisted(() => ({
  mermaidRender: vi.fn(async (id: string, chart: string) => ({
    svg: `<svg id="${id}" data-chart="${chart.trim()}"></svg>`,
  })),
  getDocument: vi.fn(() => ({
    promise: new Promise(() => {}),
    destroy: vi.fn(),
  })),
}));

vi.mock("mermaid", () => ({
  default: { initialize: vi.fn(), render: mocks.mermaidRender },
}));

vi.mock("pdfjs-dist", () => ({
  getDocument: mocks.getDocument,
  GlobalWorkerOptions: { workerSrc: "" },
}));

function mount(
  value: string,
  wrap?: (editor: React.ReactNode) => React.ReactNode,
) {
  const editor = (
    <LiveEditor
      value={value}
      searchQuery=""
      touch={false}
      noteCandidates={[]}
      formatNoteLink={(note) => `[[${note.title}]]`}
      onOpenNote={vi.fn()}
      resolveNote={async () => null}
      resolveAssetSrc={(raw) => `/assets/${raw}`}
      onChange={vi.fn()}
      onCommit={vi.fn()}
    />
  );
  const utils = render(
    <MemoryRouter>{wrap ? wrap(editor) : editor}</MemoryRouter>,
  );
  const content = screen.getByRole("textbox", { name: "Note body" });
  const view = EditorView.findFromDOM(content as HTMLElement);
  if (!view) {
    throw new Error("no editor view mounted");
  }
  return { ...utils, view, content };
}

/** The text of every line CodeMirror currently draws. */
function drawnLines(): string[] {
  return Array.from(document.querySelectorAll(".cm-line")).map(
    (line) => line.textContent ?? "",
  );
}

/** Puts the caret at `pos` with the editor focused, as a click would. */
async function placeCaret(view: EditorView, pos: number) {
  act(() => {
    view.focus();
    view.dispatch({ selection: { anchor: pos } });
  });
  // CodeMirror notices focus a tick later.
  await waitFor(() => expect(view.hasFocus).toBe(true));
}

afterEach(() => {
  mocks.mermaidRender.mockClear();
});

describe("rendered blocks in the live editor (#544)", () => {
  it("draws a mermaid fence as its diagram and shows the source under the caret", async () => {
    const value = "Before\n\n```mermaid\ngraph TD; A-->B\n```\n\nAfter\n";
    const { view } = mount(value);

    const diagram = await screen.findByTestId("live-editor-mermaid");
    await waitFor(() =>
      expect(diagram.querySelector("svg")).toHaveAttribute(
        "data-chart",
        "graph TD; A-->B",
      ),
    );
    expect(drawnLines()).not.toContain("```mermaid");

    await placeCaret(view, value.indexOf("graph"));
    await waitFor(() => expect(drawnLines()).toContain("```mermaid"));
    expect(screen.queryByTestId("live-editor-mermaid")).toBeNull();

    // Leaving the editor renders it again, whatever line the caret is on.
    act(() => view.contentDOM.blur());
    await waitFor(() =>
      expect(screen.getByTestId("live-editor-mermaid")).toBeInTheDocument(),
    );
  });

  it("shows the reading view's error treatment for a diagram that will not render", async () => {
    mocks.mermaidRender.mockRejectedValueOnce(
      new Error("Parse error on line 2"),
    );
    mount("```mermaid\ngraph TD; A-->\n```\n");
    const diagram = await screen.findByTestId("live-editor-mermaid");
    await waitFor(() =>
      expect(diagram.querySelector("pre.error")).toHaveTextContent(
        "Mermaid error: Parse error on line 2",
      ),
    );
  });

  it("leaves math and embeds inside a code span as the code they are", async () => {
    mount("Write `$x$` or `![[a.pdf]]` literally, but $y$ renders.\n");
    await screen.findByTestId("live-editor-math-inline");
    expect(screen.getAllByTestId("live-editor-math-inline")).toHaveLength(1);
    expect(screen.queryByTestId("live-editor-pdf")).toBeNull();
  });

  it("leaves math and embeds inside a code span as the code they are", async () => {
    mount("Write `$x$` or `![[a.pdf]]` literally, but $y$ renders.\n");
    await screen.findByTestId("live-editor-math-inline");
    expect(screen.getAllByTestId("live-editor-math-inline")).toHaveLength(1);
    expect(screen.queryByTestId("live-editor-pdf")).toBeNull();
  });

  it("draws display and inline math through KaTeX", async () => {
    const value = "Text with $x^2$ inline.\n\n$$\n\\frac{a}{b}\n$$\n";
    mount(value);
    const inline = await screen.findByTestId("live-editor-math-inline");
    expect(inline.querySelector(".katex")).not.toBeNull();
    const block = screen.getByTestId("live-editor-math");
    expect(block.querySelector(".katex-display")).not.toBeNull();
    expect(drawnLines()).not.toContain("$$");
  });

  it("shows a base fence as its saved-query table when results are provided", async () => {
    const value = "```base\nfilter: tag = x\n```\n";
    mount(value, (editor) => (
      <SavedQueryProvider
        vaultId="vault-1"
        markdown={value}
        state={{
          status: "ready",
          markerProblems: [],
          results: [
            {
              source: "filter: tag = x",
              status: "populated",
              columns: [{ id: "file.name", label: "Name" }],
              rows: [
                {
                  vault_id: "vault-1",
                  title: "Alpha",
                  slug: "alpha",
                  relative_path: "Alpha.md",
                  cells: ["Alpha.md"],
                },
              ],
            },
          ],
        }}
      >
        {editor}
      </SavedQueryProvider>
    ));
    expect(await screen.findByRole("link", { name: "Alpha" })).toHaveAttribute(
      "href",
      "/v/vault-1/n/alpha",
    );
    expect(drawnLines()).not.toContain("```base");
  });

  it("draws a PDF embed under its line and shows the path under the caret", async () => {
    const value = "See ![[docs/report.pdf]] here.\n";
    const { view } = mount(value);
    expect(await screen.findByText("Loading PDF preview…")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Open PDF" })).toHaveAttribute(
      "href",
      "/assets/docs/report.pdf",
    );
    // Like an image, the embed is hidden on its line until the caret comes.
    expect(drawnLines()[0]).toBe("See  here.");
    await placeCaret(view, 6);
    await waitFor(() => expect(drawnLines()[0]).toContain("report.pdf"));
    // The preview stays: the path is one thing the typist may be fixing.
    expect(screen.getByText("Loading PDF preview…")).toBeInTheDocument();
  });
});

describe("embeds follow the page's asset resolution (#544)", () => {
  it("draws every embed again when the page says the resolver knows more", async () => {
    let known = false;
    const resolve = (raw: string) =>
      known ? `/resolved/${raw}` : `/relative/${raw}`;
    const value = "![[shot.png]]\n\n![[report.pdf]]\n";
    const editor = (settled: string) => (
      <LiveEditor
        value={value}
        searchQuery=""
        touch={false}
        noteCandidates={[]}
        formatNoteLink={(note) => `[[${note.title}]]`}
        onOpenNote={vi.fn()}
        resolveNote={async () => null}
        resolveAssetSrc={resolve}
        assetsResolvedFor={settled}
        onChange={vi.fn()}
        onCommit={vi.fn()}
      />
    );
    const { rerender } = render(<MemoryRouter>{editor("")}</MemoryRouter>);
    expect(document.querySelector(".live-editor-image img")).toHaveAttribute(
      "src",
      "/relative/shot.png",
    );
    expect(
      await screen.findByRole("link", { name: "Open PDF" }),
    ).toHaveAttribute("href", "/relative/report.pdf");

    known = true;
    rerender(<MemoryRouter>{editor(value)}</MemoryRouter>);
    expect(document.querySelector(".live-editor-image img")).toHaveAttribute(
      "src",
      "/resolved/shot.png",
    );
    await waitFor(() =>
      expect(screen.getByRole("link", { name: "Open PDF" })).toHaveAttribute(
        "href",
        "/resolved/report.pdf",
      ),
    );
  });
});

describe("callouts in the live editor (#544)", () => {
  it("marks a callout's lines with its kind and draws the title in place of the marker", async () => {
    const value = "> [!tip]\n> Keep going.\n\nPlain.\n";
    const { view } = mount(value);
    const lines = document.querySelectorAll(".cm-line");
    expect(lines[0]).toHaveClass("live-editor-callout", "callout-tip");
    expect(lines[1]).toHaveClass("live-editor-callout", "callout-tip");
    expect(lines[3]).not.toHaveClass("live-editor-callout");
    expect(lines[0].textContent).toBe("Tip");

    await placeCaret(view, 3);
    await waitFor(() =>
      expect(document.querySelectorAll(".cm-line")[0].textContent).toContain(
        "[!tip]",
      ),
    );
  });

  it("keeps a custom title as the line's own text", () => {
    mount("> [!warning] Mind the gap\n> Body.\n");
    const first = document.querySelector(".cm-line");
    expect(first).toHaveClass("callout-warning");
    expect(first?.textContent).toBe("Mind the gap");
  });
});
