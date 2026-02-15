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
import { escapeMarkdownLabel } from "./markdown";

afterEach(() => {
  cleanup();
  window.localStorage.clear();
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

    expect(
      await screen.findByRole("link", { name: "Home" }),
    ).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByRole("heading", { name: "Home" })).toBeInTheDocument();
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
                relative_path: "Home",
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

  it("renders callout blocks and code copy controls", async () => {
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
                content: `> [!warning] Heads up
>
> Callout body

\`\`\`ts
const x = 1
\`\`\``,
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
      const callout = document.querySelector(".callout-warning");
      expect(callout).not.toBeNull();
      expect(callout?.textContent).toContain("Heads up");
      expect(callout?.textContent).toContain("Callout body");
    });
    await waitFor(() => {
      const block = document.querySelector(".code-block");
      expect(block).not.toBeNull();
      expect(block?.textContent).toContain("const x = 1");
    });
    const writeTextMock = vi.fn().mockRejectedValue(new Error("unavailable"));
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: writeTextMock },
    });
    const execCommandMock = vi.fn((command: string) => command === "copy");
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: execCommandMock,
    });
    const copyButton = screen.getByRole("button", { name: "Copy" });
    fireEvent.click(copyButton);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Copied" }),
      ).toBeInTheDocument(),
    );
    expect(writeTextMock).toHaveBeenCalledWith("const x = 1");
    expect(execCommandMock).toHaveBeenCalledWith("copy");
    expect(document.querySelector(".note-content > pre")).toBeNull();
  });

  it("renders collapsible callouts from obsidian marker syntax", async () => {
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
                content: `> [!warning]- Hidden details
>
> Secret text`,
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
      const collapsible = document.querySelector(".callout-collapsible");
      expect(collapsible).not.toBeNull();
      expect(collapsible?.textContent).toContain("Hidden details");
      expect(collapsible).not.toHaveAttribute("open");
    });
  });

  it("renders frontmatter as compact properties instead of markdown body", async () => {
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
                content: `---
tags: [type/reference, status/active]
created: 2026-02-08
---

# Home
Body`,
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
      await screen.findByRole("button", { name: "Show" }),
    ).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Show" }));

    expect(await screen.findByText("tags")).toBeInTheDocument();
    expect(await screen.findByText("#type/reference")).toBeInTheDocument();
    expect(await screen.findByText("created")).toBeInTheDocument();
    expect(await screen.findByText("2026-02-08")).toBeInTheDocument();
    expect(screen.queryByText(/^---$/)).not.toBeInTheDocument();
    expect(screen.queryByText(/tags:\s*\[/)).not.toBeInTheDocument();
  });

  it("opens search filtered by tag when clicking tag chip", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation(async (input: RequestInfo | URL) => {
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
                content: `---
tags: [type/reference]
---

# Home
Body`,
              },
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/search")) {
          return new Response(JSON.stringify({ results: [] }), { status: 200 });
        }

        if (url.includes("/api/resolve-batch")) {
          return new Response(JSON.stringify({ results: [] }), { status: 200 });
        }

        return new Response("not found", { status: 404 });
      });

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    fireEvent.click(await screen.findByRole("button", { name: "Show" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "#type/reference" }),
    );

    const input = await screen.findByPlaceholderText(
      "Search notes (title, path, content)",
    );
    expect(input).toHaveValue("type/reference");
    const includeContent = screen.getByRole("checkbox");
    expect(includeContent).toBeChecked();
    await waitFor(() => {
      const called = fetchMock.mock.calls.some((call) =>
        String(call[0]).includes("/api/search?q=type%2Freference&content=true"),
      );
      expect(called).toBe(true);
    });
  });

  it("renders note-local markdown images via vault-assets path", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/api/tree")) {
          return new Response(
            JSON.stringify({
              name: "Vault",
              folders: [],
              notes: [{ title: "Atlas", slug: "atlas" }],
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/note/atlas")) {
          return new Response(
            JSON.stringify({
              note: {
                title: "Atlas",
                slug: "atlas",
                relative_path: "Notes/40-reference/Homelab Atlas",
                content: "![Stack](Media-stack.png)",
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
      <MemoryRouter initialEntries={["/n/atlas"]}>
        <App />
      </MemoryRouter>,
    );

    const image = await screen.findByRole("img", { name: "Stack" });
    expect(image).toHaveAttribute(
      "src",
      "/vault-assets/Notes/40-reference/Media-stack.png",
    );
  });

  it("renders note-local obsidian image embeds via vault-assets path", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/api/tree")) {
          return new Response(
            JSON.stringify({
              name: "Vault",
              folders: [],
              notes: [{ title: "Atlas", slug: "atlas" }],
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/note/atlas")) {
          return new Response(
            JSON.stringify({
              note: {
                title: "Atlas",
                slug: "atlas",
                relative_path: "Notes/40-reference/Homelab Atlas",
                content: "![[Media-stack.png]]",
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
      <MemoryRouter initialEntries={["/n/atlas"]}>
        <App />
      </MemoryRouter>,
    );

    const image = await screen.findByRole("img", { name: "Media-stack.png" });
    expect(image).toHaveAttribute(
      "src",
      "/vault-assets/Notes/40-reference/Media-stack.png",
    );
  });

  it("renders multiple mermaid blocks in a single note", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/api/tree")) {
          return new Response(
            JSON.stringify({
              name: "Vault",
              folders: [],
              notes: [{ title: "Atlas", slug: "atlas" }],
            }),
            { status: 200 },
          );
        }

        if (url.includes("/api/note/atlas")) {
          return new Response(
            JSON.stringify({
              note: {
                title: "Atlas",
                slug: "atlas",
                relative_path: "Notes/40-reference/Homelab Atlas",
                content: [
                  "# Atlas",
                  "",
                  "```mermaid",
                  "graph TD",
                  "A-->B",
                  "```",
                  "",
                  "```mermaid",
                  "flowchart LR",
                  "X-->Y",
                  "```",
                ].join("\n"),
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
      <MemoryRouter initialEntries={["/n/atlas"]}>
        <App />
      </MemoryRouter>,
    );

    await screen.findByRole("heading", { name: "Atlas" });
    await waitFor(() => {
      expect(document.querySelectorAll(".mermaid")).toHaveLength(2);
    });
  });
});
