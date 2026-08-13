//! Pure d3-force graph model for the knowledge-graph view: node/link data
//! types, layout geometry, graph construction from API data, force-simulation
//! configuration, and hit-testing. Kept free of React and canvas so it can be
//! unit-tested; `GraphPage` owns rendering, interaction, and lifecycle.

import {
  forceCenter,
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  type Simulation,
  type SimulationLinkDatum,
  type SimulationNodeDatum,
} from "d3-force";

import type { GraphData } from "../../types";

export interface SimNode extends SimulationNodeDatum {
  vault_id: string;
  slug: string;
  title: string;
  primary_tag: string | null;
  backlink_count: number;
  x: number;
  y: number;
}

export interface SimLink extends SimulationLinkDatum<SimNode> {
  source: SimNode;
  target: SimNode;
}

/** A slug is only unique within its own Vault (#137), and graph edges never
 * cross Vaults, so nodes are identified and linked by `(vault_id, slug)`. */
export function nodeKey(node: Pick<SimNode, "vault_id" | "slug">): string {
  return `${node.vault_id}:${node.slug}`;
}

export interface Transform {
  x: number;
  y: number;
  k: number;
}

const BASE_RADIUS = 4;
const SCALE_FACTOR = 2.8;

/** Screen radius of a node, growing logarithmically with backlink count. */
export function nodeRadius(backlinks: number): number {
  return BASE_RADIUS + Math.log(backlinks + 1) * SCALE_FACTOR;
}

/** Initial random scatter half-extent for freshly placed nodes (world units). */
export const DEFAULT_SPREAD = 500;

/**
 * Build simulation nodes and links from API graph data. Nodes are scattered
 * around the origin (0,0) — never in canvas-pixel space — using `random` so
 * tests can inject a deterministic sequence. Links whose endpoints are missing
 * (danglers) are dropped, mirroring the resolved-only edges the API returns.
 */
export function buildSimulationGraph(
  data: GraphData,
  {
    spread = DEFAULT_SPREAD,
    random = Math.random,
  }: { spread?: number; random?: () => number } = {},
): { nodes: SimNode[]; links: SimLink[] } {
  const nodes: SimNode[] = data.nodes.map((n) => ({
    ...n,
    x: (random() - 0.5) * spread,
    y: (random() - 0.5) * spread,
  })) as SimNode[];

  const nodeByKey = new Map<string, SimNode>(nodes.map((n) => [nodeKey(n), n]));

  const links: SimLink[] = data.edges
    .map((e) => {
      const source = nodeByKey.get(`${e.vault_id}:${e.source_slug}`);
      const target = nodeByKey.get(`${e.vault_id}:${e.target_slug}`);
      if (!source || !target) return null;
      return { source, target } as SimLink;
    })
    .filter((l): l is SimLink => l !== null);

  return { nodes, links };
}

/** Configure the force simulation (link/charge/center/collide) used by the graph. */
export function createGraphSimulation(
  nodes: SimNode[],
  links: SimLink[],
): Simulation<SimNode, SimLink> {
  return forceSimulation<SimNode>(nodes)
    .force(
      "link",
      forceLink<SimNode, SimLink>(links)
        .id((d) => nodeKey(d))
        .distance(60)
        .strength(0.4),
    )
    .force("charge", forceManyBody<SimNode>().strength(-180).distanceMax(400))
    .force("center", forceCenter<SimNode>(0, 0))
    .force(
      "collide",
      forceCollide<SimNode>().radius((d) => nodeRadius(d.backlink_count) + 4),
    )
    .alphaDecay(0.02);
}

/**
 * Return the closest node under a canvas point, or null. `cx`/`cy` are in
 * canvas pixels; they are mapped back into world space via `transform` before
 * the radius test (with a 2px slack to make small nodes easier to grab).
 */
export function hitTest(
  nodes: SimNode[],
  transform: Transform,
  cx: number,
  cy: number,
): SimNode | null {
  const { x, y, k } = transform;
  const wx = (cx - x) / k;
  const wy = (cy - y) / k;
  let best: SimNode | null = null;
  let bestDist = Infinity;
  for (const node of nodes) {
    const r = nodeRadius(node.backlink_count);
    const d = Math.hypot(node.x - wx, node.y - wy);
    if (d <= r + 2 && d < bestDist) {
      best = node;
      bestDist = d;
    }
  }
  return best;
}
