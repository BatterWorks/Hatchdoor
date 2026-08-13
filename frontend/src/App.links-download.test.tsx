import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { VaultApp as App } from "./App";
import { escapeMarkdownLabel } from "./lib/markdown";
import { discoveryResponse, healthyVault } from "./test/fixtures/vaults";

const VAULT = healthyVault("Vault");
const VAULT_ID = VAULT.vault_id;

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function collectionEnvelope(data: unknown): Response {
  return jsonResponse({
    scope: "all",
    collection_revision: 1,
    partial: false,
    participants: [
      { vault_id: VAULT_ID, vault_name: VAULT.name, state: "fresh" },
    ],
    data,
  });
}

function noteResponse(
  overrides: Partial<{
    title: string;
    slug: string;
    relative_path: string;
    content: string;
    content_hash: string;
  }> = {},
): Response {
  return jsonResponse({
    vault_id: VAULT_ID,
    note: {
      title: "Home",
      slug: "home",
      relative_path: "Home",
      content: "",
      content_hash: "hash",
      layer: null,
      ...overrides,
    },
  });
}

function linksResponse(): Response {
  return jsonResponse({ vault_id: VAULT_ID, outgoing: [], backlinks: [] });
}

function resolveBatchResponse(
  results: Array<{ target: string; slug: string | null; archived: boolean }>,
): Response {
  return jsonResponse({ vault_id: VAULT_ID, results });
}

function treeEnvelope(notes: Array<{ title: string; slug: string }>): Response {
  return collectionEnvelope([
    {
      vault_id: VAULT_ID,
      vault_name: VAULT.name,
      tree: {
        name: "Vault",
        folders: [],
        notes: notes.map((note) => ({ vault_id: VAULT_ID, ...note })),
      },
    },
  ]);
}

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe("App links/download", () => {
  it("renders a relative Markdown PDF link as a vault asset instead of a note route", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation(async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/tree")) {
          return treeEnvelope([{ title: "Home", slug: "home" }]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        if (url.includes("/notes/home/links")) {
          return linksResponse();
        }
        if (url.includes("/notes/home")) {
          return noteResponse({
            relative_path: "Reports/Home",
            content:
              "Read [the report](vve-energy-saving-scenarios-july-2026.pdf)",
          });
        }

        return jsonResponse({ error: "not found" }, 404);
      });

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App />
      </MemoryRouter>,
    );

    const link = await screen.findByRole("link", {
      name: "the report (PDF document, opens in a new tab)",
    });
    expect(link).toHaveAttribute(
      "href",
      `/api/v1/vaults/${VAULT_ID}/assets/Reports/vve-energy-saving-scenarios-july-2026.pdf`,
    );
    expect(link).toHaveClass("pdf-link");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
    expect(link.querySelector(".pdf-link-badge")).toHaveTextContent("PDF");
    expect(link.querySelector(".pdf-link-open")).toHaveTextContent("↗");
    expect(fetchMock).not.toHaveBeenCalledWith(
      expect.stringContaining("/resolve-batch"),
      expect.anything(),
    );
  });

  it("renders unresolved wikilinks as broken links", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/tree")) {
          return treeEnvelope([{ title: "Home", slug: "home" }]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        if (url.includes("/notes/home/links")) {
          return linksResponse();
        }
        if (url.includes("/notes/home")) {
          return noteResponse({ content: "Missing [[Nope|Alias Label]]" });
        }
        if (url.includes("/resolve-batch")) {
          return resolveBatchResponse([
            { target: "Nope", slug: null, archived: false },
          ]);
        }

        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App />
      </MemoryRouter>,
    );

    await waitFor(() => {
      const broken = screen.getByText("Alias Label");
      expect(broken).toHaveClass("broken-link");
      expect(broken).toHaveAttribute("title", "Missing: Nope");
    });
  });

  it("renders archived wikilinks with archived-link class", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/tree")) {
          return treeEnvelope([{ title: "Home", slug: "home" }]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        if (url.includes("/notes/home/links")) {
          return linksResponse();
        }
        if (url.includes("/notes/home")) {
          return noteResponse({ content: "See [[Old Setup|old setup log]]" });
        }
        if (url.includes("/resolve-batch")) {
          return resolveBatchResponse([
            { target: "Old Setup", slug: "old-setup", archived: true },
          ]);
        }

        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App />
      </MemoryRouter>,
    );

    await waitFor(() => {
      const link = screen.getByText("old setup log");
      expect(link).toHaveClass("archived-link");
      expect(link).toHaveAttribute("href", `/v/${VAULT_ID}/n/old-setup`);
    });
  });

  it("shows folder context only for archived wikilinks without aliases", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/tree")) {
          return treeEnvelope([{ title: "Home", slug: "home" }]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        if (url.includes("/notes/home/links")) {
          return linksResponse();
        }
        if (url.includes("/notes/home")) {
          return noteResponse({
            content_hash: "hash-1",
            content:
              "See [[40-reference/idea - example 1]] and [[90-archive/idea - example 2]]",
          });
        }
        if (url.includes("/resolve-batch")) {
          return resolveBatchResponse([
            {
              target: "40-reference/idea - example 1",
              slug: "idea-example-1",
              archived: false,
            },
            {
              target: "90-archive/idea - example 2",
              slug: "idea-example-2",
              archived: true,
            },
          ]);
        }

        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App />
      </MemoryRouter>,
    );

    const activeLink = await screen.findByRole(
      "link",
      {
        name: "idea - example 1",
      },
      { timeout: 3_000 },
    );
    expect(activeLink).toHaveAttribute(
      "href",
      `/v/${VAULT_ID}/n/idea-example-1`,
    );

    const archivedLink = await screen.findByRole(
      "link",
      {
        name: "90-archive/idea - example 2",
      },
      { timeout: 3_000 },
    );
    expect(archivedLink).toHaveClass("archived-link");
    expect(archivedLink).toHaveAttribute(
      "href",
      `/v/${VAULT_ID}/n/idea-example-2`,
    );
  });

  it("opens external markdown links in a new tab", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/tree")) {
          return treeEnvelope([{ title: "Home", slug: "home" }]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        if (url.includes("/notes/home/links")) {
          return linksResponse();
        }
        if (url.includes("/notes/home")) {
          return noteResponse({
            content: `[External](https://example.com)\n\n[Internal](/v/${VAULT_ID}/n/home)`,
          });
        }
        if (url.includes("/resolve-batch")) {
          return resolveBatchResponse([]);
        }

        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    const external = await screen.findByRole("link", { name: "External" });
    expect(external).toHaveAttribute("href", "https://example.com");
    expect(external).toHaveAttribute("target", "_blank");
    expect(external).toHaveAttribute("rel", "noopener noreferrer");

    const internal = screen.getByRole("link", { name: "Internal" });
    expect(internal).toHaveAttribute("href", `/v/${VAULT_ID}/n/home`);
    expect(internal).not.toHaveAttribute("target");
  });

  it("triggers markdown download endpoint from the actions menu via anchor", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/tree")) {
          return treeEnvelope([{ title: "Home", slug: "home" }]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        if (url.includes("/notes/home/links")) {
          return linksResponse();
        }
        if (url.includes("/notes/home")) {
          return noteResponse({
            relative_path: "Notes/Home",
            content: "# Home\n\nDownloaded content",
          });
        }
        if (url.includes("/resolve-batch")) {
          return resolveBatchResponse([]);
        }

        return jsonResponse({ error: "not found" }, 404);
      },
    );

    const clickSpy = vi
      .spyOn(HTMLAnchorElement.prototype, "click")
      .mockImplementation(() => {});
    const originalCreateElement = document.createElement.bind(document);
    const createdAnchors: HTMLAnchorElement[] = [];
    vi.spyOn(document, "createElement").mockImplementation(((
      tagName: string,
      options?: ElementCreationOptions,
    ) => {
      const element = originalCreateElement(tagName, options);
      if (tagName.toLowerCase() === "a") {
        createdAnchors.push(element as HTMLAnchorElement);
      }
      return element;
    }) as typeof document.createElement);

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Download .md" }),
    );

    expect(clickSpy).toHaveBeenCalledTimes(1);
    const downloadAnchor = createdAnchors.find(
      (anchor) =>
        anchor.getAttribute("href") ===
        `/api/v1/vaults/${VAULT_ID}/notes/home/download`,
    );
    expect(downloadAnchor).toBeTruthy();
    expect(downloadAnchor?.hasAttribute("download")).toBe(true);
  });

  it("does not use window.open or location navigation for markdown download", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/tree")) {
          return treeEnvelope([{ title: "Home", slug: "home" }]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        if (url.includes("/notes/home/links")) {
          return linksResponse();
        }
        if (url.includes("/notes/home")) {
          return noteResponse({
            relative_path: "Notes/Home",
            content: "# Home",
          });
        }
        if (url.includes("/resolve-batch")) {
          return resolveBatchResponse([]);
        }

        return jsonResponse({ error: "not found" }, 404);
      },
    );
    const openSpy = vi.spyOn(window, "open");
    const clickSpy = vi
      .spyOn(HTMLAnchorElement.prototype, "click")
      .mockImplementation(() => {});

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Download .md" }),
    );

    expect(openSpy).not.toHaveBeenCalled();
    expect(clickSpy).toHaveBeenCalledTimes(1);
  });

  it("copies cleaned markdown from the loaded note without fetching first", async () => {
    const clipboardWrite = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: clipboardWrite },
    });

    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/tree")) {
          return treeEnvelope([{ title: "Home", slug: "home" }]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        if (url.includes("/notes/home/download")) {
          throw new Error("copy should not fetch the download endpoint");
        }
        if (url.includes("/notes/home/links")) {
          return linksResponse();
        }
        if (url.includes("/notes/home")) {
          return noteResponse({
            relative_path: "Notes/Home",
            content: "---\ntags: [vault/sort]\n---\n# Home\n\nClean body",
          });
        }
        if (url.includes("/resolve-batch")) {
          return resolveBatchResponse([]);
        }

        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Copy page content" }),
    );

    await waitFor(() => {
      expect(clipboardWrite).toHaveBeenCalledWith("# Home\n\nClean body");
    });
  });

  it("copies the note link through the fallback helper when clipboard API is unavailable", async () => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: undefined,
    });
    const execCommandMock = vi.fn((command: string) => command === "copy");
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: execCommandMock,
    });

    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/tree")) {
          return treeEnvelope([{ title: "Home", slug: "home" }]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        if (url.includes("/write-capabilities")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            enabled: true,
            warnings: [],
          });
        }
        if (url.includes("/notes/home/links")) {
          return linksResponse();
        }
        if (url.includes("/notes/home")) {
          return noteResponse({
            relative_path: "Notes/Home",
            content: "# Home",
          });
        }
        if (url.includes("/resolve-batch")) {
          return resolveBatchResponse([]);
        }

        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { level: 2, name: "Home" });
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(
      await screen.findByRole("menuitem", { name: "Copy note link" }),
    );

    await waitFor(() => {
      expect(execCommandMock).toHaveBeenCalledWith("copy");
    });
  });

  it("escapes markdown control chars in wikilink labels", () => {
    expect(escapeMarkdownLabel("a]b(c) *x*")).toBe("a\\]b\\(c\\) \\*x\\*");
  });
});
