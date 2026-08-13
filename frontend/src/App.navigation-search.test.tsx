import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
  act,
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

describe("App navigation/search", () => {
  it("renders empty state on root route", async () => {
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
              tree: { name: "Vault", folders: [], notes: [] },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
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
      await screen.findByRole("heading", { name: "Notes Explorer" }),
    ).toBeInTheDocument();
  });

  it("renders tree error state", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response("boom", { status: 500 }),
    );

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    expect(
      await screen.findByText("Failed loading tree: 500"),
    ).toBeInTheDocument();
  });

  it("loads explorer tree and shows note links", async () => {
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
              content: "# Home\\n\\nHello",
              content_hash: "hash",
              layer: null,
            },
          });
        }

        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: VAULT_ID, results: [] });
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
      expect(
        screen.getAllByRole("link", { name: "Home" }).length,
      ).toBeGreaterThan(0);
    });

    await waitFor(() => {
      expect(
        screen.getByRole("heading", { level: 2, name: "Home" }),
      ).toBeInTheDocument();
    });
  });

  it("reloads visible vault data when a vault revision event arrives", async () => {
    let treeCalls = 0;
    let modifiedCalls = 0;
    let noteCalls = 0;
    let linksCalls = 0;

    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT]));
        }

        if (url.includes("/tree")) {
          treeCalls += 1;
          return collectionEnvelope([
            {
              vault_id: VAULT_ID,
              vault_name: VAULT.name,
              tree: {
                name: "Vault",
                folders: [],
                notes:
                  treeCalls === 1
                    ? [{ vault_id: VAULT_ID, title: "Home", slug: "home" }]
                    : [
                        { vault_id: VAULT_ID, title: "Home", slug: "home" },
                        {
                          vault_id: VAULT_ID,
                          title: "Project",
                          slug: "project",
                        },
                      ],
              },
            },
          ]);
        }

        if (url.includes("/recent")) {
          modifiedCalls += 1;
          return collectionEnvelope(
            modifiedCalls === 1
              ? []
              : [
                  {
                    vault_id: VAULT_ID,
                    title: "Project",
                    slug: "project",
                    relative_path: "Project",
                    mtime_ns: 40,
                  },
                ],
          );
        }

        if (url.includes("/notes/home/links")) {
          linksCalls += 1;
          return jsonResponse({
            vault_id: VAULT_ID,
            outgoing: [],
            backlinks: [],
          });
        }

        if (url.includes("/notes/home")) {
          noteCalls += 1;
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content:
                noteCalls === 1 ? "# Home\n\nVersion 1" : "# Home\n\nVersion 2",
              content_hash: "hash",
              layer: null,
            },
          });
        }

        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: VAULT_ID, results: [] });
        }

        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App />
      </MemoryRouter>,
    );

    expect(await screen.findByText("Version 1")).toBeInTheDocument();
    expect(window.__hatchdoorEventSources[0]?.url).toBe(
      "/api/v1/vaults/events",
    );

    act(() => {
      window.__hatchdoorEventSources[0].emit(
        "vault-collection-revision",
        JSON.stringify({
          collection_revision: 1,
          vault_ids: [VAULT_ID],
          category: "content",
        }),
      );
    });

    expect(await screen.findByText("Version 2")).toBeInTheDocument();
    await waitFor(() => {
      expect(treeCalls).toBeGreaterThanOrEqual(2);
      expect(modifiedCalls).toBeGreaterThanOrEqual(2);
      expect(linksCalls).toBeGreaterThanOrEqual(2);
    });
    expect(
      screen.getAllByRole("link", { name: "Project" }).length,
    ).toBeGreaterThan(0);
  });

  it("leaves transient vault event errors to EventSource reconnect logic", async () => {
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
              tree: { name: "Vault", folders: [], notes: [] },
            },
          ]);
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

        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    expect(
      await screen.findByRole("heading", { name: "Notes Explorer" }),
    ).toBeInTheDocument();

    act(() => {
      window.__hatchdoorEventSources[0].emit("error", "");
    });

    await waitFor(() => {
      expect(
        screen.queryByRole("dialog", { name: "Access token required" }),
      ).not.toBeInTheDocument();
    });
  });

  it("does not install the old vault polling interval", async () => {
    const setIntervalSpy = vi.spyOn(window, "setInterval");
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
              tree: { name: "Vault", folders: [], notes: [] },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
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
      await screen.findByRole("heading", { name: "Notes Explorer" }),
    ).toBeInTheDocument();
    expect(
      setIntervalSpy.mock.calls.some(([, delay]) => delay === 10_000),
    ).toBe(false);
  });

  it("renders folders collapsed by default on explorer root", async () => {
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
                folders: [
                  {
                    name: "Projects",
                    folders: [],
                    notes: [
                      { vault_id: VAULT_ID, title: "Plan", slug: "plan" },
                    ],
                  },
                ],
                notes: [],
              },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }
        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    expect(await screen.findByText("Projects")).toBeInTheDocument();
    const folderDetails = document.querySelector(".folder-item details");
    expect(folderDetails).not.toBeNull();
    expect(folderDetails).not.toHaveAttribute("open");
  });

  it("opens the folder chain for the active note route", async () => {
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
                folders: [
                  {
                    name: "Projects",
                    folders: [],
                    notes: [
                      { vault_id: VAULT_ID, title: "Plan", slug: "plan" },
                    ],
                  },
                ],
                notes: [],
              },
            },
          ]);
        }

        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }

        if (url.includes("/notes/plan/links")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            outgoing: [],
            backlinks: [],
          });
        }

        if (url.includes("/notes/plan")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Plan",
              slug: "plan",
              relative_path: "Projects/Plan",
              content: "# Plan",
              content_hash: "hash",
              layer: null,
            },
          });
        }

        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: VAULT_ID, results: [] });
        }

        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/plan`]}>
        <App />
      </MemoryRouter>,
    );

    expect(await screen.findByText("Projects")).toBeInTheDocument();
    const folderDetails = document.querySelector(".folder-item details");
    expect(folderDetails).not.toBeNull();
    expect(folderDetails).toHaveAttribute("open");
  });

  it("shows recent notes after opening a note", async () => {
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

        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App />
      </MemoryRouter>,
    );

    const recent = await screen.findByTestId("recent-notes");
    expect(within(recent).getByText("Recently viewed")).toBeInTheDocument();
    await waitFor(() => {
      expect(
        within(recent).getByRole("link", { name: "Home" }),
      ).toBeInTheDocument();
    });
  });

  it("lists notes changed on disk in the changes panel", async () => {
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
                  { vault_id: VAULT_ID, title: "Project", slug: "project" },
                ],
              },
            },
          ]);
        }

        if (url.includes("/recent")) {
          return collectionEnvelope([
            {
              vault_id: VAULT_ID,
              title: "Project",
              slug: "project",
              relative_path: "Projects/Project",
              mtime_ns: 30,
            },
            {
              vault_id: VAULT_ID,
              title: "Home",
              slug: "home",
              relative_path: "Home",
              mtime_ns: 20,
            },
          ]);
        }

        return jsonResponse({ error: "not found" }, 404);
      },
    );

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    // Last Modified no longer sits in the sidebar: it conflated awareness with
    // navigation. The same data now opens from the rail instead.
    const openChanges = await screen.findByRole("button", {
      name: "Recently changed notes",
    });
    fireEvent.click(openChanges);

    const changes = await screen.findByRole("region", {
      name: "Recently changed notes",
    });
    expect(
      within(changes).getByRole("link", { name: "Project" }),
    ).toHaveAttribute("href", `/v/${VAULT_ID}/n/project`);
    expect(within(changes).getByRole("link", { name: "Home" })).toHaveAttribute(
      "title",
      "Home.md",
    );
  });

  it("opens search and lists matches", async () => {
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

        if (url.includes("/search")) {
          return collectionEnvelope({
            mode: "semantic",
            results: [
              {
                vault_id: VAULT_ID,
                chunk_id: 1,
                note_slug: "plan",
                note_title: "Plan",
                note_path: "Projects/Plan",
                heading_path: null,
                content: "Plan body text",
                score: 0.9,
                outbound_links: [],
              },
            ],
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
    fireEvent.change(input, { target: { value: "plan" } });

    expect(
      await screen.findByText(
        (_value, element) => element?.textContent === "Projects/Plan.md",
      ),
    ).toBeInTheDocument();
    expect(
      await screen.findByText(
        (_value, element) => element?.textContent === "Plan body text",
      ),
    ).toBeInTheDocument();
  });
});
