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
  forceX,
  forceY,
  type ForceLink,
  type Simulation,
  type SimulationLinkDatum,
  type SimulationNodeDatum,
} from "d3-force";

import type { GraphData, VaultGraph, VaultId } from "../../types";

export interface SimNode extends SimulationNodeDatum {
  vault_id: string;
  slug: string;
  title: string;
  primary_tag: string | null;
  backlink_count: number;
  x: number;
  y: number;
  /** Target island center in world space (#143). Undefined on the
   * single-component path, where `createGraphSimulation`'s global
   * `forceCenter` applies instead. */
  islandCx?: number;
  islandCy?: number;
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

/** Scatter half-extent (world units) for a note that joins a live layout next
 * to a neighbour already on screen: close enough to read as "arrived beside
 * this", far enough that collision does not fling the pair apart. */
const NEIGHBOUR_JITTER = 30;

export interface BuildGraphOptions {
  spread?: number;
  random?: () => number;
  /** World point a fresh scatter is centred on. Defaults to the origin. */
  origin?: { x: number; y: number };
  /** Nodes already in the live simulation, by `nodeKey` (#336). A datum whose
   * key is here reuses that very object — position, velocity and any drag pin
   * intact — with its display fields refreshed, so a data refresh updates the
   * picture instead of re-scattering it. A new datum linked to a reused node
   * is placed beside it rather than at a random point in the field. */
  previous?: ReadonlyMap<string, SimNode>;
}

/**
 * Build simulation nodes and links from API graph data. Fresh nodes are
 * scattered around `origin` (default (0,0)) — never in canvas-pixel space —
 * using `random` so tests can inject a deterministic sequence. Links whose
 * endpoints are missing (danglers) are dropped, mirroring the resolved-only
 * edges the API returns.
 */
export function buildSimulationGraph(
  data: GraphData,
  {
    spread = DEFAULT_SPREAD,
    random = Math.random,
    origin = { x: 0, y: 0 },
    previous,
  }: BuildGraphOptions = {},
): { nodes: SimNode[]; links: SimLink[] } {
  const fresh = new Set<SimNode>();
  const nodes: SimNode[] = data.nodes.map((n) => {
    const kept = previous?.get(nodeKey(n));
    if (kept) {
      kept.title = n.title;
      kept.primary_tag = n.primary_tag;
      kept.backlink_count = n.backlink_count;
      return kept;
    }
    const node = {
      ...n,
      x: origin.x + (random() - 0.5) * spread,
      y: origin.y + (random() - 0.5) * spread,
    } as SimNode;
    fresh.add(node);
    return node;
  });

  const nodeByKey = new Map<string, SimNode>(nodes.map((n) => [nodeKey(n), n]));

  const links: SimLink[] = data.edges
    .map((e) => {
      const source = nodeByKey.get(`${e.vault_id}:${e.source_slug}`);
      const target = nodeByKey.get(`${e.vault_id}:${e.target_slug}`);
      if (!source || !target) return null;
      return { source, target } as SimLink;
    })
    .filter((l): l is SimLink => l !== null);

  // Only a refresh has anything to sit beside; a first build is untouched.
  if (previous && previous.size > 0 && fresh.size > 0) {
    for (const link of links) {
      const [node, anchor] = fresh.has(link.source)
        ? [link.source, link.target]
        : [link.target, link.source];
      if (!fresh.has(node) || fresh.has(anchor)) continue;
      node.x = anchor.x + (random() - 0.5) * NEIGHBOUR_JITTER;
      node.y = anchor.y + (random() - 0.5) * NEIGHBOUR_JITTER;
      fresh.delete(node);
    }
  }

  return { nodes, links };
}

/** How warm a refreshed layout is made (#336): enough for new or removed notes
 * to find their place, far below a fresh build's alpha of 1, so the notes
 * already on screen shift a little rather than re-settling from scratch. */
export const REFRESH_ALPHA = 0.3;

/**
 * Swap a live simulation's nodes and links in place (#336) rather than
 * building a new one, so every node object the reader is looking at, dragging
 * or has selected stays the same object. Forces are re-initialised against
 * the new node list, and the layout is re-warmed to `REFRESH_ALPHA` (never
 * cooled if it is already warmer). The caller restarts or settles it.
 */
export function replaceSimulationGraph(
  sim: Simulation<SimNode, SimLink>,
  nodes: SimNode[],
  links: SimLink[],
): void {
  const linkForce = sim.force("link") as
    ForceLink<SimNode, SimLink> | undefined;
  // Empty the link force first: `nodes()` re-initialises every force, and the
  // link force would otherwise index the old links against the new node list.
  linkForce?.links([]);
  sim.nodes(nodes);
  linkForce?.links(links);
  sim.alpha(Math.max(sim.alpha(), REFRESH_ALPHA));
}

/** Bound on synchronous ticks `settleSimulationSync` will spend advancing a
 * simulation, so a pathologically large graph can't hang the main thread —
 * comfortably past the ~340 ticks `alphaDecay(0.02)` takes to cross from
 * alpha 1 to the default `alphaMin` (1e-3). */
const MAX_SYNC_SETTLE_TICKS = 500;

/**
 * Run a freshly created simulation to its resting alpha synchronously, with
 * its own timer stopped throughout, so no intermediate frame is ever
 * painted — the `prefers-reduced-motion: reduce` path (#147): the graph
 * "computes silently and paints already settled" instead of visibly
 * animating into place.
 */
export function settleSimulationSync<
  N extends SimulationNodeDatum,
  L extends SimulationLinkDatum<N>,
>(sim: Simulation<N, L>): Simulation<N, L> {
  sim.stop();
  for (
    let i = 0;
    i < MAX_SYNC_SETTLE_TICKS && sim.alpha() > sim.alphaMin();
    i++
  ) {
    sim.tick();
  }
  return sim;
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

// ── all-Vault islands (#143) ────────────────────────────────────────────────

/** Matches `ISLAND_ENCLOSURE_MARGIN` in GraphPage, plus slack for the outermost
 * node's own drawn radius, which the enclosure is measured past. */
const ISLAND_ENCLOSURE_ALLOWANCE = 56;
/** Floor for a tiny or empty Vault: two nodes still push apart under charge. */
const ISLAND_MIN_RADIUS = 120;
/** Calibrated against settled radii, then rounded up — a 49-node island
 * measured 356px, which this estimates at 362px. */
const ISLAND_RADIUS_PER_NODE = 46;
/** Clear air between neighbouring enclosures. */
const ISLAND_GUTTER = 48;
/** Caption gap plus its two lines, mirrored from GraphPage's own constants. */
const ISLAND_CAPTION_HEADROOM = 46;

/**
 * Deterministic grid centers for N islands, in the given order (Vault-
 * management order — #118's resolution: never sorted by size, note count, or
 * condition). Each column takes the width its own widest island needs and each
 * row the height its own tallest needs, so a big component never spills into
 * its neighbours and small ones are not padded out to match it; the grid
 * itself never reorders. The whole grid is centred on the origin, and
 * `GraphPage` fits the view to it once the layout settles.
 */
export function computeIslandCenters(
  nodeCounts: number[],
): { cx: number; cy: number }[] {
  const n = nodeCounts.length;
  if (n === 0) return [];
  const cols = Math.ceil(Math.sqrt(n));
  const rows = Math.ceil(n / cols);

  // Each column is as wide as its widest island and each row as tall as its
  // tallest, rather than every cell taking the largest island's footprint.
  // One uniform spacing keyed to `maxCount` did both things wrong at once: a
  // 49-note Vault beside 2-note ones overlapped its neighbours by up to 120px
  // (its own radius outgrew the shared cell), while those neighbours sat in
  // cells a third wider than they needed. Sizing per column and row keeps the
  // grid order intact and removes both faults.
  const radii = nodeCounts.map(islandRadiusEstimate);
  const colWidth = new Array<number>(cols).fill(0);
  const rowHeight = new Array<number>(rows).fill(0);
  for (let i = 0; i < n; i++) {
    const col = i % cols;
    const row = Math.floor(i / cols);
    colWidth[col] = Math.max(colWidth[col], radii[i] * 2 + ISLAND_GUTTER);
    // Captions stack above the enclosure, so a row must clear them too.
    rowHeight[row] = Math.max(
      rowHeight[row],
      radii[i] * 2 + ISLAND_CAPTION_HEADROOM + ISLAND_GUTTER,
    );
  }

  const colCenters = runningCenters(colWidth);
  const rowCenters = runningCenters(rowHeight);
  const centers: { cx: number; cy: number }[] = [];
  for (let i = 0; i < n; i++) {
    centers.push({
      cx: colCenters[i % cols],
      cy: rowCenters[Math.floor(i / cols)],
    });
  }
  return centers;
}

/**
 * A generous upper bound on an island's settled enclosure radius, from node
 * count alone — the true radius is only known once the layout settles, long
 * after centers must be fixed. Deliberately over- rather than under-estimates:
 * spare space inside a cell costs only zoom, since the view fits the whole
 * field, whereas an underestimate overlaps two Vaults and makes both illegible.
 */
export function islandRadiusEstimate(nodeCount: number): number {
  return (
    ISLAND_ENCLOSURE_ALLOWANCE +
    Math.max(ISLAND_MIN_RADIUS, ISLAND_RADIUS_PER_NODE * Math.sqrt(nodeCount))
  );
}

/** Track centers for a run of sizes, with the whole run centered on zero. */
function runningCenters(sizes: number[]): number[] {
  const total = sizes.reduce((sum, size) => sum + size, 0);
  const centers: number[] = [];
  let edge = -total / 2;
  for (const size of sizes) {
    centers.push(edge + size / 2);
    edge += size;
  }
  return centers;
}

export interface GraphIsland {
  vaultId: VaultId;
  vaultName: string;
  /** Same count the API returns nodes for — every note in the Vault under
   * the active layer selection, not just linked ones (#143's caption count
   * line). */
  nodeCount: number;
  cx: number;
  cy: number;
  nodes: SimNode[];
  links: SimLink[];
}

/**
 * Lay out every participating Vault's component on its own (via
 * `buildSimulationGraph`, unchanged) and place it at a grid-packed island
 * center. Nodes carry their island's center as `islandCx`/`islandCy` so one
 * shared simulation (`createIslandSimulation`) can pull each cluster toward
 * its own spot instead of the single shared origin the byte-identical
 * single-component path still uses.
 */
export function buildIslandGraphs(
  vaultGraphs: VaultGraph[],
  {
    random = Math.random,
    previous,
  }: {
    random?: () => number;
    /** As `buildSimulationGraph`'s: live nodes to reuse on a refresh (#336). */
    previous?: ReadonlyMap<string, SimNode>;
  } = {},
): { islands: GraphIsland[]; nodes: SimNode[]; links: SimLink[] } {
  const centers = computeIslandCenters(
    vaultGraphs.map((vaultGraph) => vaultGraph.nodes.length),
  );
  const islands: GraphIsland[] = vaultGraphs.map((vaultGraph, i) => {
    const spread = Math.max(120, 40 * Math.sqrt(vaultGraph.nodes.length || 1));
    const { cx, cy } = centers[i];
    const { nodes, links } = buildSimulationGraph(
      { nodes: vaultGraph.nodes, edges: vaultGraph.edges },
      { spread, random, origin: { x: cx, y: cy }, previous },
    );
    for (const node of nodes) {
      node.islandCx = cx;
      node.islandCy = cy;
    }
    return {
      vaultId: vaultGraph.vault_id,
      vaultName: vaultGraph.vault_name,
      nodeCount: vaultGraph.nodes.length,
      cx,
      cy,
      nodes,
      links,
    };
  });
  return {
    islands,
    nodes: islands.flatMap((island) => island.nodes),
    links: islands.flatMap((island) => island.links),
  };
}

/** Same forces as `createGraphSimulation`, except each node is pulled toward
 * its own island center (`islandCx`/`islandCy`) via `forceX`/`forceY`
 * instead of every node sharing one `forceCenter`. Edges never cross Vaults
 * (the API guarantees this), so `forceLink` never pulls two islands
 * together. */
export function createIslandSimulation(
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
    .force("x", forceX<SimNode>((d) => d.islandCx ?? 0).strength(0.08))
    .force("y", forceY<SimNode>((d) => d.islandCy ?? 0).strength(0.08))
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
