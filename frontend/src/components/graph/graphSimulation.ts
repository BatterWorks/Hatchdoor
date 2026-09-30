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

import type { VaultSlotState } from "../../app/vaultSlotLogic";
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
/** Row space reserved above each enclosure for its caption. Captions are sized
 * in screen pixels (`islandCaptionMetrics`, #337), so this is the full-size
 * stack at 1x; zoomed well out, a caption may reach into the gutter above. */
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

/** Hit-target sizing for `hitTest`, in screen (CSS) pixels. */
export interface HitTarget {
  /** Extra reach past a node's drawn edge. */
  slackPx?: number;
  /** Floor on the hit radius however small the node is drawn, so a leaf
   * note stays grabbable when the view is zoomed out (#337). */
  minRadiusPx?: number;
}

/** A finger covers far more than a cursor tip: at least a 28px target
 * (radius 14), over the ~24 CSS px floor #337 asks for, and wider still around
 * a node already drawn larger than that. */
export const TOUCH_HIT_TARGET: HitTarget = { slackPx: 6, minRadiusPx: 14 };

/** A node is never drawn smaller than this on screen (#337): the all-Vault fit
 * can frame the field well below 1x, and a leaf note's world radius of 4 there
 * became a sub-pixel dot. */
export const NODE_MIN_SCREEN_RADIUS = 2;

/** A node's drawn radius in screen pixels at zoom `k`. */
export function nodeScreenRadius(backlinks: number, k: number): number {
  return Math.max(nodeRadius(backlinks) * k, NODE_MIN_SCREEN_RADIUS);
}

/**
 * Return the closest node under a canvas point, or null. `cx`/`cy` are in
 * canvas pixels. The test runs in screen space (#337): a node is hit within its
 * drawn radius plus `slackPx`, or within `minRadiusPx` if that is larger, so
 * the reach is the same number of CSS pixels at every zoom instead of
 * shrinking with it. The mouse default is the drawn edge plus 2px.
 */
export function hitTest(
  nodes: SimNode[],
  transform: Transform,
  cx: number,
  cy: number,
  { slackPx = 2, minRadiusPx = 0 }: HitTarget = {},
): SimNode | null {
  const { x, y, k } = transform;
  let best: SimNode | null = null;
  let bestDist = Infinity;
  for (const node of nodes) {
    const reach = Math.max(
      nodeScreenRadius(node.backlink_count, k) + slackPx,
      minRadiusPx,
    );
    const d = Math.hypot(node.x * k + x - cx, node.y * k + y - cy);
    if (d <= reach && d < bestDist) {
      best = node;
      bestDist = d;
    }
  }
  return best;
}

// ── wheel zoom (#337) ───────────────────────────────────────────────────────

/** Pixels one wheel "line" stands for. Firefox reports a mouse-wheel notch as
 * three lines (`deltaMode` 1) where Chromium reports ~100-120 pixels, so 40
 * puts a Firefox notch on a par with a Chromium one. */
export const WHEEL_LINE_PX = 40;
/** Cap on one wheel event's zoom, in pixel-equivalents: a page-mode delta must
 * not cross the whole zoom range in one event. */
export const WHEEL_MAX_DELTA_PX = 600;

/**
 * A wheel event's vertical delta in pixel-equivalents, whatever unit the
 * browser reported it in (`deltaMode` 0 pixels, 1 lines, 2 pages), clamped
 * to `WHEEL_MAX_DELTA_PX` either way.
 */
export function normalizeWheelDelta(
  deltaY: number,
  deltaMode: number,
  pageHeightPx: number,
): number {
  const px =
    deltaMode === 1
      ? deltaY * WHEEL_LINE_PX
      : deltaMode === 2
        ? deltaY * (pageHeightPx > 0 ? pageHeightPx : 800)
        : deltaY;
  return Math.max(-WHEEL_MAX_DELTA_PX, Math.min(WHEEL_MAX_DELTA_PX, px));
}

// ── viewport culling and label placement (#337) ─────────────────────────────

export interface WorldRect {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

/** The world-space rectangle a `width` x `height` canvas shows under
 * `transform`, grown by `marginPx` screen pixels on every side. */
export function worldViewport(
  transform: Transform,
  width: number,
  height: number,
  marginPx = 0,
): WorldRect {
  const { x, y, k } = transform;
  return {
    minX: (-marginPx - x) / k,
    minY: (-marginPx - y) / k,
    maxX: (width + marginPx - x) / k,
    maxY: (height + marginPx - y) / k,
  };
}

/** Whether a circle of world radius `r` at (`px`, `py`) touches `view`. */
export function circleInView(
  view: WorldRect,
  px: number,
  py: number,
  r: number,
): boolean {
  return (
    px + r >= view.minX &&
    px - r <= view.maxX &&
    py + r >= view.minY &&
    py - r <= view.maxY
  );
}

/** Whether the segment's bounding box touches `view` — a cheap, conservative
 * cull: it keeps a few edges that only pass near a corner. */
export function segmentInView(
  view: WorldRect,
  ax: number,
  ay: number,
  bx: number,
  by: number,
): boolean {
  return (
    Math.max(ax, bx) >= view.minX &&
    Math.min(ax, bx) <= view.maxX &&
    Math.max(ay, by) >= view.minY &&
    Math.min(ay, by) <= view.maxY
  );
}

/** Label type, all in screen pixels: labels are drawn outside the zoom
 * transform so they stay one size at every zoom. */
export const LABEL_FONT_PX = 12;
const LABEL_PAD_X = 5;
const LABEL_PAD_Y = 3;
const LABEL_GAP = 4;
/** Clear air kept around every node circle so no label sits on one. */
const LABEL_NODE_MARGIN = 4;
/** Most labels one frame will try to place, beyond the hovered and selected
 * node (#337). The pass used to consider every visible node, which past ~1.8x
 * zoom was all of them, and compare each against every label placed before it:
 * quadratic, every frame, for the whole settle. A screen holds far fewer
 * legible labels than this anyway. */
export const LABEL_BUDGET = 120;
/** Side of one occupancy-grid cell, in screen pixels. */
const LABEL_GRID_CELL = 48;
const LABEL_MAX_CHARS = 28;

/** The text a node's label shows: its title, cut at 28 characters. */
export function labelText(title: string): string {
  return title.length > LABEL_MAX_CHARS
    ? title.slice(0, LABEL_MAX_CHARS - 2) + "…"
    : title;
}

export interface LabelPlacement {
  node: SimNode;
  text: string;
  /** Box in screen pixels. */
  x: number;
  y: number;
  width: number;
  height: number;
  /** Where the text is drawn: centred on `textX`, top at `textY`. */
  textX: number;
  textY: number;
}

interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
  owner: SimNode;
}

/** Screen-space occupancy grid: a box is filed under every cell it covers, so
 * a collision check reads only the few cells a candidate covers instead of
 * every box placed so far. */
class OccupancyGrid {
  private cells = new Map<string, Box[]>();

  private forEachCell(box: Omit<Box, "owner">, visit: (key: string) => void) {
    const x0 = Math.floor(box.x / LABEL_GRID_CELL);
    const x1 = Math.floor((box.x + box.w) / LABEL_GRID_CELL);
    const y0 = Math.floor(box.y / LABEL_GRID_CELL);
    const y1 = Math.floor((box.y + box.h) / LABEL_GRID_CELL);
    for (let gx = x0; gx <= x1; gx++) {
      for (let gy = y0; gy <= y1; gy++) visit(`${gx},${gy}`);
    }
  }

  add(box: Box) {
    this.forEachCell(box, (key) => {
      const cell = this.cells.get(key);
      if (cell) cell.push(box);
      else this.cells.set(key, [box]);
    });
  }

  collides(box: Box): boolean {
    let hit = false;
    this.forEachCell(box, (key) => {
      if (hit) return;
      for (const other of this.cells.get(key) ?? []) {
        if (
          other.owner !== box.owner &&
          box.x < other.x + other.w &&
          box.x + box.w > other.x &&
          box.y < other.y + other.h &&
          box.y + box.h > other.y
        ) {
          hit = true;
          return;
        }
      }
    });
    return hit;
  }
}

export interface LabelLayoutInput {
  /** Nodes on screen that pass the tag filter — already culled to the view. */
  nodes: SimNode[];
  /** Always labelled, in this order, even over a collision: the hovered and
   * the selected node. */
  forced: SimNode[];
  transform: Transform;
  /** Backlink count at or above which a node is a hub: always a candidate,
   * and labelled below itself even when every side collides. */
  hubMinBacklinks: number;
  /** Width in screen pixels of `text` set at `LABEL_FONT_PX`. The caller
   * caches it; measuring text is the expensive part of a frame. */
  measure: (text: string) => number;
  budget?: number;
}

/**
 * Choose which node labels to draw and where, in screen space: hovered and
 * selected first, then hubs and nodes drawn large enough at this zoom, by
 * backlink count, at most `budget` of them. Each tries below, above, right,
 * then left of its node and takes the first spot clear of every node circle and
 * every label already placed.
 */
export function layoutLabels({
  nodes,
  forced,
  transform,
  hubMinBacklinks,
  measure,
  budget = LABEL_BUDGET,
}: LabelLayoutInput): LabelPlacement[] {
  const { x, y, k } = transform;
  const grid = new OccupancyGrid();
  for (const n of nodes) {
    const r = nodeScreenRadius(n.backlink_count, k) + LABEL_NODE_MARGIN;
    grid.add({
      x: n.x * k + x - r,
      y: n.y * k + y - r,
      w: r * 2,
      h: r * 2,
      owner: n,
    });
  }

  // Zoom-adaptive pre-filter: only hubs when zoomed out, more as zoom grows.
  const threshold = 10 / Math.sqrt(k);
  const forcedSet = new Set(forced);
  const ranked = nodes
    .filter(
      (n) =>
        !forcedSet.has(n) &&
        (n.backlink_count >= hubMinBacklinks ||
          nodeRadius(n.backlink_count) * k >= threshold),
    )
    .sort((a, b) => b.backlink_count - a.backlink_count)
    .slice(0, Math.max(0, budget));

  const placements: LabelPlacement[] = [];
  const height = LABEL_FONT_PX + LABEL_PAD_Y * 2;
  for (const node of [...forced, ...ranked]) {
    const text = labelText(node.title);
    const width = measure(text) + LABEL_PAD_X * 2;
    const r = nodeScreenRadius(node.backlink_count, k);
    const sx = node.x * k + x;
    const sy = node.y * k + y;
    const spots = [
      { x: sx - width / 2, y: sy + r + LABEL_GAP },
      { x: sx - width / 2, y: sy - r - LABEL_GAP - height },
      { x: sx + r + LABEL_GAP, y: sy - height / 2 },
      { x: sx - r - LABEL_GAP - width, y: sy - height / 2 },
    ];
    const guaranteed =
      forcedSet.has(node) || node.backlink_count >= hubMinBacklinks;
    let chosen = guaranteed ? spots[0] : null;
    for (const spot of spots) {
      if (!grid.collides({ ...spot, w: width, h: height, owner: node })) {
        chosen = spot;
        break;
      }
    }
    if (!chosen) continue;
    grid.add({ ...chosen, w: width, h: height, owner: node });
    placements.push({
      node,
      text,
      x: chosen.x,
      y: chosen.y,
      width,
      height,
      textX: chosen.x + width / 2,
      textY: chosen.y + LABEL_PAD_Y,
    });
  }
  return placements;
}

/** Backlink count of the 90th-percentile node: every node at or above it is
 * a hub and is always offered a label. */
export function hubThreshold(nodes: SimNode[]): number {
  const counts = nodes.map((n) => n.backlink_count).sort((a, b) => a - b);
  return counts[Math.floor(counts.length * 0.9)] ?? 0;
}

// ── island captions (#337) ──────────────────────────────────────────────────

export interface IslandCaptionMetrics {
  /** Vault-name type size, px. */
  nameSize: number;
  /** Count-line type size, px. */
  countSize: number;
  /** Enclosure top to the count line's baseline. */
  gap: number;
  /** Count baseline to name baseline. */
  lineHeight: number;
  /** Whole stack above the enclosure, enclosure top to the name's cap. */
  height: number;
}

/**
 * Island caption type in screen pixels at zoom `k`. Captions are drawn outside
 * the zoom transform (#337): drawn in world space they shrank with the fit, and
 * the all-Vault landing view on a phone set the Vault name at about 4px. They
 * still ease down as the view zooms out, so a caption does not swamp the
 * island above it, but never below a legible floor, and never above the
 * 20px/13px design size when zoomed in.
 */
export function islandCaptionMetrics(k: number): IslandCaptionMetrics {
  const scale = (size: number, floor: number) =>
    Math.max(floor, Math.min(size, size * k));
  const nameSize = scale(20, 13);
  const countSize = scale(13, 11);
  const gap = scale(16, 6);
  const lineHeight = countSize + nameSize * 0.55;
  return {
    nameSize,
    countSize,
    gap,
    lineHeight,
    height: gap + lineHeight + nameSize,
  };
}

export interface IslandCountLine {
  text: string;
  /** Which ink the line takes: the muted count ink, or a condition tier's. */
  tone: "muted" | "warn" | "error";
}

/**
 * An island caption's second line (#143, #337): `49 notes` for a Vault that
 * is answering normally, the condition word for one that is not, and
 * `indexing` in warn ink for a Vault whose index is building or has never been
 * built. The last case used to fall through to the count, so a Vault mid-build
 * read `0 notes` — the same caption a genuinely empty Vault gets.
 */
export function islandCountLine(
  slot: VaultSlotState,
  nodeCount: number,
): IslandCountLine {
  if (slot.kind === "condition") {
    return { text: slot.word, tone: slot.tier };
  }
  if (slot.kind === "indexing") {
    return { text: "indexing", tone: "warn" };
  }
  return {
    text: `${nodeCount} ${nodeCount === 1 ? "note" : "notes"}`,
    tone: "muted",
  };
}

/** Smallest zoom the all-Vault landing fit frames the field at (#337). A
 * 390px phone framed three small Vaults at about 0.22, and eight or more well
 * below that: a field of dots. Past this floor the reader pans instead. */
export const ISLAND_FIT_MIN_SCALE = 0.2;
/** The fit never zooms in past the single-graph landing scale: a lone small
 * Vault should not arrive magnified just because it is the only thing on
 * screen. */
export const ISLAND_FIT_MAX_SCALE = 0.9;
const ISLAND_FIT_PAD = 32;

/**
 * The transform that frames every island's enclosure (`bounds`, world units)
 * plus the caption stack above the top row in a `width` x `height` canvas.
 * Captions are sized in screen pixels (`islandCaptionMetrics`), so their
 * headroom is reserved in pixels. Its height depends on the zoom being chosen,
 * so the fit runs at the design size first and again at the size that zoom
 * gives.
 */
export function fitIslandField(
  bounds: WorldRect,
  width: number,
  height: number,
): Transform {
  const spanX = Math.max(1, bounds.maxX - bounds.minX);
  const spanY = Math.max(1, bounds.maxY - bounds.minY);
  const fitFor = (captionPx: number) =>
    Math.max(
      ISLAND_FIT_MIN_SCALE,
      Math.min(
        ISLAND_FIT_MAX_SCALE,
        (width - ISLAND_FIT_PAD * 2) / spanX,
        (height - ISLAND_FIT_PAD * 2 - captionPx) / spanY,
      ),
    );
  const k = fitFor(
    islandCaptionMetrics(fitFor(islandCaptionMetrics(1).height)).height,
  );
  const captionWorld = islandCaptionMetrics(k).height / k;
  return {
    x: width / 2 - ((bounds.minX + bounds.maxX) / 2) * k,
    y: height / 2 - ((bounds.minY - captionWorld + bounds.maxY) / 2) * k,
    k,
  };
}
