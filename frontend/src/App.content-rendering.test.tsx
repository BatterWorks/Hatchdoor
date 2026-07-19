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

const mermaidInitialize = vi.fn();
const mermaidRender = vi.fn(async (id: string, chart: string) => ({
  svg: `<svg id="${id}" data-chart="${chart}"></svg>`,
}));

vi.mock("mermaid", () => ({
  default: {
    initialize: mermaidInitialize,
    render: mermaidRender,
  },
}));

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  vi.restoreAllMocks();
});

describe("App content rendering", () => {
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
          return new Response(
            JSON.stringify({ mode: "keyword", results: [] }),
            { status: 200 },
          );
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

    const input = await screen.findByPlaceholderText("Search notes…");
    expect(input).toHaveValue("type/reference");
    const includeContent = screen.getByRole("checkbox");
    expect(includeContent).toBeChecked();
    await waitFor(() => {
      const called = fetchMock.mock.calls.some((call) =>
        String(call[0]).includes("/api/search?q=type%2Freference&mode=keyword"),
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

    await screen.findByRole("heading", { level: 2, name: "Atlas" });
    await waitFor(() => {
      expect(document.querySelectorAll(".mermaid")).toHaveLength(2);
    });
    expect(mermaidInitialize).toHaveBeenCalledWith({
      startOnLoad: false,
      securityLevel: "strict",
      theme: "default",
      fontFamily: "Inter Tight, system-ui, sans-serif",
      themeVariables: {
        fontFamily: "Inter Tight, system-ui, sans-serif",
      },
    });
  });

  it("keeps note prose paragraph styles out of mermaid labels", async () => {
    mermaidRender.mockClear();
    mermaidRender.mockImplementation(async (id: string, chart: string) => ({
      svg: [
        `<svg id="${id}" data-chart="${chart}">`,
        '<foreignObject width="100" height="24">',
        "<div>",
        "<p>Diagram Label</p>",
        "</div>",
        "</foreignObject>",
        "</svg>",
      ].join(""),
    }));

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

    await screen.findByRole("heading", { level: 2, name: "Atlas" });
    const label = await waitFor(() => {
      const paragraph = document.querySelector(".mermaid foreignObject p");
      expect(paragraph).not.toBeNull();
      return paragraph as HTMLParagraphElement;
    });

    expect(getComputedStyle(label).marginBottom).toBe("0px");
  });

  it("waits for web fonts before rendering mermaid diagrams", async () => {
    mermaidRender.mockClear();

    let fontsReady = false;
    let resolveFonts: () => void = () => {};
    const fontReadyPromise = new Promise<void>((resolve) => {
      resolveFonts = () => {
        fontsReady = true;
        resolve();
      };
    });

    Object.defineProperty(document, "fonts", {
      configurable: true,
      value: {
        ready: fontReadyPromise,
      },
    });

    mermaidRender.mockImplementation(async (id: string, chart: string) => {
      expect(fontsReady).toBe(true);
      return { svg: `<svg id="${id}" data-chart="${chart}"></svg>` };
    });

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

    await screen.findByRole("heading", { level: 2, name: "Atlas" });
    await Promise.resolve();
    expect(mermaidRender).not.toHaveBeenCalled();

    resolveFonts();

    await waitFor(() => {
      expect(document.querySelectorAll(".mermaid")).toHaveLength(1);
    });
    expect(mermaidRender).toHaveBeenCalled();
  });
});
