import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NOTE_PROPERTIES_COLLAPSED_KEY } from "../app/constants";
import keyboardBarSource from "./note-page/live-editor/KeyboardBar.tsx?raw";
import menusSource from "./note-page/live-editor/menus.ts?raw";
import { NotePage } from "./NotePage";
import { NotePreview } from "./note-page/NotePreview";

const VAULT = "vault-1";

// `==text==` highlights (#572), with the cases that must stay as written.
const CONTENT = [
  "Before ==these words== after",
  "",
  "Inline `a == b` code and $a == b$ math.",
  "",
  "```text",
  "==x==",
  "```",
  "",
  "==",
  "",
  "==open",
  "",
  "====",
  "",
  "==**bold** and [[Other note]]==",
  "",
  "Across ==two",
  "lines== stays.",
  "",
  "A ==fleet of boats== sails.",
  "",
].join("\n");

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function mockVault(content: string) {
  vi.spyOn(globalThis, "fetch").mockImplementation(
    async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes("/resolve-batch")) {
        const body = JSON.parse(String(init?.body ?? "{}")) as {
          targets: string[];
          asset_targets: string[];
        };
        return jsonResponse({
          vault_id: VAULT,
          results: body.targets.map((target) => ({
            target,
            slug: target === "Other note" ? "other-note" : null,
            archived: false,
          })),
          asset_results: (body.asset_targets ?? []).map((target) => ({
            target,
            path: null,
          })),
        });
      }
      if (url.includes("/notes/home")) {
        return jsonResponse({
          vault_id: VAULT,
          note: {
            title: "home",
            slug: "home",
            relative_path: "home.md",
            content,
            content_hash: "hash-home",
            layer: null,
          },
        });
      }
      return jsonResponse({ error: "not found" }, 404);
    },
  );
}

function renderReadingView(content: string, search = "") {
  mockVault(content);
  return render(
    <MemoryRouter initialEntries={[`/v/${VAULT}/n/home${search}`]}>
      <Routes>
        <Route
          path="/v/:vaultId/n/:slug"
          element={
            <NotePage
              onActiveNoteChange={vi.fn()}
              onTagSelect={vi.fn()}
              propertiesCollapsedStorageKey={NOTE_PROPERTIES_COLLAPSED_KEY}
              vaultRevision={null}
              writeEnabled={false}
              editRequestId={0}
              vaults={[]}
            />
          }
        />
      </Routes>
    </MemoryRouter>,
  );
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  window.localStorage.clear();
});

function paragraphStarting(body: HTMLElement, start: string) {
  return Array.from(body.querySelectorAll("p")).find((p) =>
    (p.textContent ?? "").startsWith(start),
  );
}

function expectHighlights(body: HTMLElement) {
  const marks = Array.from(body.querySelectorAll("mark.note-highlight"));
  expect(marks.map((mark) => mark.textContent)).toEqual([
    "these words",
    "bold and Other note",
    "two\nlines",
    "fleet of boats",
  ]);
  expect(paragraphStarting(body, "Before")?.textContent).toBe(
    "Before these words after",
  );

  // Code, a fenced block and math keep their equals signs.
  expect(body.querySelector("p code")?.textContent).toBe("a == b");
  expect(body.querySelector("pre")?.textContent).toContain("==x==");
  expect(paragraphStarting(body, "Inline")?.textContent).toContain("a == b");
  expect(body.querySelector("code mark, pre mark, .katex mark")).toBeNull();

  // A lone pair, an unclosed one and a run of four are text.
  const texts = Array.from(body.querySelectorAll("p")).map(
    (p) => p.textContent,
  );
  expect(texts).toContain("==");
  expect(texts).toContain("==open");
  expect(texts).toContain("====");

  // Bold and a wikilink still render inside a highlight.
  const rich = marks[1];
  expect(rich.querySelector("strong")?.textContent).toBe("bold");
  expect(rich.querySelector("a")?.getAttribute("href")).toBe(
    `/v/${VAULT}/n/other-note`,
  );

  // As in the editor, a highlight may run onto the next line of its
  // paragraph.
  expect(paragraphStarting(body, "Across")?.textContent).not.toContain("==");
}

describe("highlights (#572)", () => {
  it("draws ==text== as a mark in the Reading view", async () => {
    const { container } = renderReadingView(CONTENT);

    await screen.findByRole("link", { name: "Other note" });
    expectHighlights(container);
  });

  it("draws ==text== as a mark in the source preview", async () => {
    mockVault(CONTENT);
    const { container } = render(
      <MemoryRouter>
        <NotePreview vaultId={VAULT} content={CONTENT} relativePath="home.md" />
      </MemoryRouter>,
    );

    await screen.findByRole("link", { name: "Other note" });
    expectHighlights(container);
  });

  it("marks a search hit inside a highlight and makes it the active hit", async () => {
    const { container } = renderReadingView(CONTENT, "?q=fleet");

    await screen.findByRole("link", { name: "Other note" });
    await waitFor(() => {
      const hit = container.querySelector(
        "mark.note-highlight mark.search-hit",
      );
      expect(hit?.textContent).toBe("fleet");
      expect(hit).toHaveClass("active-hit");
    });
  });
});

describe("search hits in the hidden title heading (#585)", () => {
  const originalScrollIntoView = Element.prototype.scrollIntoView;
  // Test files share one environment (`isolate: false`), so the stub must
  // not outlive this block.
  afterEach(() => {
    Element.prototype.scrollIntoView = originalScrollIntoView;
  });

  it("counts only visible hits and lands on the body's", async () => {
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    const { container } = renderReadingView(
      "# home\n\nThe home lab.\n",
      "?q=hom",
    );

    await waitFor(() => {
      expect(screen.getByText(/Match 1 of 1/)).toBeInTheDocument();
    });
    // The title heading does carry a mark: it is hidden, not unmarked.
    expect(
      container.querySelector(".note-heading-duplicate mark.search-hit"),
    ).not.toBeNull();
    const bodyHit = container.querySelector("p mark.search-hit");
    expect(bodyHit).toHaveClass("active-hit");
    expect(
      container.querySelector(".note-heading-duplicate mark.active-hit"),
    ).toBeNull();
    expect(scrollIntoView.mock.contexts[0]).toBe(bodyHit);
  });

  it("shows no navigator when the title holds the only hits", async () => {
    const { container } = renderReadingView("# home\n\nA lab.\n", "?q=hom");

    await waitFor(() => {
      expect(
        container.querySelector(".note-heading-duplicate mark.search-hit"),
      ).not.toBeNull();
    });
    expect(container.querySelector(".note-search-nav")).toBeNull();
  });
});

/** The element the Reading view draws for each inline syntax. */
const DRAWN_AS: Record<string, string> = {
  "**": "strong",
  "*": "em",
  "~~": "del",
  "`": "code",
  "==": "mark",
};

/** Every delimiter the editor's two formatting toolbars wrap a selection in. */
function toolbarDelimiters(): string[] {
  const found = new Set<string>();
  for (const source of [menusSource, keyboardBarSource]) {
    const calls = [...source.matchAll(/wrapSelection\(/g)].length;
    const read = [...source.matchAll(/wrapSelection\(v, "([^"]+)"\)/g)];
    // A call in any other shape would slip past this check unread.
    expect(read.length, "every wrapSelection call is read").toBe(calls);
    for (const match of read) found.add(match[1]);
  }
  return [...found];
}

describe("the Reading view draws what the editor's buttons write (#572)", () => {
  it("renders every toolbar format as its element, delimiters gone", async () => {
    const delimiters = toolbarDelimiters();
    // The toolbars' sources were read; an empty list would prove nothing.
    expect(delimiters).toContain("**");
    for (const delimiter of delimiters) {
      expect(
        DRAWN_AS[delimiter],
        `no Reading view element for the ${delimiter} button`,
      ).toBeDefined();
    }
    // The link button writes `[text](url)` through `insertLink`.
    expect(menusSource).toContain("insertLink");
    expect(keyboardBarSource).toContain("insertLink");

    const lines = delimiters.map(
      (delimiter, index) => `Line${index} ${delimiter}word${delimiter} end`,
    );
    lines.push("LineLink [word](url) end");
    const { container } = renderReadingView(lines.join("\n\n"));

    await screen.findByText(/LineLink/);
    delimiters.forEach((delimiter, index) => {
      const paragraph = paragraphStarting(container, `Line${index} `);
      expect(paragraph?.textContent).toBe(`Line${index} word end`);
      expect(paragraph?.querySelector(DRAWN_AS[delimiter])?.textContent).toBe(
        "word",
      );
    });
    const link = paragraphStarting(container, "LineLink");
    expect(link?.textContent).toBe("LineLink word end");
    expect(link?.querySelector("a")?.textContent).toBe("word");
  });
});
