import {
  cleanup,
  fireEvent,
  render,
  renderHook,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import ReactMarkdown from "react-markdown";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { apiFetch } from "../../api/api";
import { createNoteMarkdownComponents } from "./renderers";
import { SavedQueryProvider } from "./SavedQueryBlock";
import {
  baseFenceLines,
  formatCell,
  hasSavedQueryMarkup,
  nextSort,
  remarkHideQueryMarkers,
  savedQueryKey,
  sortRows,
  useSavedQueries,
  type SavedQueryMarkerProblem,
  type SavedQueryResult,
  type SavedQueryState,
} from "./savedQueries";

vi.mock("../../api/api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../api/api")>()),
  apiFetch: vi.fn(),
}));

const mockedApiFetch = vi.mocked(apiFetch);

afterEach(() => {
  cleanup();
  mockedApiFetch.mockReset();
});

const VAULT_ID = "vault-1";

const ACTIVE = `filters: 'finished == null || finished > now()'
views:
  - type: table
    name: Active subscriptions
    order: [file.name, price]`;

const CHEAP = "filters: 'price < 10'";

function row(slug: string, title: string, cells: unknown[]) {
  return {
    vault_id: VAULT_ID,
    title,
    slug,
    relative_path: `subscriptions/${title}`,
    cells,
  };
}

const ACTIVE_RESULT: SavedQueryResult = {
  name: "active-subscriptions",
  source: ACTIVE,
  status: "populated",
  view_name: "Active subscriptions",
  columns: [
    { id: "file.name", label: "name" },
    { id: "price", label: "price" },
  ],
  rows: [
    row("netflix", "Netflix", ["Netflix.md", 13.99]),
    row("newspaper", "Newspaper", ["Newspaper.md", null]),
  ],
};

const CHEAP_RESULT: SavedQueryResult = {
  source: CHEAP,
  status: "populated",
  columns: [{ id: "price", label: "price" }],
  rows: [row("newspaper", "Newspaper", [8])],
};

function ready(
  results: SavedQueryResult[],
  markerProblems: SavedQueryMarkerProblem[] = [],
): SavedQueryState {
  return { status: "ready", results, markerProblems };
}

function renderNote(markdown: string, state: SavedQueryState | null) {
  const body = (
    <ReactMarkdown
      remarkPlugins={[remarkHideQueryMarkers]}
      components={createNoteMarkdownComponents(
        VAULT_ID,
        "Dashboard",
        new Map(),
      )}
    >
      {markdown}
    </ReactMarkdown>
  );
  return render(
    <MemoryRouter>
      {state ? (
        <SavedQueryProvider
          state={state}
          vaultId={VAULT_ID}
          markdown={markdown}
        >
          {body}
        </SavedQueryProvider>
      ) : (
        body
      )}
    </MemoryRouter>,
  );
}

const TWO_BLOCKS = `# Dashboard

Intro text.

<!-- hatchdoor-query: active-subscriptions -->

\`\`\`base
${ACTIVE}
\`\`\`

Between the two.

\`\`\`base
${CHEAP}
\`\`\`
`;

describe("saved query blocks on the note page", () => {
  it("draws each block as its own table in its own position", () => {
    const { container } = renderNote(
      TWO_BLOCKS,
      ready([ACTIVE_RESULT, CHEAP_RESULT]),
    );

    const frames = container.querySelectorAll(".saved-query");
    expect(frames).toHaveLength(2);

    // Document order: intro, first table, the paragraph between, second table.
    const blocks = Array.from(container.children).map((element) =>
      element.classList.contains("saved-query")
        ? "table"
        : element.textContent?.trim(),
    );
    expect(blocks).toEqual([
      "Dashboard",
      "Intro text.",
      "table",
      "Between the two.",
      "table",
    ]);

    const first = within(frames[0] as HTMLElement);
    expect(first.getByText("Active subscriptions")).toBeInTheDocument();
    expect(
      first.getAllByRole("columnheader").map((cell) => cell.textContent),
    ).toEqual(["name", "price"]);
    expect(first.getByRole("link", { name: "Netflix" })).toHaveAttribute(
      "href",
      "/v/vault-1/n/netflix",
    );
    // A property the note does not carry is an empty cell, not "null".
    const newspaper = first
      .getByRole("link", { name: "Newspaper" })
      .closest("tr");
    expect(newspaper?.querySelectorAll("td")[1].textContent).toBe("");

    const second = within(frames[1] as HTMLElement);
    expect(second.getByRole("link", { name: "8" })).toHaveAttribute(
      "href",
      "/v/vault-1/n/newspaper",
    );
  });

  it("never shows the name marker or the definition text", () => {
    renderNote(TWO_BLOCKS, ready([ACTIVE_RESULT, CHEAP_RESULT]));
    expect(document.body.textContent).not.toContain("hatchdoor-query");
    expect(document.body.textContent).not.toContain("finished > now");
  });

  it("tells two identical blocks apart by their position", () => {
    const block = `\`\`\`base\n${CHEAP}\n\`\`\``;
    renderNote(
      `${block}\n\n${block}`,
      ready([
        CHEAP_RESULT,
        {
          source: CHEAP,
          status: "stopped",
          message: "Too many saved queries.",
        },
      ]),
    );
    expect(screen.getByRole("link", { name: "8" })).toBeInTheDocument();
    expect(screen.getByText(/Too many saved queries/)).toBeInTheDocument();
  });

  it("hides only the marker, leaving other raw HTML as it always rendered", () => {
    renderNote("<!-- a comment of the author's own -->\n\ntext", null);
    expect(document.body.textContent).toContain(
      "a comment of the author's own",
    );
  });

  it("keeps refused, stopped, empty and populated visibly different", () => {
    const blocks = ["filters: 'a'", "filters: 'b'", "filters: 'c'", CHEAP];
    const markdown = blocks
      .map((source) => `\`\`\`base\n${source}\n\`\`\``)
      .join("\n\n");
    const { container } = renderNote(
      markdown,
      ready([
        {
          source: blocks[0],
          status: "refused",
          construct: "daysUntil()",
          message: "The function daysUntil() is not supported.",
        },
        { source: blocks[1], status: "stopped", message: "Too many notes." },
        { source: blocks[2], status: "empty", columns: [] },
        CHEAP_RESULT,
      ]),
    );
    const frames = Array.from(container.querySelectorAll(".saved-query")).map(
      (frame) => frame.textContent ?? "",
    );
    expect(frames[0]).toContain(
      "Not evaluated. The function daysUntil() is not supported.",
    );
    expect(frames[1]).toContain("Stopped. Too many notes.");
    // Empty says it was read and checked, so it cannot pass for a failure,
    // and a refusal never says anything that could pass for empty.
    expect(frames[2]).toContain("No matches.");
    expect(frames[2]).toContain("read and checked against every note");
    expect(frames[0]).not.toMatch(/No matches|none qualifies/);
    expect(container.querySelectorAll(".saved-query table")).toHaveLength(1);
    expect(frames[3]).toContain("8");
  });

  it("names an ignored presentation instruction under rows that are all there", () => {
    renderNote(
      `\`\`\`base\n${CHEAP}\n\`\`\``,
      ready([
        {
          ...CHEAP_RESULT,
          ignored: [
            {
              instruction: "groupBy",
              message:
                "Grouping is not supported, so the rows are shown ungrouped.",
            },
          ],
        },
      ]),
    );
    expect(screen.getByRole("link", { name: "8" })).toBeInTheDocument();
    expect(
      screen.getByText(
        "Grouping is not supported, so the rows are shown ungrouped.",
      ),
    ).toBeInTheDocument();
  });

  it("says when rows were held back, and by whom", () => {
    const markdown = `\`\`\`base\n${CHEAP}\n\`\`\``;
    renderNote(
      markdown,
      ready([
        {
          ...CHEAP_RESULT,
          truncated: { reason: "definition_limit", shown: 1 },
        },
      ]),
    );
    expect(
      screen.getByText(/the limit this saved query sets/),
    ).toBeInTheDocument();
    cleanup();
    renderNote(
      markdown,
      ready([{ ...CHEAP_RESULT, truncated: { reason: "ceiling", shown: 1 } }]),
    );
    expect(screen.getByText(/^Truncated:/)).toBeInTheDocument();
  });

  it("puts an orphaned marker's notice where the marker sits and leaves the rest alone", () => {
    const markdown = `Before.

<!-- hatchdoor-query: lonely -->

After.

<!-- hatchdoor-query: cheap -->

\`\`\`base
${CHEAP}
\`\`\`
`;
    const { container } = renderNote(
      markdown,
      ready(
        [{ ...CHEAP_RESULT, name: "cheap" }],
        [
          {
            problem: "orphaned",
            name: "lonely",
            line: 3,
            message:
              'The marker naming "lonely" is not followed by a base block, so it names nothing.',
          },
        ],
      ),
    );
    const blocks = Array.from(container.children).map((element) =>
      element.classList.contains("saved-query")
        ? "table"
        : element.textContent?.trim(),
    );
    expect(blocks).toEqual([
      "Before.",
      'The marker naming "lonely" is not followed by a base block, so it names nothing.',
      "After.",
      "table",
    ]);
    expect(document.body.textContent).not.toContain("hatchdoor-query");
  });

  it("shows nothing for an orphaned marker the server has not reported", () => {
    const { container } = renderNote(
      "Before.\n\n<!-- hatchdoor-query: lonely -->\n\nAfter.",
      null,
    );
    expect(container.textContent).not.toContain("lonely");
    expect(container.querySelector(".saved-query-orphan")).toBeNull();
  });

  it("draws both tables of a name collision, each with the collision notice", () => {
    const block = (source: string) =>
      `<!-- hatchdoor-query: same -->\n\`\`\`base\n${source}\n\`\`\``;
    const message =
      '2 saved queries in this note are named "same", so none of them can be addressed by that name until only one is.';
    const { container } = renderNote(
      `${block(ACTIVE)}\n\n${block(CHEAP)}`,
      ready(
        [
          { ...ACTIVE_RESULT, name: "same" },
          { ...CHEAP_RESULT, name: "same" },
        ],
        [{ problem: "duplicate_name", name: "same", queries: [0, 1], message }],
      ),
    );
    const frames = container.querySelectorAll(".saved-query");
    expect(frames).toHaveLength(2);
    frames.forEach((frame) => {
      expect(frame.querySelector("table")).not.toBeNull();
      expect(frame.textContent).toContain(message);
    });
  });

  it("draws a block with an unusable name unnamed, with a notice", () => {
    renderNote(
      `<!-- hatchdoor-query: Bad Name -->\n\`\`\`base\n${CHEAP}\n\`\`\``,
      ready(
        [CHEAP_RESULT],
        [
          {
            problem: "unusable_name",
            name: "Bad Name",
            query: 0,
            message: '"Bad Name" is not a usable name.',
          },
        ],
      ),
    );
    expect(screen.getByRole("link", { name: "8" })).toBeInTheDocument();
    expect(
      screen.getByText('"Bad Name" is not a usable name.'),
    ).toBeInTheDocument();
  });

  it("re-sorts one table by a clicked heading, leaving the other alone", () => {
    const THIRD = row("gym", "Gym", ["Gym.md", 30]);
    const { container } = renderNote(
      TWO_BLOCKS,
      ready([
        { ...ACTIVE_RESULT, rows: [...ACTIVE_RESULT.rows, THIRD] },
        {
          ...CHEAP_RESULT,
          rows: [row("b", "B", [2]), row("a", "A", [1])],
        },
      ]),
    );
    const [first, second] = Array.from(
      container.querySelectorAll(".saved-query"),
    ).map((frame) => within(frame as HTMLElement));
    const names = () =>
      first.getAllByRole("link").map((link) => link.textContent);
    const prices = () =>
      second.getAllByRole("link").map((link) => link.textContent);

    expect(names()).toEqual(["Netflix", "Newspaper", "Gym"]);
    const price = first.getByRole("button", { name: "price" });
    fireEvent.click(price);
    // Ascending by price, the note without one last.
    expect(names()).toEqual(["Netflix", "Gym", "Newspaper"]);
    expect(price.closest("th")).toHaveAttribute("aria-sort", "ascending");
    fireEvent.click(price);
    expect(names()).toEqual(["Gym", "Netflix", "Newspaper"]);
    expect(price.closest("th")).toHaveAttribute("aria-sort", "descending");
    // The other table kept the server's order.
    expect(prices()).toEqual(["2", "1"]);
    fireEvent.click(price);
    expect(names()).toEqual(["Netflix", "Newspaper", "Gym"]);
    expect(price.closest("th")).toHaveAttribute("aria-sort", "none");
  });

  it("keeps a sort on screen only: nothing is sent and a fresh render forgets it", () => {
    const state = ready([
      {
        ...CHEAP_RESULT,
        rows: [row("b", "B", [2]), row("a", "A", [1])],
      },
    ]);
    const markdown = `\`\`\`base\n${CHEAP}\n\`\`\``;
    renderNote(markdown, state);
    fireEvent.click(screen.getByRole("button", { name: "price" }));
    expect(screen.getAllByRole("link").map((link) => link.textContent)).toEqual(
      ["1", "2"],
    );
    expect(mockedApiFetch).not.toHaveBeenCalled();
    cleanup();
    renderNote(markdown, state);
    expect(screen.getAllByRole("link").map((link) => link.textContent)).toEqual(
      ["2", "1"],
    );
  });

  it("shows the definition as code in the editor preview, where nothing is evaluated", () => {
    renderNote(`\`\`\`base\n${CHEAP}\n\`\`\``, null);
    expect(screen.getByText(CHEAP)).toBeInTheDocument();
    expect(document.querySelector(".saved-query")).toBeNull();
  });

  it("reports a loading state and a failed load inside the block", () => {
    const markdown = `\`\`\`base\n${CHEAP}\n\`\`\``;
    renderNote(markdown, {
      status: "loading",
      results: [],
      markerProblems: [],
    });
    expect(screen.getByText("Evaluating saved query…")).toBeInTheDocument();
    cleanup();
    renderNote(markdown, { status: "error", message: "Vault unavailable" });
    expect(
      screen.getByText(/could not be loaded: Vault unavailable/),
    ).toBeInTheDocument();
  });
});

describe("saved query helpers", () => {
  it("counts base fences the way the server does", () => {
    expect(
      baseFenceLines(
        "# T\n\n```base\na\n```\n\n````md\n```base\n````\n\n~~~ base\nb\n~~~",
      ),
    ).toEqual([3, 11]);
  });

  it("finds base fences and name markers only", () => {
    expect(hasSavedQueryMarkup("text\n```base\nviews: []\n```")).toBe(true);
    expect(hasSavedQueryMarkup("~~~~ base\n")).toBe(true);
    expect(hasSavedQueryMarkup("x\n<!-- hatchdoor-query: lonely -->")).toBe(
      true,
    );
    expect(hasSavedQueryMarkup("```baseline\n```")).toBe(false);
    expect(hasSavedQueryMarkup("```yaml\nbase: 1\n```")).toBe(false);
    expect(hasSavedQueryMarkup("<!-- an ordinary comment -->")).toBe(false);
    expect(hasSavedQueryMarkup("no fences at all")).toBe(false);
  });

  it("cycles a heading through ascending, descending and the server's order", () => {
    expect(nextSort(null, 1)).toEqual({ column: 1, direction: "ascending" });
    expect(nextSort({ column: 1, direction: "ascending" }, 1)).toEqual({
      column: 1,
      direction: "descending",
    });
    expect(nextSort({ column: 1, direction: "descending" }, 1)).toBeNull();
    expect(nextSort({ column: 1, direction: "descending" }, 0)).toEqual({
      column: 0,
      direction: "ascending",
    });
  });

  it("sorts numbers as numbers, text naturally, and empty cells last", () => {
    const rows = [
      row("a", "A", ["item 10", 10]),
      row("b", "B", [null, 9]),
      row("c", "C", ["Item 9", null]),
      row("d", "D", ["item 2", 100]),
    ];
    const titles = (column: number, direction: "ascending" | "descending") =>
      sortRows(rows, { column, direction }).map((sorted) => sorted.title);
    expect(titles(0, "ascending")).toEqual(["D", "C", "A", "B"]);
    expect(titles(0, "descending")).toEqual(["A", "C", "D", "B"]);
    expect(titles(1, "ascending")).toEqual(["B", "A", "D", "C"]);
    expect(sortRows(rows, null)).toBe(rows);
  });

  it("matches a block to its result across line-ending and trailing-space noise", () => {
    expect(savedQueryKey("a: 1  \r\nb: 2\n")).toBe(savedQueryKey("a: 1\nb: 2"));
  });

  it("formats cells the way a reader writes them", () => {
    expect(formatCell(null)).toBe("");
    expect(formatCell(["a", "b"])).toBe("a, b");
    expect(formatCell(true)).toBe("true");
    expect(formatCell(12.5)).toBe("12.5");
  });
});

describe("useSavedQueries", () => {
  const notePath = "/api/v1/vaults/vault-1/notes/dashboard";

  it("asks nothing of a note without a base block", () => {
    const { result } = renderHook(() =>
      useSavedQueries(notePath, "# Plain", "hash-1", 0),
    );
    expect(result.current).toEqual({ status: "none" });
    expect(mockedApiFetch).not.toHaveBeenCalled();
  });

  it("fetches the evaluated queries and refetches when the note changes", async () => {
    mockedApiFetch.mockImplementation(
      async () =>
        new Response(
          JSON.stringify({
            scope: "vault-1",
            collection_revision: 1,
            partial: false,
            participants: [],
            data: {
              vault_id: "vault-1",
              slug: "dashboard",
              queries: [CHEAP_RESULT],
              marker_problems: [],
            },
          }),
          { status: 200 },
        ),
    );
    const markdown = `\`\`\`base\n${CHEAP}\n\`\`\``;
    const { result, rerender } = renderHook(
      ({ hash }) => useSavedQueries(notePath, markdown, hash, 0),
      { initialProps: { hash: "hash-1" } },
    );
    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(mockedApiFetch).toHaveBeenCalledWith(`${notePath}/saved-queries`);
    expect(result.current).toEqual(ready([CHEAP_RESULT]));

    rerender({ hash: "hash-2" });
    await waitFor(() => expect(mockedApiFetch).toHaveBeenCalledTimes(2));
    // The previous tables stay up while the refetch is in flight.
    expect(result.current.status === "error").toBe(false);
  });

  it("surfaces a failed evaluation as an error rather than an empty result", async () => {
    mockedApiFetch.mockResolvedValue(
      new Response(
        JSON.stringify({
          code: "vault_unavailable",
          message: "Vault unavailable",
        }),
        {
          status: 503,
        },
      ),
    );
    const { result } = renderHook(() =>
      useSavedQueries(notePath, "```base\nviews: []\n```", "hash-1", 0),
    );
    await waitFor(() =>
      expect(result.current).toEqual({
        status: "error",
        message: "Vault unavailable",
      }),
    );
  });
});
