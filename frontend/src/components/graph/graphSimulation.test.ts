import { describe, expect, it } from "vitest";

import type { GraphData } from "../../types";
import {
  buildSimulationGraph,
  createGraphSimulation,
  hitTest,
  nodeRadius,
  type SimNode,
} from "./graphSimulation";

function sequenceRandom(values: number[]): () => number {
  let i = 0;
  return () => values[i++ % values.length];
}

const DATA: GraphData = {
  nodes: [
    { slug: "a", title: "Alpha", primary_tag: "topic/x", backlink_count: 3 },
    { slug: "b", title: "Bravo", primary_tag: null, backlink_count: 0 },
    { slug: "c", title: "Charlie", primary_tag: "topic/y", backlink_count: 1 },
  ],
  edges: [
    { source: "a", target: "b" },
    { source: "b", target: "c" },
    { source: "a", target: "ghost" }, // dangler — target missing
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
    expect(
      links.map((l) => `${l.source.slug}->${l.target.slug}`),
    ).toEqual(["a->b", "b->c"]);
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

describe("hitTest", () => {
  const identity = { x: 0, y: 0, k: 1 };
  const nodes: SimNode[] = [
    {
      slug: "a",
      title: "Alpha",
      primary_tag: null,
      backlink_count: 0,
      x: 0,
      y: 0,
    },
    {
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
