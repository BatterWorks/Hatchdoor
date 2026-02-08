import { render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import App from "./App";
import { escapeMarkdownLabel } from "./markdown";

afterEach(() => {
  vi.restoreAllMocks();
});

describe("App", () => {
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

    expect(await screen.findByText("Notes Explorer")).toBeInTheDocument();
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

    expect(
      await screen.findByRole("link", { name: "Home" }),
    ).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByRole("heading", { name: "Home" })).toBeInTheDocument();
    });
  });

  it("renders unresolved wikilinks as broken links", async () => {
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
                content: "Missing [[Nope|Alias Label]]",
              },
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/resolve-batch")) {
          return new Response(
            JSON.stringify({
              results: [{ target: "Nope", slug: null }],
            }),
            { status: 200 },
          );
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
      const broken = screen.getByText("Alias Label");
      expect(broken).toHaveClass("broken-link");
      expect(broken).toHaveAttribute("title", "Missing: Nope");
    });
  });

  it("escapes markdown control chars in wikilink labels", () => {
    expect(escapeMarkdownLabel("a]b(c) *x*")).toBe("a\\]b\\(c\\) \\*x\\*");
  });
});
