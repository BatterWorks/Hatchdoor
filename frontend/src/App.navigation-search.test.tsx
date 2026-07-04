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

import App from "./App";

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe("App navigation/search", () => {
  it("renders empty state on root route", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(
        JSON.stringify({
          name: "Vault",
          folders: [],
          notes: [],
        }),
        { status: 200 },
      ),
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
        if (url.includes("/api/tree")) {
          return new Response(
            JSON.stringify({
              name: "Vault",
              folders: [],
              notes: [{ title: "Home", slug: "home" }],
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/recently-modified")) {
          return new Response(JSON.stringify({ notes: [] }), { status: 200 });
        }

        if (url.includes("/api/note/home")) {
          return new Response(
            JSON.stringify({
              note: {
                title: "Home",
                slug: "home",
                relative_path: "Home",
                content: "# Home\\n\\nHello",
              },
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/resolve-batch")) {
          return new Response(JSON.stringify({ results: [] }), { status: 200 });
        }

        return new Response("not found", { status: 404 });
      },
    );

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
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
        if (url.includes("/api/tree")) {
          treeCalls += 1;
          return new Response(
            JSON.stringify({
              name: "Vault",
              folders: [],
              notes:
                treeCalls === 1
                  ? [{ title: "Home", slug: "home" }]
                  : [
                      { title: "Home", slug: "home" },
                      { title: "Project", slug: "project" },
                    ],
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/recently-modified")) {
          modifiedCalls += 1;
          return new Response(
            JSON.stringify({
              notes:
                modifiedCalls === 1
                  ? []
                  : [
                      {
                        title: "Project",
                        slug: "project",
                        relative_path: "Project",
                        mtime_ns: 40,
                      },
                    ],
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/note/home/links")) {
          linksCalls += 1;
          return new Response(
            JSON.stringify({ links: { outgoing: [], backlinks: [] } }),
            { status: 200 },
          );
        }

        if (url.includes("/api/note/home")) {
          noteCalls += 1;
          return new Response(
            JSON.stringify({
              note: {
                title: "Home",
                slug: "home",
                relative_path: "Home",
                content:
                  noteCalls === 1
                    ? "# Home\n\nVersion 1"
                    : "# Home\n\nVersion 2",
              },
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/resolve-batch")) {
          return new Response(JSON.stringify({ results: [] }), { status: 200 });
        }

        return new Response("not found", { status: 404 });
      },
    );

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    expect(await screen.findByText("Version 1")).toBeInTheDocument();
    expect(window.__hatchdoorEventSources[0]?.url).toBe("/api/vault-events");

    act(() => {
      window.__hatchdoorEventSources[0].emit(
        "vault-revision",
        JSON.stringify({ revision: 1 }),
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

  it("shows the token prompt and closes the stream when vault events error", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/api/tree")) {
          return new Response(
            JSON.stringify({
              name: "Vault",
              folders: [],
              notes: [],
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/recently-modified")) {
          return new Response(JSON.stringify({ notes: [] }), { status: 200 });
        }

        if (url.includes("/api/write-capabilities")) {
          return new Response(JSON.stringify({ enabled: true, warnings: [] }), {
            status: 200,
          });
        }

        return new Response("not found", { status: 404 });
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

    expect(
      await screen.findByRole("dialog", { name: "Access token required" }),
    ).toBeInTheDocument();
  });

  it("does not install the old vault polling interval", async () => {
    const setIntervalSpy = vi.spyOn(window, "setInterval");
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(
        JSON.stringify({
          name: "Vault",
          folders: [],
          notes: [],
        }),
        { status: 200 },
      ),
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
      async () =>
        new Response(
          JSON.stringify({
            name: "Vault",
            folders: [
              {
                name: "Projects",
                folders: [],
                notes: [{ title: "Plan", slug: "plan" }],
              },
            ],
            notes: [],
          }),
          { status: 200 },
        ),
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
        if (url.includes("/api/tree")) {
          return new Response(
            JSON.stringify({
              name: "Vault",
              folders: [
                {
                  name: "Projects",
                  folders: [],
                  notes: [{ title: "Plan", slug: "plan" }],
                },
              ],
              notes: [],
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/note/plan")) {
          return new Response(
            JSON.stringify({
              note: {
                title: "Plan",
                slug: "plan",
                relative_path: "Projects/Plan",
                content: "# Plan",
              },
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/resolve-batch")) {
          return new Response(JSON.stringify({ results: [] }), { status: 200 });
        }

        return new Response("not found", { status: 404 });
      },
    );

    render(
      <MemoryRouter initialEntries={["/n/plan"]}>
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
        if (url.includes("/api/tree")) {
          return new Response(
            JSON.stringify({
              name: "Vault",
              folders: [],
              notes: [{ title: "Home", slug: "home" }],
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/note/home")) {
          return new Response(
            JSON.stringify({
              note: {
                title: "Home",
                slug: "home",
                relative_path: "Home",
                content: "# Home",
              },
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/resolve-batch")) {
          return new Response(JSON.stringify({ results: [] }), { status: 200 });
        }

        return new Response("not found", { status: 404 });
      },
    );

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    const recent = await screen.findByTestId("recent-notes");
    expect(within(recent).getByText("Recently Viewed")).toBeInTheDocument();
    await waitFor(() => {
      expect(
        within(recent).getByRole("link", { name: "Home" }),
      ).toBeInTheDocument();
    });
  });

  it("shows last modified notes from source file metadata", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/api/tree")) {
          return new Response(
            JSON.stringify({
              name: "Vault",
              folders: [],
              notes: [
                { title: "Home", slug: "home" },
                { title: "Project", slug: "project" },
              ],
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/recently-modified")) {
          return new Response(
            JSON.stringify({
              notes: [
                {
                  title: "Project",
                  slug: "project",
                  relative_path: "Projects/Project",
                  mtime_ns: 30,
                },
                {
                  title: "Home",
                  slug: "home",
                  relative_path: "Home",
                  mtime_ns: 20,
                },
              ],
            }),
            { status: 200 },
          );
        }

        return new Response("not found", { status: 404 });
      },
    );

    render(
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    const modified = await screen.findByTestId("last-modified-notes");
    expect(within(modified).getByText("Last Modified")).toBeInTheDocument();
    expect(
      within(modified).getByRole("link", { name: "Project" }),
    ).toHaveAttribute("href", "/n/project");
    expect(
      within(modified).getByRole("link", { name: "Home" }),
    ).toHaveAttribute("title", "Home.md");
  });

  it("opens search and lists matches", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/api/tree")) {
          return new Response(
            JSON.stringify({
              name: "Vault",
              folders: [],
              notes: [{ title: "Home", slug: "home" }],
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/search")) {
          return new Response(
            JSON.stringify({
              mode: "semantic",
              results: [
                {
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
            }),
            { status: 200 },
          );
        }

        return new Response("not found", { status: 404 });
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
