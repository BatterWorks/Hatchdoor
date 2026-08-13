import { describe, expect, it } from "vitest";

import type { GraphData, VaultGraph } from "../../types";
import {
  buildIslandGraphs,
  buildSimulationGraph,
  computeIslandCenters,
  createGraphSimulation,
  createIslandSimulation,
  hitTest,
  nodeRadius,
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
