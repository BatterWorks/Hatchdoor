import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import App from "./App";

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe("App enhancements", () => {
  it("persists expanded folders in localStorage", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(
        JSON.stringify({
          name: "Vault",
          folders: [{ name: "Projects", folders: [], notes: [] }],
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

        if (url.includes("/api/note/home/links")) {
          return new Response(
            JSON.stringify({
              links: {
                outgoing: [
                  { title: "Plan", slug: "plan", relative_path: "Plan" },
                ],
                backlinks: [
                  {
                    title: "Overview",
                    slug: "overview",
                    relative_path: "Overview",
                  },
                ],
              },
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
                content: "# Intro\n\n## Section",
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
    window.localStorage.setItem("hatchdoor.lastNote", "home");

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

        if (url.includes("/api/note/home/links")) {
          return new Response(
            JSON.stringify({ links: { outgoing: [], backlinks: [] } }),
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
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    expect(
      await screen.findByRole("heading", { name: "Home" }),
    ).toBeInTheDocument();
  });

  it("highlights in-note matches after opening a search result", async () => {
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
                  title: "Home",
                  slug: "home",
                  relative_path: "Home",
                  match_kind: "content",
                  snippet: "token found here",
                },
              ],
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/note/home/links")) {
          return new Response(
            JSON.stringify({ links: { outgoing: [], backlinks: [] } }),
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
                content: "This line has token and another token.",
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
      <MemoryRouter initialEntries={["/"]}>
        <App />
      </MemoryRouter>,
    );

    fireEvent.click(await screen.findByRole("button", { name: "Search" }));
    const input = await screen.findByPlaceholderText(
      "Search notes (title, path, content)",
    );
    fireEvent.change(input, { target: { value: "token" } });

    fireEvent.click(await screen.findByRole("button", { name: /Home/ }));

    expect(await screen.findByText(/Match 1 of 2/)).toBeInTheDocument();
    expect(document.querySelectorAll("mark.search-hit")).toHaveLength(2);
  });
});
