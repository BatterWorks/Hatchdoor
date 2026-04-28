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

  it("renders folders collapsed by default on explorer root", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
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
    await waitFor(() => {
      expect(
        within(recent).getByRole("link", { name: "Home" }),
      ).toBeInTheDocument();
    });
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
              results: [
                {
                  title: "Plan",
                  slug: "plan",
                  relative_path: "Projects/Plan",
                  match_kind: "title",
                  snippet: null,
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
    const input = await screen.findByPlaceholderText(
      "Search notes (title, path, content)",
    );
    fireEvent.change(input, { target: { value: "plan" } });

    expect(
      await screen.findByText(
        (_value, element) => element?.textContent === "Projects/Plan.md",
      ),
    ).toBeInTheDocument();
    expect(await screen.findByText("title")).toBeInTheDocument();
  });
});
