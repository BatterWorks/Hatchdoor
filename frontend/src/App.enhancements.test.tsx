import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { VaultApp as App } from "./App";
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

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe("App enhancements", () => {
  it("persists expanded folders in localStorage", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/tree")) {
          return collectionEnvelope([
            {
              vault_id: VAULT_ID,
              vault_name: VAULT.name,
              tree: {
                name: "Vault",
                folders: [{ name: "Projects", folders: [], notes: [] }],
                notes: [],
              },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        if (url.includes("/write-capabilities")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            enabled: false,
            warnings: [],
          });
        }
        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    fireEvent.click(await screen.findByText("Projects"));

    await waitFor(() => {
      const stored = window.localStorage.getItem("hatchdoor.expandedFolders");
      expect(stored).toContain('"Projects":true');
    });
  });

  it("renders links panel and table of contents for a note", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/tree")) {
          return collectionEnvelope([
            {
              vault_id: VAULT_ID,
              vault_name: VAULT.name,
              tree: {
                name: "Vault",
                folders: [],
                notes: [{ vault_id: VAULT_ID, title: "Home", slug: "home" }],
              },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }

        if (url.includes("/notes/home/links")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            outgoing: [
              {
                vault_id: VAULT_ID,
                link: {
                  title: "Plan",
                  slug: "plan",
                  relative_path: "Plan",
                  layer: null,
                },
              },
            ],
            backlinks: [
              {
                vault_id: VAULT_ID,
                link: {
                  title: "Overview",
                  slug: "overview",
                  relative_path: "Overview",
                  layer: null,
                },
              },
            ],
          });
        }

        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "# Intro\n\n## Section",
              content_hash: "hash",
              layer: null,
            },
          });
        }

        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: VAULT_ID, results: [] });
        }

        if (url.includes("/write-capabilities")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            enabled: false,
            warnings: [],
          });
        }

        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App />
      </MemoryRouter>,
    );

    const linksPanel = await screen.findByLabelText("Note links");
    expect(linksPanel).not.toHaveAttribute("open");
    fireEvent.click(screen.getByText("Links"));
    expect(await screen.findByText("Outgoing")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Plan" })).toBeInTheDocument();
    expect((await screen.findAllByText("On this page")).length).toBeGreaterThan(
      0,
    );
    expect(screen.getByRole("button", { name: "Section" })).toBeInTheDocument();
  });

  it("restores the last opened note when landing on root", async () => {
    window.localStorage.setItem(
      "hatchdoor.lastNote",
      JSON.stringify({ vaultId: VAULT_ID, slug: "home" }),
    );

    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/tree")) {
          return collectionEnvelope([
            {
              vault_id: VAULT_ID,
              vault_name: VAULT.name,
              tree: {
                name: "Vault",
                folders: [],
                notes: [{ vault_id: VAULT_ID, title: "Home", slug: "home" }],
              },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }

        if (url.includes("/notes/home/links")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            outgoing: [],
            backlinks: [],
          });
        }

        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "# Home",
              content_hash: "hash",
              layer: null,
            },
          });
        }

        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: VAULT_ID, results: [] });
        }

        if (url.includes("/write-capabilities")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            enabled: false,
            warnings: [],
          });
        }

        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    expect(
      await screen.findByRole("heading", { level: 2, name: "Home" }),
    ).toBeInTheDocument();
  });

  it("highlights in-note matches after opening a search result", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/tree")) {
          return collectionEnvelope([
            {
              vault_id: VAULT_ID,
              vault_name: VAULT.name,
              tree: {
                name: "Vault",
                folders: [],
                notes: [{ vault_id: VAULT_ID, title: "Home", slug: "home" }],
              },
            },
          ]);
        }

        if (url.includes("/search")) {
          return collectionEnvelope({
            mode: "semantic",
            results: [
              {
                vault_id: VAULT_ID,
                chunk_id: 1,
                note_slug: "home",
                note_title: "Home",
                note_path: "Home",
                heading_path: null,
                content: "token found here",
                score: 0.9,
                outbound_links: [],
              },
            ],
          });
        }

        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }

        if (url.includes("/notes/home/links")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            outgoing: [],
            backlinks: [],
          });
        }

        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "This line has token and another token.",
              content_hash: "hash",
              layer: null,
            },
          });
        }

        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: VAULT_ID, results: [] });
        }

        if (url.includes("/write-capabilities")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            enabled: false,
            warnings: [],
          });
        }

        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    fireEvent.click(await screen.findByRole("button", { name: "Search" }));
    const input = await screen.findByPlaceholderText("Search notes…");
    fireEvent.change(input, { target: { value: "token" } });

    fireEvent.click(await screen.findByRole("button", { name: /Home/ }));

    expect(await screen.findByText(/Match 1 of 2/)).toBeInTheDocument();
    expect(document.querySelectorAll("mark.search-hit")).toHaveLength(2);
  });

  it("keeps searched note renders stable while wikilinks resolve", async () => {
    const consoleError = vi
      .spyOn(console, "error")
      .mockImplementation(() => {});
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }
        if (url.includes("/tree")) {
          return collectionEnvelope([
            {
              vault_id: VAULT_ID,
              vault_name: VAULT.name,
              tree: {
                name: "Vault",
                folders: [],
                notes: [
                  { vault_id: VAULT_ID, title: "Home", slug: "home" },
                  { vault_id: VAULT_ID, title: "Plan", slug: "plan" },
                ],
              },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }

        if (url.includes("/notes/home/links")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            outgoing: [],
            backlinks: [],
          });
        }

        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "Read the token in [[Plan]].",
              content_hash: "hash",
              layer: null,
            },
          });
        }

        if (url.includes("/resolve-batch")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            results: [{ target: "Plan", slug: "plan", archived: false }],
          });
        }

        if (url.includes("/write-capabilities")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            enabled: false,
            warnings: [],
          });
        }

        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home?q=token`]}>
        <App />
      </MemoryRouter>,
    );

    expect(await screen.findByText(/Match 1 of 1/)).toBeInTheDocument();
    const noteBody = document.querySelector(".note-body");
    expect(noteBody).not.toBeNull();
    await waitFor(() => {
      expect(
        within(noteBody as HTMLElement).getByRole("link", { name: "Plan" }),
      ).toHaveAttribute("href", `/v/${VAULT_ID}/n/plan`);
    });
    expect(document.querySelectorAll("mark.search-hit")).toHaveLength(1);
    expect(consoleError).not.toHaveBeenCalled();
  });
});
