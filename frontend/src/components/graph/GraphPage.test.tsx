import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setStoredScope } from "../../lib/storage";
import {
  collectionEnvelope,
  discoveryResponse,
  EIGHT_VAULTS,
  ONE_VAULT,
  participantFor,
  THREE_VAULTS,
  TWO_VAULTS,
} from "../../test/fixtures/vaults";
import type { GraphNode, VaultGraph, VaultSummary } from "../../types";
import { GraphPage } from "./GraphPage";

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function graphFor(vault: VaultSummary, nodeCount: number): VaultGraph {
  const nodes: GraphNode[] = Array.from({ length: nodeCount }, (_, i) => ({
    vault_id: vault.vault_id,
    slug: `note-${i}`,
    title: `Note ${i}`,
    primary_tag: null,
    backlink_count: 0,
  }));
  return {
    vault_id: vault.vault_id,
    vault_name: vault.name,
    nodes,
    edges: [],
  };
}

function mockDiscoveryAndGraph(vaults: VaultSummary[], graphEnvelope: unknown) {
  return vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith("/api/v1/vaults")) {
        return jsonResponse(discoveryResponse(vaults));
      }
      if (url.includes("/graph")) {
        return jsonResponse(graphEnvelope);
      }
      return jsonResponse({ error: "not found" });
    });
}

class ResizeObserverStub {
  observe() {}
  disconnect() {}
}

describe("GraphPage", () => {
  beforeEach(() => {
    vi.stubGlobal("ResizeObserver", ResizeObserverStub);
    vi.stubGlobal(
      "requestAnimationFrame",
      vi.fn(() => 1),
    );
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    window.localStorage.clear();
  });

  it("requests the graph and renders the backend error", async () => {
    // A fresh Response per call — GraphPage now also fetches vault discovery
    // (#143), and a single shared Response object's body can only be read
    // once across both calls.
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(
      async () =>
        new Response(
          JSON.stringify({
            code: "vault_read_unavailable",
            message: "Graph index is unavailable",
            retryable: true,
          }),
          {
            status: 503,
            headers: { "content-type": "application/json" },
          },
        ),
    );

    render(
      <MemoryRouter>
        <GraphPage />
      </MemoryRouter>,
    );

    expect(screen.getByText("Mapping your vault…")).toBeVisible();
    expect(
      await screen.findByRole("heading", { name: "Graph Unavailable" }),
    ).toBeVisible();
    expect(screen.getByText("Graph index is unavailable")).toBeVisible();
    // useVaultScope() defaults to "all" (nothing stored) — the collection
    // route is scoped by that value.
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/v1/vaults/all/graph",
      expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
  });
});

describe("GraphPage — all-Vault islands (#143)", () => {
  beforeEach(() => {
    vi.stubGlobal("ResizeObserver", ResizeObserverStub);
    vi.stubGlobal(
      "requestAnimationFrame",
      vi.fn(() => 1),
    );
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    window.localStorage.clear();
  });

  it("stays the single-Vault shape at one enabled Vault, even under all scope", async () => {
    mockDiscoveryAndGraph(
      ONE_VAULT,
      collectionEnvelope(
        "all",
        [graphFor(ONE_VAULT[0], 4)],
        [participantFor(ONE_VAULT[0], "fresh")],
      ),
    );

    render(
      <MemoryRouter>
        <GraphPage />
      </MemoryRouter>,
    );

    expect(await screen.findByText("Vault · Knowledge Graph")).toBeVisible();
    expect(
      screen.queryByText("ALL VAULTS · KNOWLEDGE GRAPH"),
    ).not.toBeInTheDocument();
  });

  it.each([
    ["two", TWO_VAULTS],
    ["three", THREE_VAULTS],
    ["eight", EIGHT_VAULTS],
  ] as const)(
    "draws the ALL VAULTS field with totals summed across islands, at %s Vaults",
    async (_label, vaults) => {
      const graphs = vaults.map((vault, i) => graphFor(vault, i + 1));
      mockDiscoveryAndGraph(
        vaults,
        collectionEnvelope(
          "all",
          graphs,
          vaults.map((vault) => participantFor(vault, "fresh")),
        ),
      );

      render(
        <MemoryRouter>
          <GraphPage />
        </MemoryRouter>,
      );

      expect(
        await screen.findByText("ALL VAULTS · KNOWLEDGE GRAPH"),
      ).toBeVisible();

      const expectedNodes = graphs.reduce((sum, g) => sum + g.nodes.length, 0);
      await waitFor(() => {
        const metaNums = document.querySelectorAll(".graph-meta-num");
        expect(metaNums[0]).toHaveTextContent(String(expectedNodes));
        expect(metaNums[1]).toHaveTextContent("0");
      });
      expect(screen.queryByText(/could not be drawn/)).not.toBeInTheDocument();
    },
  );

  it("names only the Vault that could not be drawn, in a warn-ink line, at three Vaults", async () => {
    const [alpha, beta, gamma] = THREE_VAULTS;
    mockDiscoveryAndGraph(
      THREE_VAULTS,
      collectionEnvelope(
        "all",
        [graphFor(alpha, 2), graphFor(beta, 3)],
        [
          participantFor(alpha, "fresh"),
          participantFor(beta, "fresh"),
          participantFor(gamma, "unavailable"),
        ],
      ),
    );

    render(
      <MemoryRouter>
        <GraphPage />
      </MemoryRouter>,
    );

    expect(
      await screen.findByText(`${gamma.name} could not be drawn.`),
    ).toHaveClass("graph-not-drawn");
  });

  it("names every Vault that could not be drawn, at eight Vaults", async () => {
    const drawn = EIGHT_VAULTS.slice(0, 6);
    const missing = EIGHT_VAULTS.slice(6);
    mockDiscoveryAndGraph(
      EIGHT_VAULTS,
      collectionEnvelope(
        "all",
        drawn.map((vault, i) => graphFor(vault, i + 1)),
        [
          ...drawn.map((vault) => participantFor(vault, "fresh")),
          ...missing.map((vault) => participantFor(vault, "unavailable")),
        ],
      ),
    );

    render(
      <MemoryRouter>
        <GraphPage />
      </MemoryRouter>,
    );

    expect(
      await screen.findByText(
        `${missing[0].name} and ${missing[1].name} could not be drawn.`,
      ),
    ).toBeVisible();
  });

  it("stays in island mode when only one of several enabled Vaults answered, naming the rest", async () => {
    const [alpha, beta, gamma] = THREE_VAULTS;
    mockDiscoveryAndGraph(
      THREE_VAULTS,
      collectionEnvelope(
        "all",
        [graphFor(alpha, 2)],
        [
          participantFor(alpha, "fresh"),
          participantFor(beta, "unavailable"),
          participantFor(gamma, "unavailable"),
        ],
      ),
    );

    render(
      <MemoryRouter>
        <GraphPage />
      </MemoryRouter>,
    );

    // A Vault going down doesn't collapse the shape back to the plain
    // single-graph page — it's still an island field, just with fewer
    // islands and both gaps named.
    expect(
      await screen.findByText("ALL VAULTS · KNOWLEDGE GRAPH"),
    ).toBeVisible();
    expect(
      await screen.findByText(
        `${beta.name} and ${gamma.name} could not be drawn.`,
      ),
    ).toBeVisible();
  });

  it("renders today's narrowed graph page unchanged when scope is one Vault", async () => {
    const [alpha] = THREE_VAULTS;
    setStoredScope(alpha.vault_id);
    mockDiscoveryAndGraph(
      THREE_VAULTS,
      collectionEnvelope(
        alpha.vault_id,
        [graphFor(alpha, 5)],
        [participantFor(alpha, "fresh")],
      ),
    );

    render(
      <MemoryRouter>
        <GraphPage />
      </MemoryRouter>,
    );

    expect(await screen.findByText("Vault · Knowledge Graph")).toBeVisible();
    expect(
      screen.queryByText("ALL VAULTS · KNOWLEDGE GRAPH"),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/could not be drawn/)).not.toBeInTheDocument();
  });
});

describe("GraphPage — settles instantly under prefers-reduced-motion (#147)", () => {
  beforeEach(() => {
    vi.stubGlobal("ResizeObserver", ResizeObserverStub);
    vi.stubGlobal(
      "requestAnimationFrame",
      vi.fn(() => 1),
    );
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    vi.stubGlobal("matchMedia", (query: string) => ({
      matches: query === "(prefers-reduced-motion: reduce)",
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }));
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    window.localStorage.clear();
  });

  it("still draws the plain single-graph page with reduced motion preferred", async () => {
    const [alpha] = THREE_VAULTS;
    setStoredScope(alpha.vault_id);
    mockDiscoveryAndGraph(
      THREE_VAULTS,
      collectionEnvelope(
        alpha.vault_id,
        [graphFor(alpha, 5)],
        [participantFor(alpha, "fresh")],
      ),
    );

    render(
      <MemoryRouter>
        <GraphPage />
      </MemoryRouter>,
    );

    expect(await screen.findByText("Vault · Knowledge Graph")).toBeVisible();
  });

  it("still draws the all-Vault island field with reduced motion preferred", async () => {
    const graphs = THREE_VAULTS.map((vault, i) => graphFor(vault, i + 1));
    mockDiscoveryAndGraph(
      THREE_VAULTS,
      collectionEnvelope(
        "all",
        graphs,
        THREE_VAULTS.map((vault) => participantFor(vault, "fresh")),
      ),
    );

    render(
      <MemoryRouter>
        <GraphPage />
      </MemoryRouter>,
    );

    expect(
      await screen.findByText("ALL VAULTS · KNOWLEDGE GRAPH"),
    ).toBeVisible();
  });
});
