import { describe, expect, it, vi } from "vitest";

import type { GraphData, VaultGraph } from "../../types";
import {
  buildIslandGraphs,
  buildSimulationGraph,
  computeIslandCenters,
  islandRadiusEstimate,
  createGraphSimulation,
  createIslandSimulation,
  circleInView,
  fitIslandField,
  hitTest,
  islandCaptionMetrics,
  islandCountLine,
  ISLAND_FIT_MIN_SCALE,
  LABEL_BUDGET,
  labelText,
  layoutLabels,
  nodeKey,
  nodeRadius,
  normalizeWheelDelta,
  REFRESH_ALPHA,
  segmentInView,
  TOUCH_HIT_TARGET,
  worldViewport,
  replaceSimulationGraph,
  settleSimulationSync,
  type SimNode,
} from "./graphSimulation";

function sequenceRandom(values: number[]): () => number {
  let i = 0;
  return () => values[i++ % values.length];
}

const VAULT_ID = "vault-1";

const DATA: GraphData = {
  nodes: [
    {
      vault_id: VAULT_ID,
      slug: "a",
      title: "Alpha",
      primary_tag: "topic/x",
      backlink_count: 3,
    },
    {
      vault_id: VAULT_ID,
      slug: "b",
      title: "Bravo",
      primary_tag: null,
      backlink_count: 0,
    },
    {
      vault_id: VAULT_ID,
      slug: "c",
      title: "Charlie",
      primary_tag: "topic/y",
      backlink_count: 1,
    },
  ],
  edges: [
    { vault_id: VAULT_ID, source_slug: "a", target_slug: "b" },
    { vault_id: VAULT_ID, source_slug: "b", target_slug: "c" },
    { vault_id: VAULT_ID, source_slug: "a", target_slug: "ghost" }, // dangler — target missing
  ],
};

describe("nodeRadius", () => {
  it("is the base radius at zero backlinks and grows monotonically", () => {
    expect(nodeRadius(0)).toBeCloseTo(4);
    expect(nodeRadius(1)).toBeGreaterThan(nodeRadius(0));
    expect(nodeRadius(50)).toBeGreaterThan(nodeRadius(10));
  });
});

describe("buildSimulationGraph", () => {
  it("creates one node per datum and drops links with a missing endpoint", () => {
    const { nodes, links } = buildSimulationGraph(DATA, {
      random: sequenceRandom([0.5]),
    });
    expect(nodes.map((n) => n.slug)).toEqual(["a", "b", "c"]);
    // The a→ghost edge is dropped; a→b and b→c survive.
    expect(links).toHaveLength(2);
    expect(links.map((l) => `${l.source.slug}->${l.target.slug}`)).toEqual([
      "a->b",
      "b->c",
    ]);
  });

  it("resolves link endpoints to the same node objects as the node list", () => {
    const { nodes, links } = buildSimulationGraph(DATA, {
      random: sequenceRandom([0.5]),
    });
    const a = nodes.find((n) => n.slug === "a");
    expect(links[0].source).toBe(a);
  });

  it("scatters nodes deterministically around the origin using injected random", () => {
    const { nodes } = buildSimulationGraph(DATA, {
      spread: 100,
      random: sequenceRandom([0, 1]), // x=(0-0.5)*100=-50, y=(1-0.5)*100=50
    });
    expect(nodes[0].x).toBeCloseTo(-50);
    expect(nodes[0].y).toBeCloseTo(50);
  });
});

describe("buildSimulationGraph — refreshing a live layout (#336)", () => {
  function byKey(nodes: SimNode[]): Map<string, SimNode> {
    return new Map(nodes.map((n) => [nodeKey(n), n]));
  }

  it("reuses the live node objects, keeping their positions and refreshing their fields", () => {
    const first = buildSimulationGraph(DATA, {
      random: sequenceRandom([0.1, 0.9]),
    });
    const alpha = first.nodes[0];
    alpha.x = 123;
    alpha.y = -45;
    alpha.fx = 123;
    const renamed: GraphData = {
      ...DATA,
      nodes: DATA.nodes.map((n) =>
        n.slug === "a" ? { ...n, title: "Alpha 2", backlink_count: 9 } : n,
      ),
    };

    const second = buildSimulationGraph(renamed, {
      random: sequenceRandom([0.5]),
      previous: byKey(first.nodes),
    });

    expect(second.nodes[0]).toBe(alpha);
    expect(alpha).toMatchObject({ x: 123, y: -45, fx: 123 });
    expect(alpha.title).toBe("Alpha 2");
    expect(alpha.backlink_count).toBe(9);
    expect(second.links[0].source).toBe(alpha);
  });

  it("places a new note beside the live note it links to, not at a random spot", () => {
    const first = buildSimulationGraph(DATA, { random: sequenceRandom([0.5]) });
    const bravo = first.nodes[1];
    bravo.x = 400;
    bravo.y = 300;
    const grown: GraphData = {
      nodes: [
        ...DATA.nodes,
        {
          vault_id: VAULT_ID,
          slug: "d",
          title: "Delta",
          primary_tag: null,
          backlink_count: 0,
        },
      ],
      edges: [
        ...DATA.edges,
        { vault_id: VAULT_ID, source_slug: "d", target_slug: "b" },
      ],
    };

    const second = buildSimulationGraph(grown, {
      random: sequenceRandom([0.0]),
      previous: byKey(first.nodes),
    });

    const delta = second.nodes[3];
    expect(Math.abs(delta.x - 400)).toBeLessThanOrEqual(15);
    expect(Math.abs(delta.y - 300)).toBeLessThanOrEqual(15);
    expect(second.nodes.slice(0, 3)).toEqual(first.nodes);
  });

  it("keeps island nodes where they are across a refresh", () => {
    const first = buildIslandGraphs(VAULT_GRAPHS, {
      random: sequenceRandom([0.2, 0.7]),
    });
    const before = first.nodes.map((n) => ({ x: n.x, y: n.y }));
    const second = buildIslandGraphs(VAULT_GRAPHS, {
      random: sequenceRandom([0.5]),
      previous: byKey(first.nodes),
    });
    second.nodes.forEach((n, i) => {
      expect(n).toBe(first.nodes[i]);
      expect({ x: n.x, y: n.y }).toEqual(before[i]);
    });
  });
});

describe("replaceSimulationGraph (#336)", () => {
  it("swaps nodes and links into the same simulation and re-warms it gently", () => {
    const first = buildSimulationGraph(DATA, {
      random: sequenceRandom([0.3, 0.6]),
    });
    const sim = createGraphSimulation(first.nodes, first.links);
    try {
      settleSimulationSync(sim);
      const next = buildSimulationGraph(
        { nodes: DATA.nodes.slice(0, 2), edges: DATA.edges.slice(0, 1) },
        { previous: new Map(first.nodes.map((n) => [nodeKey(n), n])) },
      );

      replaceSimulationGraph(sim, next.nodes, next.links);

      expect(sim.nodes()).toEqual(next.nodes);
      expect(sim.nodes()[0]).toBe(first.nodes[0]);
      const linkForce = sim.force("link") as unknown as { links(): unknown[] };
      expect(linkForce.links()).toEqual(next.links);
      expect(sim.alpha()).toBeCloseTo(REFRESH_ALPHA);
      settleSimulationSync(sim);
      expect(Number.isFinite(sim.nodes()[0].x)).toBe(true);
    } finally {
      sim.stop();
    }
  });
});

describe("createGraphSimulation", () => {
  it("registers the link, charge, center, and collide forces", () => {
    const { nodes, links } = buildSimulationGraph(DATA, {
      random: sequenceRandom([0.5]),
    });
    const sim = createGraphSimulation(nodes, links);
    try {
      expect(sim.force("link")).toBeTruthy();
      expect(sim.force("charge")).toBeTruthy();
      expect(sim.force("center")).toBeTruthy();
      expect(sim.force("collide")).toBeTruthy();
      expect(sim.nodes()).toHaveLength(3);
    } finally {
      sim.stop();
    }
  });
});

describe("settleSimulationSync", () => {
  it("stops the simulation's own timer and leaves it at rest", () => {
    const { nodes, links } = buildSimulationGraph(DATA, {
      random: sequenceRandom([0.5]),
    });
    const sim = createGraphSimulation(nodes, links);
    try {
      settleSimulationSync(sim);
      expect(sim.alpha()).toBeLessThanOrEqual(sim.alphaMin());
    } finally {
      sim.stop();
    }
  });

  it("moves nodes off their initial scatter position — the layout actually ran", () => {
    const { nodes, links } = buildSimulationGraph(DATA, {
      random: sequenceRandom([0.1, 0.9, 0.3]),
    });
    const initial = nodes.map((n) => ({ x: n.x, y: n.y }));
    const sim = createGraphSimulation(nodes, links);
    try {
      settleSimulationSync(sim);
      const moved = nodes.some(
        (n, i) => n.x !== initial[i].x || n.y !== initial[i].y,
      );
      expect(moved).toBe(true);
      for (const n of nodes) {
        expect(Number.isFinite(n.x)).toBe(true);
        expect(Number.isFinite(n.y)).toBe(true);
      }
    } finally {
      sim.stop();
    }
  });

  it("leaves no running timer behind — node positions hold after it returns", () => {
    vi.useFakeTimers();
    try {
      const { nodes, links } = buildSimulationGraph(DATA, {
        random: sequenceRandom([0.1, 0.9, 0.3]),
      });
      const sim = createGraphSimulation(nodes, links);
      try {
        settleSimulationSync(sim);
        const settled = nodes.map((n) => ({ x: n.x, y: n.y }));
        // If d3-force's own timer were still scheduled, advancing past
        // several animation-frame intervals would keep perturbing alpha
        // and node positions.
        vi.advanceTimersByTime(2000);
        expect(nodes.map((n) => ({ x: n.x, y: n.y }))).toEqual(settled);
      } finally {
        sim.stop();
      }
    } finally {
      vi.useRealTimers();
    }
  });
});

const VAULT_A = "vault-a";
const VAULT_B = "vault-b";

const VAULT_GRAPHS: VaultGraph[] = [
  {
    vault_id: VAULT_A,
    vault_name: "Alpha Vault",
    nodes: [
      {
        vault_id: VAULT_A,
        slug: "a1",
        title: "A1",
        primary_tag: null,
        backlink_count: 0,
      },
      {
        vault_id: VAULT_A,
        slug: "a2",
        title: "A2",
        primary_tag: null,
        backlink_count: 2,
      },
    ],
    edges: [{ vault_id: VAULT_A, source_slug: "a1", target_slug: "a2" }],
  },
  {
    vault_id: VAULT_B,
    vault_name: "Beta Vault",
    nodes: [
      {
        vault_id: VAULT_B,
        slug: "b1",
        title: "B1",
        primary_tag: null,
        backlink_count: 0,
      },
    ],
    edges: [],
  },
];

describe("computeIslandCenters", () => {
  it("returns an empty array for zero islands", () => {
    expect(computeIslandCenters([])).toEqual([]);
  });

  it("centers a single island on the origin", () => {
    expect(computeIslandCenters([10])).toEqual([{ cx: 0, cy: 0 }]);
  });

  it("packs three islands on a grid in the given order, never sorted by count", () => {
    const centers = computeIslandCenters([1, 100, 1]);
    expect(centers).toHaveLength(3);
    // 2x2 grid (ceil(sqrt(3)) = 2 cols): index 0 and 1 share the top row,
    // index 2 starts the next row — regardless of the middle island's size.
    expect(centers[0].cy).toBe(centers[1].cy);
    expect(centers[0].cx).toBeLessThan(centers[1].cx);
    expect(centers[2].cy).toBeGreaterThan(centers[0].cy);
  });

  it("is deterministic for the same input", () => {
    expect(computeIslandCenters([3, 5, 2])).toEqual(
      computeIslandCenters([3, 5, 2]),
    );
  });

  it("never overlaps two islands, however lopsided the Vault sizes", () => {
    // The failing shape from #118's review: one large Vault beside several
    // tiny ones overlapped its neighbours by up to 120px under one uniform
    // spacing, so both captions and both enclosures became unreadable.
    const counts = [2, 3, 2, 0, 2, 49];
    const centers = computeIslandCenters(counts);
    for (let i = 0; i < counts.length; i++) {
      for (let j = i + 1; j < counts.length; j++) {
        const distance = Math.hypot(
          centers[i].cx - centers[j].cx,
          centers[i].cy - centers[j].cy,
        );
        const needed =
          islandRadiusEstimate(counts[i]) + islandRadiusEstimate(counts[j]);
        expect(distance).toBeGreaterThanOrEqual(needed);
      }
    }
  });

  it("sizes each column to its own widest island, not to the largest anywhere", () => {
    // 2x2: column 0 holds both 1-node Vaults, column 1 holds the 400-node one.
    // The all-small column must stay narrow instead of inheriting its
    // neighbour's width, which one uniform spacing gave it.
    const centers = computeIslandCenters([1, 400, 1, 1]);
    const totalWidth = 2 * (centers[1].cx - centers[0].cx);
    const smallColumnWidth = 2 * (centers[0].cx + totalWidth / 2);
    const largeColumnWidth = totalWidth - smallColumnWidth;
    expect(smallColumnWidth).toBeLessThan(largeColumnWidth);
  });

  it("grows spacing with the largest island's node count", () => {
    const tight = computeIslandCenters([1, 1]);
    const spread = computeIslandCenters([1, 400]);
    const tightGap = Math.abs(tight[1].cx - tight[0].cx);
    const spreadGap = Math.abs(spread[1].cx - spread[0].cx);
    expect(spreadGap).toBeGreaterThan(tightGap);
  });
});

describe("buildIslandGraphs", () => {
  it("builds one island per Vault, in the given order, with its own node count", () => {
    const { islands } = buildIslandGraphs(VAULT_GRAPHS, {
      random: sequenceRandom([0.5]),
    });
    expect(islands.map((i) => i.vaultId)).toEqual([VAULT_A, VAULT_B]);
    expect(islands[0].vaultName).toBe("Alpha Vault");
    expect(islands[0].nodeCount).toBe(2);
    expect(islands[1].nodeCount).toBe(1);
    // Packed on a grid, not stacked on the same spot.
    expect(
      islands[1].cx !== islands[0].cx || islands[1].cy !== islands[0].cy,
    ).toBe(true);
  });

  it("centers each island's nodes on its own grid center, not the shared origin", () => {
    const { islands } = buildIslandGraphs(VAULT_GRAPHS, {
      random: sequenceRandom([0.5]), // (0.5-0.5)*spread = 0 scatter offset
    });
    for (const island of islands) {
      for (const node of island.nodes) {
        expect(node.x).toBeCloseTo(island.cx);
        expect(node.y).toBeCloseTo(island.cy);
        expect(node.islandCx).toBe(island.cx);
        expect(node.islandCy).toBe(island.cy);
      }
    }
  });

  it("flattens every island's nodes and links", () => {
    const { nodes, links } = buildIslandGraphs(VAULT_GRAPHS, {
      random: sequenceRandom([0.5]),
    });
    expect(nodes).toHaveLength(3);
    expect(links).toHaveLength(1);
    expect(links[0].source.slug).toBe("a1");
    expect(links[0].target.slug).toBe("a2");
  });
});

describe("createIslandSimulation", () => {
  it("registers link, charge, x, y, and collide forces — no shared center force", () => {
    const { nodes, links } = buildIslandGraphs(VAULT_GRAPHS, {
      random: sequenceRandom([0.5]),
    });
    const sim = createIslandSimulation(nodes, links);
    try {
      expect(sim.force("link")).toBeTruthy();
      expect(sim.force("charge")).toBeTruthy();
      expect(sim.force("x")).toBeTruthy();
      expect(sim.force("y")).toBeTruthy();
      expect(sim.force("collide")).toBeTruthy();
      expect(sim.force("center")).toBeUndefined();
      expect(sim.nodes()).toHaveLength(3);
    } finally {
      sim.stop();
    }
  });
});

describe("hitTest", () => {
  const identity = { x: 0, y: 0, k: 1 };
  const nodes: SimNode[] = [
    {
      vault_id: VAULT_ID,
      slug: "a",
      title: "Alpha",
      primary_tag: null,
      backlink_count: 0,
      x: 0,
      y: 0,
    },
    {
      vault_id: VAULT_ID,
      slug: "b",
      title: "Bravo",
      primary_tag: null,
      backlink_count: 0,
      x: 100,
      y: 0,
    },
  ];

  it("returns the node under the point (identity transform)", () => {
    expect(hitTest(nodes, identity, 0, 0)?.slug).toBe("a");
    expect(hitTest(nodes, identity, 100, 0)?.slug).toBe("b");
  });

  it("returns null when the point is outside every node radius", () => {
    expect(hitTest(nodes, identity, 50, 50)).toBeNull();
  });

  it("maps canvas coordinates through the transform before testing", () => {
    // pan +200 in x, zoom 2×: node "a" at world (0,0) sits at canvas (200,0).
    const transform = { x: 200, y: 0, k: 2 };
    expect(hitTest(nodes, transform, 200, 0)?.slug).toBe("a");
    expect(hitTest(nodes, transform, 0, 0)).toBeNull();
  });

  it("prefers the closest node when radii overlap", () => {
    const near: SimNode[] = [
      { ...nodes[0], slug: "far", x: 6, y: 0, backlink_count: 40 },
      { ...nodes[0], slug: "near", x: 1, y: 0, backlink_count: 40 },
    ];
    expect(hitTest(near, identity, 0, 0)?.slug).toBe("near");
  });
});

describe("hitTest — screen-space reach (#337)", () => {
  const leaf: SimNode = {
    vault_id: VAULT_ID,
    slug: "leaf",
    title: "Leaf",
    primary_tag: null,
    backlink_count: 0,
    x: 0,
    y: 0,
  };

  it("measures its slack in screen pixels, so it does not shrink when zoomed out", () => {
    // At k = 0.2 a leaf is drawn at the 2px floor; 2px of slack reaches 4px.
    const zoomedOut = { x: 0, y: 0, k: 0.2 };
    expect(hitTest([leaf], zoomedOut, 3.5, 0)).toBe(leaf);
    expect(hitTest([leaf], zoomedOut, 5, 0)).toBeNull();
  });

  it("gives a touch at least a 24px target around a leaf note, at any zoom", () => {
    for (const k of [0.2, 0.9, 2]) {
      const transform = { x: 0, y: 0, k };
      expect(hitTest([leaf], transform, 12, 0, TOUCH_HIT_TARGET)).toBe(leaf);
      expect(hitTest([leaf], transform, 0, -12, TOUCH_HIT_TARGET)).toBe(leaf);
    }
    // The mouse keeps the tight reach.
    expect(hitTest([leaf], { x: 0, y: 0, k: 0.9 }, 12, 0)).toBeNull();
  });

  it("still picks the nearer of two nodes inside a touch target", () => {
    const other = { ...leaf, slug: "other", x: 20 };
    const transform = { x: 0, y: 0, k: 1 };
    expect(hitTest([leaf, other], transform, 12, 0, TOUCH_HIT_TARGET)).toBe(
      other,
    );
    expect(hitTest([leaf, other], transform, 8, 0, TOUCH_HIT_TARGET)).toBe(
      leaf,
    );
  });
});

describe("normalizeWheelDelta (#337)", () => {
  const stepFor = (deltaY: number, deltaMode: number) =>
    Math.pow(0.999, normalizeWheelDelta(deltaY, deltaMode, 800));

  it("passes pixel deltas through", () => {
    expect(normalizeWheelDelta(-4, 0, 800)).toBe(-4);
    expect(normalizeWheelDelta(120, 0, 800)).toBe(120);
  });

  it("zooms a Firefox line-mode notch within a small factor of a Chromium pixel notch", () => {
    // Chromium: ~100-120px per notch. Firefox: 3 lines per notch.
    const chrome = Math.log(stepFor(120, 0));
    const firefox = Math.log(stepFor(3, 1));
    expect(firefox / chrome).toBeGreaterThan(0.5);
    expect(firefox / chrome).toBeLessThan(2);
  });

  it("scales page-mode deltas by the page height, clamped to one bounded step", () => {
    expect(normalizeWheelDelta(1, 2, 300)).toBe(300);
    expect(normalizeWheelDelta(1, 2, 2000)).toBe(600);
    expect(normalizeWheelDelta(-1, 2, 2000)).toBe(-600);
  });
});

describe("viewport culling (#337)", () => {
  // Panned so world (0,0) sits at canvas (100,50), zoomed 2x, on a 400x300
  // canvas: world x runs -50..150, world y -25..125.
  const view = worldViewport({ x: 100, y: 50, k: 2 }, 400, 300);

  it("maps the canvas back to a world rectangle", () => {
    expect(view).toEqual({ minX: -50, minY: -25, maxX: 150, maxY: 125 });
    expect(worldViewport({ x: 100, y: 50, k: 2 }, 400, 300, 20).minX).toBe(-60);
  });

  it("keeps a circle that overlaps the edge and drops one wholly outside", () => {
    expect(circleInView(view, 0, 0, 1)).toBe(true);
    expect(circleInView(view, -55, 0, 6)).toBe(true);
    expect(circleInView(view, -55, 0, 4)).toBe(false);
    expect(circleInView(view, 0, 200, 10)).toBe(false);
  });

  it("keeps an edge that crosses the view even with both ends outside it", () => {
    expect(segmentInView(view, -500, 0, 500, 0)).toBe(true);
    expect(segmentInView(view, -500, 0, -100, 0)).toBe(false);
    expect(segmentInView(view, 0, 200, 50, 300)).toBe(false);
  });
});

describe("layoutLabels (#337)", () => {
  function gridNodes(count: number): SimNode[] {
    return Array.from({ length: count }, (_, i) => ({
      vault_id: VAULT_ID,
      slug: `n${i}`,
      title: `Note number ${i}`,
      primary_tag: null,
      backlink_count: i % 7,
      x: (i % 40) * 12,
      y: Math.floor(i / 40) * 12,
    }));
  }

  it("considers at most a fixed budget of labels however many nodes are eligible", () => {
    // At k = 2 every node clears the size threshold: the old pass took all
    // 800 as candidates and measured every one of them, every frame.
    const nodes = gridNodes(800);
    const measure = vi.fn((text: string) => text.length * 6);
    const placed = layoutLabels({
      nodes,
      forced: [],
      transform: { x: 0, y: 0, k: 2 },
      hubMinBacklinks: 99,
      measure,
    });
    expect(measure.mock.calls.length).toBeLessThanOrEqual(LABEL_BUDGET);
    expect(placed.length).toBeLessThanOrEqual(LABEL_BUDGET);
  });

  it("never lets two placed labels overlap", () => {
    const placed = layoutLabels({
      nodes: gridNodes(400),
      forced: [],
      transform: { x: 0, y: 0, k: 3 },
      hubMinBacklinks: 99,
      measure: (text) => text.length * 6,
    });
    expect(placed.length).toBeGreaterThan(0);
    for (let i = 0; i < placed.length; i++) {
      for (let j = i + 1; j < placed.length; j++) {
        const a = placed[i];
        const b = placed[j];
        const overlap =
          a.x < b.x + b.width &&
          a.x + a.width > b.x &&
          a.y < b.y + b.height &&
          a.y + a.height > b.y;
        expect(overlap).toBe(false);
      }
    }
  });

  it("always labels the hovered and selected nodes first, even crowded out", () => {
    const nodes = gridNodes(400);
    const hovered = nodes[123];
    const selected = nodes[124];
    const placed = layoutLabels({
      nodes,
      forced: [hovered, selected],
      transform: { x: 0, y: 0, k: 1 },
      hubMinBacklinks: 0,
      measure: (text) => text.length * 6,
      budget: 0,
    });
    expect(placed.map((p) => p.node)).toEqual([hovered, selected]);
  });

  it("places labels in screen space, below the node when there is room", () => {
    const [node] = gridNodes(1);
    const [label] = layoutLabels({
      nodes: [node],
      forced: [node],
      transform: { x: 100, y: 50, k: 2 },
      hubMinBacklinks: 0,
      measure: () => 40,
    });
    expect(label.width).toBe(50);
    expect(label.x).toBe(100 - 25);
    // Below the node's drawn edge (radius 4 * 2 = 8px) plus a 4px gap.
    expect(label.y).toBe(50 + 8 + 4);
  });

  it("cuts long titles", () => {
    expect(labelText("short")).toBe("short");
    expect(labelText("x".repeat(40))).toBe("x".repeat(26) + "…");
  });
});

describe("island captions (#337)", () => {
  it("keep their design size at 1x and above", () => {
    for (const k of [1, 4]) {
      const m = islandCaptionMetrics(k);
      expect(m.nameSize).toBe(20);
      expect(m.countSize).toBe(13);
      expect(m.gap + m.lineHeight).toBeCloseTo(16 + 24);
    }
  });

  it("stay legible at the scale a phone frames three Vaults at", () => {
    // The world-space caption was 20 * 0.221 = 4.4px here.
    const m = islandCaptionMetrics(0.221);
    expect(m.nameSize).toBeGreaterThanOrEqual(13);
    expect(m.countSize).toBeGreaterThanOrEqual(11);
  });

  it("say indexing, in warn ink, for a Vault whose index is building", () => {
    expect(islandCountLine({ kind: "indexing" }, 0)).toEqual({
      text: "indexing",
      tone: "warn",
    });
  });

  it("carry the condition word and tier, or the note count", () => {
    expect(
      islandCountLine(
        { kind: "condition", word: "unavailable", tier: "error", sentence: "" },
        3,
      ),
    ).toEqual({ text: "unavailable", tone: "error" });
    expect(islandCountLine({ kind: "count", count: 1 }, 1).text).toBe("1 note");
    expect(islandCountLine({ kind: "count", count: 49 }, 49).text).toBe(
      "49 notes",
    );
  });
});

describe("fitIslandField (#337)", () => {
  // The audit's three small Vaults: a grid about 1474 x 1354 world units.
  const bounds = { minX: -737, minY: -677, maxX: 737, maxY: 677 };

  it("never frames the field below the minimum scale on a phone", () => {
    const t = fitIslandField(bounds, 390, 640);
    expect(t.k).toBeGreaterThanOrEqual(ISLAND_FIT_MIN_SCALE);
  });

  it("frames the whole field, caption stack included, when it fits", () => {
    const width = 1200;
    const height = 900;
    const t = fitIslandField(bounds, width, height);
    const caption = islandCaptionMetrics(t.k).height;
    const top = bounds.minY * t.k + t.y - caption;
    const bottom = bounds.maxY * t.k + t.y;
    expect(top).toBeGreaterThanOrEqual(0);
    expect(bottom).toBeLessThanOrEqual(height);
    expect(bounds.minX * t.k + t.x).toBeGreaterThanOrEqual(0);
    expect(bounds.maxX * t.k + t.x).toBeLessThanOrEqual(width);
  });

  it("never zooms a small field in past the landing scale", () => {
    const t = fitIslandField(
      { minX: -50, minY: -50, maxX: 50, maxY: 50 },
      1200,
      900,
    );
    expect(t.k).toBe(0.9);
  });
});
