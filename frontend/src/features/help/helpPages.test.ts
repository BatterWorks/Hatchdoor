import { afterEach, describe, expect, it, vi } from "vitest";

import { setToken } from "../../api/api";
import helpCss from "./help.css?raw";
import {
  fetchHelpPage,
  helpPageTitle,
  plainExcerpt,
  resolveHelpLink,
  searchHelp,
} from "./helpPages";

afterEach(() => {
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe("resolveHelpLink", () => {
  it("resolves a relative page link against the page it sits on", () => {
    expect(
      resolveHelpLink(
        "../reference/supported-markdown-reference.md#callouts",
        "guides/how-to-set-up-a-git-backed-vault",
      ),
    ).toEqual({
      page: "reference/supported-markdown-reference",
      heading: "callouts",
    });
    expect(
      resolveHelpLink("get-started/connect-your-agent.md", "index"),
    ).toEqual({ page: "get-started/connect-your-agent" });
  });

  it("keeps an anchor-only link on the same page", () => {
    expect(resolveHelpLink("#tables", "reference/x")).toEqual({
      page: "reference/x",
      heading: "tables",
    });
  });

  it("reads the short deploy address as the deploy page", () => {
    expect(resolveHelpLink("/docs/deploy.md", "index")).toEqual({
      page: "guides/how-to-deploy-hatchdoor-with-an-agent",
    });
  });

  it("leaves external and non-manual links alone", () => {
    expect(resolveHelpLink("https://example.com/a.md", "index")).toBeNull();
    expect(resolveHelpLink("/api/v1/vaults", "index")).toBeNull();
    expect(resolveHelpLink("mailto:a@b.c", "index")).toBeNull();
    expect(resolveHelpLink(undefined, "index")).toBeNull();
  });
});

describe("helpPageTitle", () => {
  it("takes the first top-level heading outside code", () => {
    expect(
      helpPageTitle("```\n# not this\n```\n\n# Install Hatchdoor\n\n## Next"),
    ).toBe("Install Hatchdoor");
  });

  it("falls back to the page name", () => {
    expect(helpPageTitle("No heading here", "guides/a-page")).toBe(
      "guides/a-page",
    );
  });
});

describe("plainExcerpt", () => {
  it("keeps snake_case names and punctuation where they were", () => {
    expect(plainExcerpt("Set `HATCHDOOR_MCP_WRITE_ENABLED`? **Yes**.")).toBe(
      "Set HATCHDOOR_MCP_WRITE_ENABLED? Yes.",
    );
  });

  it("drops Markdown syntax but keeps the words", () => {
    expect(
      plainExcerpt(
        "| `git` | from the **Settings** screen, see [How to deploy](guides/how-to-deploy) |",
      ),
    ).toBe("git from the Settings screen, see How to deploy");
  });
});

describe("fetchHelpPage", () => {
  it("fetches the public address and sends the web token when there is one", async () => {
    setToken("web-token");
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(new Response("# Page", { status: 200 }));

    const result = await fetchHelpPage("guides/a-page");

    expect(result).toEqual({ kind: "page", markdown: "# Page" });
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("/docs/guides/a-page.md");
    expect(new Headers(init?.headers).get("Authorization")).toBe(
      "Bearer web-token",
    );
  });

  it("sends no token when signed out", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValue(new Response("# Page", { status: 200 }));

    await fetchHelpPage("index");

    const [, init] = fetchMock.mock.calls[0];
    expect(new Headers(init?.headers).has("Authorization")).toBe(false);
  });

  it("names a missing page, a private page and a failure apart", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch");
    fetchMock.mockResolvedValueOnce(new Response("", { status: 404 }));
    expect(await fetchHelpPage("nope")).toEqual({ kind: "missing" });
    fetchMock.mockResolvedValueOnce(new Response("", { status: 401 }));
    expect(await fetchHelpPage("whats-new")).toEqual({ kind: "private" });
    fetchMock.mockResolvedValueOnce(new Response("", { status: 500 }));
    expect(await fetchHelpPage("index")).toEqual({ kind: "error" });
    fetchMock.mockRejectedValueOnce(new TypeError("offline"));
    expect(await fetchHelpPage("index")).toEqual({ kind: "error" });
  });
});

describe("searchHelp", () => {
  it("asks the docs search and returns its hits", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(
        JSON.stringify({
          results: [{ name: "guides/a", title: "A", excerpt: "about a" }],
        }),
        { status: 200 },
      ),
    );

    const hits = await searchHelp("git sync");

    expect(fetchMock.mock.calls[0][0]).toBe("/docs/search?q=git+sync");
    expect(hits).toEqual([
      { name: "guides/a", title: "A", excerpt: "about a" },
    ]);
  });
});

describe("phone layout contract", () => {
  // jsdom applies no media queries, so the phone rules are checked as source:
  // below the app's 920px breakpoint Help is a full-screen sheet with a
  // labelled Close and no full-width toggle (#417 resolution).
  it("makes Help a full-screen sheet with a labelled Close below 920px", () => {
    const phone = helpCss.split("@media (max-width: 920px)")[1] ?? "";
    expect(phone).toMatch(
      /\.help-panel,\s*\.help-panel\.is-full\s*{[^}]*top: 0;[^}]*left: 0;[^}]*width: auto;/,
    );
    expect(phone).toMatch(/\.help-full-toggle\s*{\s*display: none;/);
    expect(phone).toMatch(/\.help-close-text\s*{\s*display: inline;/);
  });
});
