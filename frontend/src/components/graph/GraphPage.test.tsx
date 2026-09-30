import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import type { Simulation } from "d3-force";
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
import * as graphSimulation from "./graphSimulation";
import type { SimLink, SimNode } from "./graphSimulation";

// Every simulation the page builds, so a test can count rebuilds and reach the
// live nodes. The real constructors run underneath.
vi.mock("./graphSimulation", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./graphSimulation")>();
  return {
    ...actual,
    createGraphSimulation: vi.fn(actual.createGraphSimulation),
    createIslandSimulation: vi.fn(actual.createIslandSimulation),
  };
});

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

// ── stability (#336) ─────────────────────────────────────────────────────────

type Sim = Simulation<SimNode, SimLink>;

function builtSimulations(): Sim[] {
  return [
    ...vi.mocked(graphSimulation.createGraphSimulation).mock.results,
    ...vi.mocked(graphSimulation.createIslandSimulation).mock.results,
  ].map((result) => result.value as Sim);
}

function emitRevision(revision: number) {
  for (const source of window.__hatchdoorEventSources) {
    if (source.url.includes("/api/v1/vaults/events")) {
      source.emit(
        "vault-collection-revision",
        JSON.stringify({ collection_revision: revision }),
      );
    }
  }
}

/** A graph route whose answer the test can change between reads, behind a
 * discovery route that can change too. */
function mockLiveCollection(initial: {
  vaults: VaultSummary[];
  graphs: VaultGraph[];
}) {
  const live = { ...initial };
  const graphReads = { count: 0 };
  vi.spyOn(globalThis, "fetch").mockImplementation(
    async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.endsWith("/api/v1/vaults")) {
        return jsonResponse(discoveryResponse(live.vaults));
      }
      if (url.includes("/graph")) {
        graphReads.count += 1;
        return jsonResponse(
          collectionEnvelope(
            "all",
            live.graphs,
            live.vaults.map((vault) => participantFor(vault, "fresh")),
          ),
        );
      }
      return jsonResponse({ error: "not found" });
    },
  );
  return { live, graphReads };
}

function touchEvent(
  type: string,
  touches: { clientX: number; clientY: number }[],
  changed = touches,
): Event {
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(event, "touches", { value: touches });
  Object.defineProperty(event, "changedTouches", { value: changed });
  return event;
}

function canvas(): HTMLCanvasElement {
  const element = document.querySelector("canvas");
  if (!element) throw new Error("no canvas");
  return element;
}

describe("GraphPage — stability (#336)", () => {
  let resizeCallbacks: (() => void)[];

  beforeEach(() => {
    resizeCallbacks = [];
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(callback: () => void) {
          resizeCallbacks.push(callback);
        }
        observe() {}
        disconnect() {}
      },
    );
    vi.stubGlobal(
      "requestAnimationFrame",
      vi.fn(() => 1),
    );
    vi.stubGlobal("cancelAnimationFrame", vi.fn());
    vi.mocked(graphSimulation.createGraphSimulation).mockClear();
    vi.mocked(graphSimulation.createIslandSimulation).mockClear();
  });

  afterEach(() => {
    for (const sim of builtSimulations()) sim.stop();
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    window.localStorage.clear();
  });

  it("folds a note write into the live layout instead of rebuilding it", async () => {
    const [solo] = ONE_VAULT;
    const { live, graphReads } = mockLiveCollection({
      vaults: ONE_VAULT,
      graphs: [graphFor(solo, 3)],
    });

    render(
      <MemoryRouter>
        <GraphPage />
      </MemoryRouter>,
    );
    await waitFor(() =>
      expect(document.querySelector(".graph-meta-num")).toHaveTextContent("3"),
    );
    await waitFor(() => expect(builtSimulations()).toHaveLength(1));
    const [sim] = builtSimulations();
    const before = [...sim.nodes()];
    before[0].x = 321;
    const readsBefore = graphReads.count;

    // A note is written: the index turn moves the collection revision, and
    // the graph now has a fourth note.
    live.graphs = [graphFor(solo, 4)];
    act(() => emitRevision(99));

    await waitFor(() =>
      expect(document.querySelector(".graph-meta-num")).toHaveTextContent("4"),
    );
    expect(graphReads.count).toBeGreaterThan(readsBefore);
    // Same simulation, same node objects, same positions.
    expect(builtSimulations()).toHaveLength(1);
    await waitFor(() => expect(sim.nodes()).toHaveLength(4));
    expect(sim.nodes().slice(0, 3)).toEqual(before);
    expect(sim.nodes()[0]).toBe(before[0]);
  });

  it("repaints island captions on a Vault status change without rebuilding the field", async () => {
    const [alpha, beta] = TWO_VAULTS;
    const { live } = mockLiveCollection({
      vaults: TWO_VAULTS,
      graphs: [graphFor(alpha, 2), graphFor(beta, 3)],
    });

    render(
      <MemoryRouter>
        <GraphPage />
      </MemoryRouter>,
    );
    expect(
      await screen.findByText("ALL VAULTS · KNOWLEDGE GRAPH"),
    ).toBeVisible();
    await waitFor(() => expect(builtSimulations()).toHaveLength(1));

    // Beta's index turn: `search` goes `indexing` and back, twice publishing
    // a new Vault list with identical graph data.
    live.vaults = [alpha, { ...beta, search: "indexing" }];
    act(() => emitRevision(50));
    await waitFor(() =>
      expect(
        vi
          .mocked(globalThis.fetch)
          .mock.calls.filter(([url]) => String(url).includes("/graph")).length,
      ).toBeGreaterThanOrEqual(2),
    );
    live.vaults = [alpha, beta];
    act(() => emitRevision(51));
    await waitFor(() =>
      expect(
        vi
          .mocked(globalThis.fetch)
          .mock.calls.filter(([url]) => String(url).includes("/graph")).length,
      ).toBeGreaterThanOrEqual(3),
    );
    await new Promise((resolve) => setTimeout(resolve, 20));

    expect(builtSimulations()).toHaveLength(1);
  });

  describe("drag lifecycle", () => {
    // One node, pinned by the stubbed scatter at world (0,0), which the page
    // maps to canvas (0,0) here (jsdom lays nothing out).
    async function renderOneNode(): Promise<{ sim: Sim; node: SimNode }> {
      vi.spyOn(Math, "random").mockReturnValue(0.5);
      mockLiveCollection({
        vaults: ONE_VAULT,
        graphs: [graphFor(ONE_VAULT[0], 1)],
      });
      render(
        <MemoryRouter>
          <GraphPage />
        </MemoryRouter>,
      );
      await waitFor(() => expect(builtSimulations()).toHaveLength(1));
      const [sim] = builtSimulations();
      return { sim, node: sim.nodes()[0] };
    }

    function expectReleased(sim: Sim, node: SimNode) {
      expect(node.fx).toBeNull();
      expect(node.fy).toBeNull();
      expect(sim.alphaTarget()).toBe(0);
    }

    it("releases a mouse-dragged node when the pointer leaves with no button held", async () => {
      const { sim, node } = await renderOneNode();
      fireEvent.mouseDown(canvas(), { clientX: 0, clientY: 0, button: 0 });
      fireEvent.mouseMove(window, { clientX: 20, clientY: 20, buttons: 1 });
      expect(node.fx).toBeCloseTo(20 / 0.9);
      expect(sim.alphaTarget()).toBeGreaterThan(0);

      fireEvent.mouseLeave(canvas(), { buttons: 0 });

      expectReleased(sim, node);
    });

    it("keeps following a drag that leaves the canvas with the button held, then releases on mouseup", async () => {
      const { sim, node } = await renderOneNode();
      fireEvent.mouseDown(canvas(), { clientX: 0, clientY: 0, button: 0 });
      fireEvent.mouseLeave(canvas(), { buttons: 1 });
      fireEvent.mouseMove(window, { clientX: 45, clientY: 0, buttons: 1 });
      expect(node.fx).toBeCloseTo(50);

      fireEvent.mouseUp(window, { clientX: 45, clientY: 0 });

      expectReleased(sim, node);
    });

    it("releases a touch-dragged node when a second finger starts a pinch", async () => {
      const { sim, node } = await renderOneNode();
      canvas().dispatchEvent(
        touchEvent("touchstart", [{ clientX: 0, clientY: 0 }]),
      );
      canvas().dispatchEvent(
        touchEvent("touchmove", [{ clientX: 20, clientY: 20 }]),
      );
      expect(sim.alphaTarget()).toBeGreaterThan(0);

      canvas().dispatchEvent(
        touchEvent("touchstart", [
          { clientX: 20, clientY: 20 },
          { clientX: 120, clientY: 120 },
        ]),
      );

      expectReleased(sim, node);
    });

    it("releases a touch-dragged node when the system cancels the touch", async () => {
      const { sim, node } = await renderOneNode();
      canvas().dispatchEvent(
        touchEvent("touchstart", [{ clientX: 0, clientY: 0 }]),
      );
      canvas().dispatchEvent(
        touchEvent("touchmove", [{ clientX: 20, clientY: 20 }]),
      );
      expect(sim.alphaTarget()).toBeGreaterThan(0);

      canvas().dispatchEvent(
        touchEvent("touchcancel", [], [{ clientX: 20, clientY: 20 }]),
      );

      expectReleased(sim, node);
    });
  });

  describe("resize and density", () => {
    let size: { w: number; h: number };

    beforeEach(() => {
      size = { w: 400, h: 300 };
      vi.spyOn(
        HTMLElement.prototype,
        "getBoundingClientRect",
      ).mockImplementation(function (this: HTMLElement) {
        const laidOut = this.classList.contains("graph-canvas-wrap");
        return {
          x: 0,
          y: 0,
          left: 0,
          top: 0,
          width: laidOut ? size.w : 0,
          height: laidOut ? size.h : 0,
          right: laidOut ? size.w : 0,
          bottom: laidOut ? size.h : 0,
          toJSON: () => ({}),
        };
      });
      vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(
        () => size.w,
      );
      vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockImplementation(
        () => size.h,
      );
    });

    it("keeps the centre of the view in view when the canvas is rotated", async () => {
      vi.spyOn(Math, "random").mockReturnValue(0.5);
      mockLiveCollection({
        vaults: ONE_VAULT,
        graphs: [graphFor(ONE_VAULT[0], 1)],
      });
      render(
        <MemoryRouter>
          <GraphPage />
        </MemoryRouter>,
      );
      await waitFor(() => expect(builtSimulations()).toHaveLength(1));
      const [sim] = builtSimulations();
      const node = sim.nodes()[0];

      // Landscape to portrait: the node at world (0,0) sat at the old centre.
      size = { w: 300, h: 400 };
      act(() => resizeCallbacks.forEach((callback) => callback()));

      // It is now at the new centre, where a press lands on it.
      fireEvent.mouseDown(canvas(), { clientX: 150, clientY: 200, button: 0 });
      fireEvent.mouseMove(window, { clientX: 150, clientY: 200, buttons: 1 });
      expect(node.fx).toBeCloseTo(0);
      expect(node.fy).toBeCloseTo(0);
      fireEvent.mouseUp(window, { clientX: 150, clientY: 200 });
    });

    it("re-sizes the canvas buffer when the display density changes", async () => {
      const dprListeners: (() => void)[] = [];
      vi.stubGlobal("matchMedia", (query: string) => ({
        matches: false,
        media: query,
        onchange: null,
        addEventListener: (_type: string, listener: () => void) => {
          if (query.startsWith("(resolution")) dprListeners.push(listener);
        },
        removeEventListener: () => {},
        dispatchEvent: () => false,
      }));
      vi.stubGlobal("devicePixelRatio", 1);
      mockLiveCollection({
        vaults: ONE_VAULT,
        graphs: [graphFor(ONE_VAULT[0], 1)],
      });
      render(
        <MemoryRouter>
          <GraphPage />
        </MemoryRouter>,
      );
      await waitFor(() => expect(canvas().width).toBe(400));

      // Moved to a 2x display: no CSS size change, so no resize observation.
      vi.stubGlobal("devicePixelRatio", 2);
      act(() => dprListeners.forEach((listener) => listener()));

      expect(canvas().width).toBe(800);
      expect(canvas().height).toBe(600);
    });
  });

  describe("an empty graph", () => {
    it("says the Vault has no notes yet", async () => {
      mockLiveCollection({
        vaults: ONE_VAULT,
        graphs: [graphFor(ONE_VAULT[0], 0)],
      });
      render(
        <MemoryRouter>
          <GraphPage />
        </MemoryRouter>,
      );
      expect(
        await screen.findByRole("heading", { name: "No Notes Yet" }),
      ).toBeVisible();
      expect(screen.getByText(/This Vault has no notes yet/)).toBeVisible();
    });

    it("says which Vault is still being indexed rather than that it is empty", async () => {
      const indexing = { ...ONE_VAULT[0], search: "indexing" as const };
      mockLiveCollection({
        vaults: [indexing],
        graphs: [graphFor(indexing, 0)],
      });
      render(
        <MemoryRouter>
          <GraphPage />
        </MemoryRouter>,
      );
      expect(
        await screen.findByText(`${indexing.name} is still being indexed.`),
      ).toBeVisible();
    });

    it("draws no empty-state block over a graph with notes", async () => {
      mockLiveCollection({
        vaults: ONE_VAULT,
        graphs: [graphFor(ONE_VAULT[0], 2)],
      });
      render(
        <MemoryRouter>
          <GraphPage />
        </MemoryRouter>,
      );
      await waitFor(() =>
        expect(document.querySelector(".graph-meta-num")).toHaveTextContent(
          "2",
        ),
      );
      expect(
        screen.queryByRole("heading", { name: "No Notes Yet" }),
      ).toBeNull();
    });
  });
});
