import { render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { markdown } from "@codemirror/lang-markdown";
import { syntaxTree, syntaxTreeAvailable } from "@codemirror/language";
import { EditorState } from "@codemirror/state";
import { EditorView, type Decoration } from "@codemirror/view";
import { Tree } from "@lezer/common";

import { treeSoFar } from "./focusState";
import { LiveEditor } from "./LiveEditor";

vi.mock("mermaid", () => ({
  default: {
    initialize: vi.fn(),
    render: vi.fn(async () => ({ svg: "<svg></svg>" })),
  },
}));

afterEach(() => {
  vi.restoreAllMocks();
});

// A long note opens before the parser reaches its end. Here a clock that
// jumps 10 ms each time it is read makes the layers' 50 ms parse run out
// on a note of any size, as a long enough note does in a browser (#783).
function slowClock() {
  let now = Date.now();
  return vi.spyOn(Date, "now").mockImplementation(() => (now += 10));
}

function filler(paragraphs: number): string {
  return Array.from(
    { length: paragraphs },
    (_, n) => `Paragraph ${n} of filler text.\n`,
  ).join("\n");
}

function mountWithSlowParse(value: string) {
  const clock = slowClock();
  render(
    <MemoryRouter>
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
    </MemoryRouter>,
  );
  clock.mockRestore();
  const content = screen.getByRole("textbox", { name: "Note body" });
  const view = EditorView.findFromDOM(content as HTMLElement);
  if (!view) {
    throw new Error("no editor view mounted");
  }
  return view;
}

/** Every decoration the editor holds from `from` on, whatever its layer. */
function decorationsFrom(view: EditorView, from: number): Decoration[] {
  const found: Decoration[] = [];
  for (const source of view.state.facet(EditorView.decorations)) {
    const set = typeof source === "function" ? source(view) : source;
    set.between(from, view.state.doc.length, (_from, _to, decoration) => {
      found.push(decoration);
    });
  }
  return found;
}

describe("the tree the layers read (#783)", () => {
  it("is what is parsed so far when the parse runs out, the same for every layer", () => {
    const clock = slowClock();
    const state = EditorState.create({
      doc: filler(3000),
      extensions: markdown(),
    });
    const tree = treeSoFar(state);
    const readsAfterFirst = clock.mock.calls.length;
    expect(tree).toBeInstanceOf(Tree);
    expect(tree).toBe(syntaxTree(state));
    expect(syntaxTreeAvailable(state)).toBe(false);
    // A second layer on the same state spends no parse time of its own.
    expect(treeSoFar(state)).toBe(tree);
    expect(clock.mock.calls.length).toBe(readsAfterFirst);
  });
});

describe("layers in a note longer than the first parse (#783)", () => {
  it("draws a callout, a diagram and an image past the first parse once parsing goes on", async () => {
    const head = filler(3000);
    const tail = [
      "> [!note] Last callout",
      "> Its body.",
      "",
      "```mermaid",
      "graph TD; A-->B",
      "```",
      "",
      "![Last image](pictures/last.png)",
      "",
    ].join("\n");
    const view = mountWithSlowParse(`${head}\n${tail}`);
    const tailFrom = head.length;
    expect(syntaxTreeAvailable(view.state)).toBe(false);

    await waitFor(
      () => {
        const specs = decorationsFrom(view, tailFrom).map((d) => d.spec);
        expect(
          specs.some((spec) =>
            String(spec.class ?? "").includes("callout-note"),
          ),
        ).toBe(true);
        expect(
          specs.some((spec) =>
            String(spec.widget?.className ?? "").includes(
              "live-editor-mermaid",
            ),
          ),
        ).toBe(true);
        expect(
          specs.some(
            (spec) => spec.widget?.src === "/assets/pictures/last.png",
          ),
        ).toBe(true);
      },
      { timeout: 10000 },
    );
  });
});
