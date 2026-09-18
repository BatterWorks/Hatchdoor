import {
  cleanup,
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
  remarkHideQueryMarkers,
  hasSavedQueryBlock,
  savedQueryKey,
  useSavedQueries,
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
  status: "table",
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
  status: "table",
  columns: [{ id: "price", label: "price" }],
  rows: [row("newspaper", "Newspaper", [8])],
};

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
    const { container } = renderNote(TWO_BLOCKS, {
      status: "ready",
      results: [ACTIVE_RESULT, CHEAP_RESULT],
    });

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
    renderNote(TWO_BLOCKS, {
      status: "ready",
      results: [ACTIVE_RESULT, CHEAP_RESULT],
    });
    expect(document.body.textContent).not.toContain("hatchdoor-query");
    expect(document.body.textContent).not.toContain("finished > now");
  });

  it("tells two identical blocks apart by their position", () => {
    const block = `\`\`\`base\n${CHEAP}\n\`\`\``;
    renderNote(`${block}\n\n${block}`, {
      status: "ready",
      results: [
        CHEAP_RESULT,
        {
          source: CHEAP,
          status: "refused",
          message: "The name is already used.",
        },
      ],
    });
    expect(screen.getByRole("link", { name: "8" })).toBeInTheDocument();
    expect(
      screen.getByText("Not evaluated. The name is already used."),
    ).toBeInTheDocument();
  });

  it("hides only the marker, leaving other raw HTML as it always rendered", () => {
    renderNote("<!-- a comment of the author's own -->\n\ntext", null);
    expect(document.body.textContent).toContain(
      "a comment of the author's own",
    );
  });

  it("keeps refused, stopped and empty visibly different", () => {
    const blocks = ["filters: 'a'", "filters: 'b'", "filters: 'c'"];
    const markdown = blocks
      .map((source) => `\`\`\`base\n${source}\n\`\`\``)
      .join("\n\n");
    renderNote(markdown, {
      status: "ready",
      results: [
        {
          source: blocks[0],
          status: "refused",
          message: "formulas is not supported.",
        },
        { source: blocks[1], status: "stopped", message: "Too many notes." },
        { source: blocks[2], status: "table", columns: [], rows: [] },
      ],
    });
    expect(
      screen.getByText("Not evaluated. formulas is not supported."),
    ).toBeInTheDocument();
    expect(screen.getByText("Stopped. Too many notes.")).toBeInTheDocument();
    expect(
      screen.getByText("No notes match this saved query."),
    ).toBeInTheDocument();
  });

  it("shows a set-aside name under a table that still has its rows", () => {
    renderNote(`\`\`\`base\n${CHEAP}\n\`\`\``, {
      status: "ready",
      results: [
        { ...CHEAP_RESULT, notices: ['The name "Bad Name" is not usable.'] },
      ],
    });
    expect(screen.getByRole("link", { name: "8" })).toBeInTheDocument();
    expect(
      screen.getByText('The name "Bad Name" is not usable.'),
    ).toBeInTheDocument();
  });

  it("says when rows were held back, and by whom", () => {
    renderNote(`\`\`\`base\n${CHEAP}\n\`\`\``, {
      status: "ready",
      results: [
        {
          ...CHEAP_RESULT,
          truncated: { reason: "definition_limit", shown: 1 },
        },
      ],
    });
    expect(
      screen.getByText(/the limit this saved query sets/),
    ).toBeInTheDocument();
  });

  it("shows the definition as code in the editor preview, where nothing is evaluated", () => {
    renderNote(`\`\`\`base\n${CHEAP}\n\`\`\``, null);
    expect(screen.getByText(CHEAP)).toBeInTheDocument();
    expect(document.querySelector(".saved-query")).toBeNull();
  });

  it("reports a loading state and a failed load inside the block", () => {
    const markdown = `\`\`\`base\n${CHEAP}\n\`\`\``;
    renderNote(markdown, { status: "loading", results: [] });
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

  it("finds base fences only", () => {
    expect(hasSavedQueryBlock("text\n```base\nviews: []\n```")).toBe(true);
    expect(hasSavedQueryBlock("~~~~ base\n")).toBe(true);
    expect(hasSavedQueryBlock("```baseline\n```")).toBe(false);
    expect(hasSavedQueryBlock("```yaml\nbase: 1\n```")).toBe(false);
    expect(hasSavedQueryBlock("no fences at all")).toBe(false);
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
    expect(result.current).toEqual({
      status: "ready",
      results: [CHEAP_RESULT],
    });

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
