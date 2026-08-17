import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { StrictMode } from "react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { VaultApp as App } from "./App";
import { discoveryResponse, healthyVault } from "./test/fixtures/vaults";

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

describe("App content rendering", () => {
  it("renders callout blocks and code copy controls", async () => {
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

        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: VAULT_ID,
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
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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

  it("expands a closed callout whose body follows its marker on the next line", async () => {
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

        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: `> [!abstract]-
> A collapsible callout that starts **closed** (\`[!abstract]-\`). Click to expand.`,
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
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    await waitFor(() => {
      const collapsible = document.querySelector(".callout-collapsible");
      expect(collapsible).not.toBeNull();
      expect(collapsible?.textContent).toContain(
        "A collapsible callout that starts closed",
      );
      expect(collapsible).not.toHaveAttribute("open");
      expect(collapsible?.querySelector("summary")).toHaveTextContent(
        "Abstract",
      );
      expect(collapsible?.querySelector(".callout-body")).toHaveTextContent(
        "A collapsible callout that starts closed",
      );
    });

    fireEvent.click(document.querySelector(".callout-collapsible > summary")!);
    expect(document.querySelector(".callout-collapsible")).toHaveAttribute(
      "open",
    );
  });

  it("shows a Vault property row when more than one Vault is enabled, even with no frontmatter (#140)", async () => {
    const otherVault = healthyVault("Second Vault");
    vi.spyOn(globalThis, "fetch").mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = String(input);
        if (url.endsWith("/api/v1/vaults")) {
          return jsonResponse(discoveryResponse([VAULT, otherVault]));
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
        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "# Just a note\n\nNo frontmatter on this one.",
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
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    await waitFor(() => {
      const grid = document.querySelector(".note-properties-grid");
      expect(grid).not.toBeNull();
      const row = grid!.querySelector(".note-property-row");
      expect(row?.querySelector("dt")).toHaveTextContent("Vault");
      expect(row?.querySelector("dd")).toHaveTextContent(VAULT.name);
    });
  });

  it("keeps table-of-contents targets stable after the note re-renders", async () => {
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
              content: "# Home\n\n## Target section\n\nBody",
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
      <StrictMode>
        <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
          <App
            startupStatus={{ state: "ready" }}
            onRetryModelSetup={() => {}}
          />
        </MemoryRouter>
      </StrictMode>,
    );

    await waitFor(() =>
      expect(
        document.querySelector(".note-toc-desktop .note-toc-link"),
      ).not.toBeNull(),
    );
    const heading = document.getElementById("target-section");
    expect(heading).toHaveTextContent("Target section");
    const scrollIntoView = vi.fn();
    Object.defineProperty(heading!, "scrollIntoView", {
      value: scrollIntoView,
      configurable: true,
    });
    const tocTarget = Array.from(
      document.querySelectorAll<HTMLButtonElement>(
        ".note-toc-desktop .note-toc-link",
      ),
    ).find((button) => button.textContent === "Target section");
    expect(tocTarget).toBeDefined();
    fireEvent.click(tocTarget!);

    // The jump waits a frame so the trailing scroll space is in the DOM first;
    // without it the scroll clamps short of an end-of-note heading.
    await waitFor(() =>
      expect(scrollIntoView).toHaveBeenCalledWith({
        behavior: "smooth",
        block: "start",
        inline: "nearest",
      }),
    );
    expect(document.querySelector(".note-content")).toHaveAttribute(
      "data-tail",
      "true",
    );
  });

  it("renders frontmatter as compact properties instead of markdown body", async () => {
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

        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: VAULT_ID,
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
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    // The Properties heading is the disclosure now; there is no Show button.
    const toggle = await screen.findByRole("button", { name: "Properties" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");

    // The grid stays mounted while collapsed so aria-controls has a target,
    // so these assert visibility rather than mere presence.
    expect(screen.getByText("tags")).not.toBeVisible();

    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");

    expect(await screen.findByText("tags")).toBeVisible();
    expect(await screen.findByText("#type/reference")).toBeVisible();
    expect(await screen.findByText("created")).toBeVisible();
    expect(await screen.findByText("2026-02-08")).toBeVisible();
    expect(screen.queryByText(/^---$/)).not.toBeInTheDocument();
    expect(screen.queryByText(/tags:\s*\[/)).not.toBeInTheDocument();
  });

  it("opens search filtered by tag when clicking tag chip", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation(async (input: RequestInfo | URL) => {
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

        if (url.includes("/notes/home")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: `---
tags: [type/reference]
---

# Home
Body`,
              content_hash: "hash",
              layer: null,
            },
          });
        }

        if (url.includes("/search")) {
          return collectionEnvelope({ mode: "keyword", results: [] });
        }

        if (url.includes("/resolve-batch")) {
          return jsonResponse({ vault_id: VAULT_ID, results: [] });
        }

        return jsonResponse({ error: "not found" }, 404);
      });

    render(
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/home`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    fireEvent.click(await screen.findByRole("button", { name: "Properties" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "#type/reference" }),
    );

    const input = await screen.findByPlaceholderText("Search notes…");
    expect(input).toHaveValue("#type/reference");
    const includeContent = screen.getByRole("checkbox");
    expect(includeContent).toBeChecked();
    await waitFor(() => {
      const called = fetchMock.mock.calls.some((call) =>
        String(call[0]).includes(
          "/api/v1/vaults/all/search?q=%23type%2Freference&mode=keyword",
        ),
      );
      expect(called).toBe(true);
    });
  });

  it("renders note-local markdown images via the vault asset API", async () => {
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
                notes: [{ vault_id: VAULT_ID, title: "Atlas", slug: "atlas" }],
              },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }

        if (url.includes("/notes/atlas")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Atlas",
              slug: "atlas",
              relative_path: "Notes/40-reference/Homelab Atlas",
              content: "![Stack](Media-stack.png)",
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
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/atlas`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    const image = await screen.findByRole("img", { name: "Stack" });
    expect(image).toHaveAttribute(
      "src",
      `/api/v1/vaults/${VAULT_ID}/assets/Notes/40-reference/Media-stack.png`,
    );
  });

  it("renders note-local obsidian image embeds via the vault asset API", async () => {
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
                notes: [{ vault_id: VAULT_ID, title: "Atlas", slug: "atlas" }],
              },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }

        if (url.includes("/notes/atlas")) {
          return jsonResponse({
            vault_id: VAULT_ID,
            note: {
              title: "Atlas",
              slug: "atlas",
              relative_path: "Notes/40-reference/Homelab Atlas",
              content: "![[Media-stack.png]]",
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
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/atlas`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
      </MemoryRouter>,
    );

    const image = await screen.findByRole("img", { name: "Media-stack.png" });
    expect(image).toHaveAttribute(
      "src",
      `/api/v1/vaults/${VAULT_ID}/assets/Notes/40-reference/Media-stack.png`,
    );
  });

  it("renders multiple mermaid blocks in a single note", async () => {
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
                notes: [{ vault_id: VAULT_ID, title: "Atlas", slug: "atlas" }],
              },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }

        if (url.includes("/notes/atlas")) {
          return jsonResponse({
            vault_id: VAULT_ID,
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
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/atlas`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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
                notes: [{ vault_id: VAULT_ID, title: "Atlas", slug: "atlas" }],
              },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }

        if (url.includes("/notes/atlas")) {
          return jsonResponse({
            vault_id: VAULT_ID,
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
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/atlas`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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
                notes: [{ vault_id: VAULT_ID, title: "Atlas", slug: "atlas" }],
              },
            },
          ]);
        }
        if (url.includes("/recent")) {
          return collectionEnvelope([]);
        }

        if (url.includes("/notes/atlas")) {
          return jsonResponse({
            vault_id: VAULT_ID,
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
      <MemoryRouter initialEntries={[`/v/${VAULT_ID}/n/atlas`]}>
        <App startupStatus={{ state: "ready" }} onRetryModelSetup={() => {}} />
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
