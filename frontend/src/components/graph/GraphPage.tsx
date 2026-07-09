import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { type Simulation } from "d3-force";

import { apiFetch } from "../../api/api";
import { readErrorMessage } from "../../api/apiError";
import type { GraphData, GraphNode } from "../../types";
import { StateBlock } from "../ui";
import {
  buildSimulationGraph,
  createGraphSimulation,
  hitTest as hitTestNodes,
  nodeRadius,
  type SimLink,
  type SimNode,
} from "./graphSimulation";

// ── helpers ───────────────────────────────────────────────────────────────────

function tagHue(tag: string): number {
  let hash = 0;
  for (let i = 0; i < tag.length; i++) {
    hash = tag.charCodeAt(i) + ((hash << 5) - hash);
  }
  return Math.abs(hash) % 360;
}

function nodeColor(tag: string | null, alpha = 1): string {
  if (!tag) return `rgba(138, 134, 120, ${alpha})`;
  const hue = tagHue(tag);
  return `hsla(${hue}, 60%, 58%, ${alpha})`;
}

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement)
    .getPropertyValue(name)
    .trim();
}

interface ThemeColors {
  bg: string;
  paper: string;
  rule: string;
  ink: string;
  muted: string;
  hot: string;
}

// getComputedStyle forces a synchronous style recalc, so the six theme reads
// are resolved once here and cached — refreshed only when the theme changes
// rather than on every animation frame.
function readThemeColors(): ThemeColors {
  return {
    bg: cssVar("--bg"),
    paper: cssVar("--paper"),
    rule: cssVar("--rule"),
    ink: cssVar("--ink"),
    muted: cssVar("--muted"),
    hot: cssVar("--hot"),
  };
}

// ── component ─────────────────────────────────────────────────────────────────

export function GraphPage() {
  const navigate = useNavigate();
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const [graphData, setGraphData] = useState<GraphData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [allTags, setAllTags] = useState<string[]>([]);
  const [activeTags, setActiveTags] = useState<Set<string>>(new Set());
  const [nodeCount, setNodeCount] = useState(0);
  const [edgeCount, setEdgeCount] = useState(0);

  // mutable state shared between render loop and event handlers
  const simNodesRef = useRef<SimNode[]>([]);
  const simLinksRef = useRef<SimLink[]>([]);
  const transformRef = useRef({ x: 0, y: 0, k: 1 });
  const hoveredRef = useRef<SimNode | null>(null);
  const selectedRef = useRef<SimNode | null>(null);
  const activeTagsRef = useRef<Set<string>>(new Set());
  const lastClickSlugRef = useRef<string | null>(null);
  const rafRef = useRef<number>(0);
  const runningRef = useRef(false);
  const themeColorsRef = useRef<ThemeColors | null>(null);
  if (themeColorsRef.current === null)
    themeColorsRef.current = readThemeColors();
  const simRef = useRef<Simulation<SimNode, SimLink> | null>(null);
  const dragRef = useRef<{
    node: SimNode;
    startX: number;
    startY: number;
  } | null>(null);
  const panRef = useRef<{
    startX: number;
    startY: number;
    ox: number;
    oy: number;
  } | null>(null);
  const zoomAnimRef = useRef<{
    targetK: number;
    cx: number;
    cy: number;
  } | null>(null);
  const pinchRef = useRef<{ dist: number; cx: number; cy: number } | null>(
    null,
  );

  // keep activeTagsRef in sync
  useEffect(() => {
    activeTagsRef.current = activeTags;
  }, [activeTags]);

  // ── data fetch ──────────────────────────────────────────────────────────────

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      setLoading(true);
      setError(null);
      try {
        const res = await apiFetch("/api/graph");
        if (!res.ok)
          throw new Error(await readErrorMessage(res, "Graph fetch failed"));
        const data = (await res.json()) as GraphData;
        if (cancelled) return;

        setGraphData(data);
        setNodeCount(data.nodes.length);
        setEdgeCount(data.edges.length);

        const tags = Array.from(
          new Set(
            data.nodes
              .map((n: GraphNode) => n.primary_tag)
              .filter((t): t is string => t !== null),
          ),
        ).sort();
        setAllTags(tags);
      } catch (err) {
        if (!cancelled)
          setError(err instanceof Error ? err.message : "Failed to load graph");
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // ── hit test ────────────────────────────────────────────────────────────────

  const hitTest = useCallback(
    (cx: number, cy: number): SimNode | null =>
      hitTestNodes(simNodesRef.current, transformRef.current, cx, cy),
    [],
  );

  // ── canvas rendering ────────────────────────────────────────────────────────

  const render = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;

    // Sync physical canvas buffer to its CSS layout dimensions on every frame.
    // This is more reliable than the ResizeObserver alone when the height
    // chain is established after the observer's first fire.
    const cssW = canvas.clientWidth;
    const cssH = canvas.clientHeight;
    if (cssW > 0 && cssH > 0) {
      const needW = Math.round(cssW * dpr);
      const needH = Math.round(cssH * dpr);
      if (canvas.width !== needW || canvas.height !== needH) {
        canvas.width = needW;
        canvas.height = needH;
      }
    }

    // Animate zoom — lerp in log-space each frame so speed is perceptually even.
    if (zoomAnimRef.current) {
      const anim = zoomAnimRef.current;
      const t = transformRef.current;
      const logCurrent = Math.log(t.k);
      const logTarget = Math.log(anim.targetK);
      const diff = logTarget - logCurrent;
      const newK =
        Math.abs(diff) < 0.0008
          ? ((zoomAnimRef.current = null), anim.targetK)
          : Math.exp(logCurrent + diff * 0.18);
      transformRef.current = {
        k: newK,
        x: anim.cx - ((anim.cx - t.x) / t.k) * newK,
        y: anim.cy - ((anim.cy - t.y) / t.k) * newK,
      };
    }

    const W = canvas.width / dpr;
    const H = canvas.height / dpr;
    ctx.save();
    ctx.scale(dpr, dpr);

    const { x, y, k } = transformRef.current;
    const activeTags = activeTagsRef.current;
    const hovered = hoveredRef.current;
    const selected = selectedRef.current;

    // theme colors — cached; refreshed only on theme change, not per frame
    const theme = themeColorsRef.current ?? readThemeColors();
    const bgColor = theme.bg;
    const paperColor = theme.paper;
    const ruleColor = theme.rule;
    const inkColor = theme.ink;
    const mutedColor = theme.muted;
    const hotColor = theme.hot;

    // clear
    ctx.fillStyle = bgColor;
    ctx.fillRect(0, 0, W, H);

    // subtle grid
    ctx.save();
    ctx.strokeStyle = ruleColor;
    ctx.lineWidth = 0.5;
    ctx.globalAlpha = 0.4;
    const gridStep = 40 * k;
    const gridOffX = ((x % gridStep) + gridStep) % gridStep;
    const gridOffY = ((y % gridStep) + gridStep) % gridStep;
    for (let gx = gridOffX; gx < W; gx += gridStep) {
      ctx.beginPath();
      ctx.moveTo(gx, 0);
      ctx.lineTo(gx, H);
      ctx.stroke();
    }
    for (let gy = gridOffY; gy < H; gy += gridStep) {
      ctx.beginPath();
      ctx.moveTo(0, gy);
      ctx.lineTo(W, gy);
      ctx.stroke();
    }
    ctx.restore();

    ctx.save();
    ctx.translate(x, y);
    ctx.scale(k, k);

    const nodes = simNodesRef.current;
    const links = simLinksRef.current;

    // determine which nodes are "visible" based on tag filter
    const isVisible = (node: SimNode) => {
      if (activeTags.size === 0) return true;
      return node.primary_tag !== null && activeTags.has(node.primary_tag);
    };

    // connected slugs for selection highlight
    const connectedSlugs = new Set<string>();
    if (selected) {
      connectedSlugs.add(selected.slug);
      for (const link of links) {
        if (link.source.slug === selected.slug)
          connectedSlugs.add(link.target.slug);
        if (link.target.slug === selected.slug)
          connectedSlugs.add(link.source.slug);
      }
    }

    // draw edges
    for (const link of links) {
      const src = link.source;
      const tgt = link.target;
      const srcVis = isVisible(src);
      const tgtVis = isVisible(tgt);

      let alpha = 0.18;
      let color = mutedColor;

      if (selected) {
        const srcConn = connectedSlugs.has(src.slug);
        const tgtConn = connectedSlugs.has(tgt.slug);
        if (srcConn && tgtConn) {
          alpha = 0.55;
          color = hotColor;
        } else alpha = 0.04;
      } else if (hovered) {
        if (src.slug === hovered.slug || tgt.slug === hovered.slug) {
          alpha = 0.6;
          color = hotColor;
        } else {
          alpha = 0.06;
        }
      }

      if (!srcVis || !tgtVis) alpha *= 0.2;

      ctx.beginPath();
      ctx.moveTo(src.x, src.y);
      ctx.lineTo(tgt.x, tgt.y);
      ctx.strokeStyle = color;
      ctx.globalAlpha = alpha;
      ctx.lineWidth =
        selected && connectedSlugs.has(src.slug) && connectedSlugs.has(tgt.slug)
          ? 1.5 / k
          : 1 / k;
      ctx.stroke();
    }
    ctx.globalAlpha = 1;

    // draw nodes
    for (const node of nodes) {
      const r = nodeRadius(node.backlink_count);
      const vis = isVisible(node);
      const isHovered = hovered?.slug === node.slug;
      const isSelected = selected?.slug === node.slug;
      const isConnected = selected ? connectedSlugs.has(node.slug) : false;

      let alpha = vis ? 1 : 0.15;
      if (selected && !isConnected) alpha = vis ? 0.2 : 0.06;

      const color = node.primary_tag ? nodeColor(node.primary_tag) : mutedColor;

      ctx.globalAlpha = alpha;

      // glow for hovered/selected
      if (isHovered || isSelected) {
        ctx.beginPath();
        ctx.arc(node.x, node.y, r + 5 / k, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.globalAlpha = alpha * 0.2;
        ctx.fill();
        ctx.globalAlpha = alpha;
      }

      // node fill
      ctx.beginPath();
      ctx.arc(node.x, node.y, r, 0, Math.PI * 2);
      ctx.fillStyle = color;
      ctx.fill();

      // node border
      ctx.strokeStyle = isSelected
        ? hotColor
        : isHovered
          ? inkColor
          : paperColor;
      ctx.lineWidth = (isSelected || isHovered ? 2 : 1) / k;
      ctx.globalAlpha = isHovered || isSelected ? 1 : alpha * 0.6;
      ctx.stroke();
      ctx.globalAlpha = 1;
    }

    // Zoom-adaptive pre-filter: raise threshold when zoomed out so only hubs
    // are candidates; lower it as zoom increases to admit more nodes.
    const labelThreshold = 10 / Math.sqrt(k);
    const LABEL_SCREEN_SIZE = 12; // px on screen — constant regardless of zoom
    const LABEL_PAD_X = 5; // screen-px padding (converted to world below)
    const LABEL_PAD_Y = 3;

    // Hub threshold: top 10% by backlink count always get a label (guaranteed).
    const sortedCounts = nodes
      .map((n) => n.backlink_count)
      .sort((a, b) => a - b);
    const hubMinBacklinks =
      sortedCounts[Math.floor(sortedCounts.length * 0.9)] ?? 0;

    // Collect candidates: hovered/selected first, then hubs, then rest by importance.
    const seen = new Set<string>();
    const guaranteed = new Set<string>();
    const labelCandidates: SimNode[] = [];
    const pushLabel = (n: SimNode, force = false) => {
      if (!seen.has(n.slug)) {
        seen.add(n.slug);
        labelCandidates.push(n);
        if (force) guaranteed.add(n.slug);
      }
    };

    if (hovered) pushLabel(hovered, true);
    if (selected && selected !== hovered) pushLabel(selected, true);
    // Sort remaining candidates by importance so hubs win deconfliction.
    const ranked = nodes
      .filter(
        (n) =>
          isVisible(n) &&
          n.slug !== hovered?.slug &&
          n.slug !== selected?.slug &&
          (n.backlink_count >= hubMinBacklinks ||
            nodeRadius(n.backlink_count) * k >= labelThreshold),
      )
      .sort((a, b) => b.backlink_count - a.backlink_count);
    for (const n of ranked) pushLabel(n, n.backlink_count >= hubMinBacklinks);

    // Deconfliction: track occupied regions in screen space.
    // Pre-seed with every visible node circle so labels can't overlap nodes.
    // Each entry carries the owning slug so a node's own label can self-exclude.
    const NODE_MARGIN = 4; // extra px around each circle
    const placed: Array<{
      sx: number;
      sy: number;
      sw: number;
      sh: number;
      slug?: string;
    }> = nodes.filter(isVisible).map((n) => {
      const rScr = nodeRadius(n.backlink_count) * k + NODE_MARGIN;
      return {
        sx: n.x * k + x - rScr,
        sy: n.y * k + y - rScr,
        sw: rScr * 2,
        sh: rScr * 2,
        slug: n.slug,
      };
    });

    const fontSize = LABEL_SCREEN_SIZE / k;
    ctx.font = `500 ${fontSize}px "Inter Tight", system-ui, sans-serif`;

    const collidesWithPlaced = (
      sx: number,
      sy: number,
      sw: number,
      sh: number,
      ownSlug: string,
    ) =>
      placed.some(
        (p) =>
          p.slug !== ownSlug &&
          sx < p.sx + p.sw &&
          sx + sw > p.sx &&
          sy < p.sy + p.sh &&
          sy + sh > p.sy,
      );

    for (const node of labelCandidates) {
      const r = nodeRadius(node.backlink_count);
      const isHov = node.slug === hovered?.slug;
      const isSel = node.slug === selected?.slug;
      const isGuaranteed = guaranteed.has(node.slug);

      ctx.save();
      ctx.font = `500 ${fontSize}px "Inter Tight", system-ui, sans-serif`;
      ctx.textAlign = "center";
      ctx.textBaseline = "top";

      const label =
        node.title.length > 28 ? node.title.slice(0, 26) + "…" : node.title;
      const metrics = ctx.measureText(label);
      const padX = LABEL_PAD_X / k;
      const padY = LABEL_PAD_Y / k;
      const bw = metrics.width + padX * 2;
      const bh = fontSize + padY * 2;
      const gap = 4 / k;

      // Candidate positions: below, above, right, left.
      const candidates = [
        { bx: node.x - bw / 2, by: node.y + r + gap },
        { bx: node.x - bw / 2, by: node.y - r - gap - bh },
        { bx: node.x + r + gap, by: node.y - bh / 2 },
        { bx: node.x - r - gap - bw, by: node.y - bh / 2 },
      ];

      // Pick the first position that doesn't collide with any placed region.
      // Guaranteed nodes fall back to the default (below) if nothing is clear.
      let chosen = isGuaranteed ? candidates[0] : null;
      for (const pos of candidates) {
        const sx = pos.bx * k + x;
        const sy = pos.by * k + y;
        const sw = bw * k;
        const sh = bh * k;
        if (!collidesWithPlaced(sx, sy, sw, sh, node.slug)) {
          chosen = pos;
          break;
        }
      }

      if (chosen) {
        const { bx, by } = chosen;
        const sx = bx * k + x;
        const sy = by * k + y;
        const sw = bw * k;
        const sh = bh * k;
        placed.push({ sx, sy, sw, sh, slug: node.slug });

        ctx.globalAlpha = isSel ? 1 : isHov ? 0.95 : 0.75;
        ctx.fillStyle = paperColor;
        ctx.fillRect(bx, by, bw, bh);
        ctx.strokeStyle = ruleColor;
        ctx.lineWidth = 1 / k;
        ctx.strokeRect(bx, by, bw, bh);
        ctx.fillStyle = isSel ? hotColor : inkColor;
        ctx.fillText(label, bx + bw / 2, by + padY);
        ctx.globalAlpha = 1;
      }

      ctx.restore();
    }

    ctx.restore();
    ctx.restore();
  }, []);

  // ── animation loop ───────────────────────────────────────────────────────────

  // Draw one frame and keep looping only while something is actually moving
  // (simulation still warm, an animated zoom, or an active drag/pan/pinch).
  // Once everything settles the rAF loop stops entirely instead of spinning a
  // core at 60fps forever; interaction handlers call requestRender() to wake it.
  const requestRender = useCallback(() => {
    if (runningRef.current) return;
    runningRef.current = true;
    const tick = () => {
      render();
      const sim = simRef.current;
      const simActive = !!sim && sim.alpha() > sim.alphaMin();
      const busy =
        simActive ||
        zoomAnimRef.current !== null ||
        dragRef.current !== null ||
        panRef.current !== null ||
        pinchRef.current !== null;
      if (busy) {
        rafRef.current = requestAnimationFrame(tick);
      } else {
        runningRef.current = false;
      }
    };
    rafRef.current = requestAnimationFrame(tick);
  }, [render]);

  // ── simulation setup ─────────────────────────────────────────────────────────

  useEffect(() => {
    if (!graphData) return;

    // Nodes live in world space centred at (0,0). The canvas transform maps
    // world (0,0) → canvas centre. Do NOT use canvas pixel dimensions here —
    // using them caused a double-shift that put every node off-screen.
    const { nodes, links } = buildSimulationGraph(graphData);

    simNodesRef.current = nodes;
    simLinksRef.current = links;

    // Centre transform on the canvas. The canvas is already sized by the
    // ResizeObserver so clientWidth/Height are reliable here.
    const canvas = canvasRef.current;
    const W = canvas?.clientWidth ?? 800;
    const H = canvas?.clientHeight ?? 600;
    transformRef.current = { x: W / 2, y: H / 2, k: 0.9 };

    simRef.current?.stop();
    const sim = createGraphSimulation(nodes, links);

    simRef.current = sim;
    requestRender();

    return () => {
      sim.stop();
    };
  }, [graphData, requestRender]);

  // ── canvas resize ───────────────────────────────────────────────────────────

  useEffect(() => {
    const canvas = canvasRef.current;
    const wrap = wrapRef.current;
    if (!canvas || !wrap) return;

    let centred = false;

    const resize = () => {
      const dpr = window.devicePixelRatio || 1;
      const rect = wrap.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) return;
      canvas.width = rect.width * dpr;
      canvas.height = rect.height * dpr;
      canvas.style.width = `${rect.width}px`;
      canvas.style.height = `${rect.height}px`;
      // Re-centre the world origin on first valid size so the graph
      // is always visible regardless of when the sim initialised.
      if (!centred) {
        centred = true;
        transformRef.current = {
          x: rect.width / 2,
          y: rect.height / 2,
          k: 0.9,
        };
      }
      requestRender();
    };

    resize();
    const ro = new ResizeObserver(resize);
    ro.observe(wrap);
    return () => ro.disconnect();
  }, [requestRender]);

  // ── start render loop ────────────────────────────────────────────────────────

  useEffect(() => {
    requestRender();
    return () => {
      cancelAnimationFrame(rafRef.current);
      runningRef.current = false;
    };
  }, [requestRender]);

  // Refresh cached theme colors when the theme changes (data-theme attribute
  // for explicit themes, media query for the "auto" theme) and redraw once.
  useEffect(() => {
    const refresh = () => {
      themeColorsRef.current = readThemeColors();
      requestRender();
    };
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const observer = new MutationObserver(refresh);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });
    mq.addEventListener("change", refresh);
    return () => {
      observer.disconnect();
      mq.removeEventListener("change", refresh);
    };
  }, [requestRender]);

  // Redraw when the tag filter changes (state only touches refs otherwise).
  useEffect(() => {
    requestRender();
  }, [activeTags, requestRender]);

  // ── mouse/touch events ───────────────────────────────────────────────────────

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const getPos = (e: MouseEvent) => {
      const rect = canvas.getBoundingClientRect();
      return { cx: e.clientX - rect.left, cy: e.clientY - rect.top };
    };

    // window-level move handler used during drag/pan so events keep firing
    // even when the cursor leaves the canvas element.
    const onWindowMouseMove = (e: MouseEvent) => {
      const { cx, cy } = getPos(e);

      if (panRef.current) {
        const dx = cx - panRef.current.startX;
        const dy = cy - panRef.current.startY;
        transformRef.current.x = panRef.current.ox + dx;
        transformRef.current.y = panRef.current.oy + dy;
        return;
      }

      if (dragRef.current) {
        const { k, x, y } = transformRef.current;
        const wx = (cx - x) / k;
        const wy = (cy - y) / k;
        dragRef.current.node.x = wx;
        dragRef.current.node.y = wy;
        dragRef.current.node.fx = wx;
        dragRef.current.node.fy = wy;
        simRef.current?.alphaTarget(0.1).restart();
        return;
      }
    };

    const onMouseMove = (e: MouseEvent) => {
      if (panRef.current || dragRef.current) return; // handled by window listener

      const { cx, cy } = getPos(e);
      const hit = hitTest(cx, cy);
      if (hit !== hoveredRef.current) {
        hoveredRef.current = hit;
        canvas.style.cursor = hit ? "pointer" : "grab";
        requestRender();
      }
    };

    const onMouseDown = (e: MouseEvent) => {
      if (e.button !== 0) return;
      const { cx, cy } = getPos(e);
      const hit = hitTest(cx, cy);

      if (hit) {
        dragRef.current = { node: hit, startX: cx, startY: cy };
        canvas.style.cursor = "grabbing";
      } else {
        panRef.current = {
          startX: cx,
          startY: cy,
          ox: transformRef.current.x,
          oy: transformRef.current.y,
        };
        canvas.style.cursor = "grabbing";
      }
      requestRender();
    };

    const onMouseUp = (e: MouseEvent) => {
      const { cx, cy } = getPos(e);

      if (dragRef.current) {
        const movedX = Math.abs(cx - dragRef.current.startX);
        const movedY = Math.abs(cy - dragRef.current.startY);
        const moved = movedX > 4 || movedY > 4;

        if (!moved) {
          const node = dragRef.current.node;
          if (lastClickSlugRef.current === node.slug) {
            void navigate(`/n/${node.slug}`);
            lastClickSlugRef.current = null;
          } else {
            selectedRef.current =
              selectedRef.current?.slug === node.slug ? null : node;
            lastClickSlugRef.current = node.slug;
            setTimeout(() => {
              if (lastClickSlugRef.current === node.slug) {
                lastClickSlugRef.current = null;
              }
            }, 500);
          }
        }

        dragRef.current.node.fx = null;
        dragRef.current.node.fy = null;
        simRef.current?.alphaTarget(0).restart();
        dragRef.current = null;
      } else if (panRef.current) {
        const movedX = Math.abs(cx - panRef.current.startX);
        const movedY = Math.abs(cy - panRef.current.startY);
        if (movedX < 4 && movedY < 4) {
          selectedRef.current = null;
          lastClickSlugRef.current = null;
        }
        panRef.current = null;
      }

      canvas.style.cursor = hitTest(cx, cy) ? "pointer" : "grab";
      requestRender();
    };

    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const { cx, cy } = getPos(e);
      // Proportional factor: works naturally for both mouse wheels (~120/notch)
      // and trackpad gestures (small continuous deltas).
      const factor = Math.pow(0.999, e.deltaY);
      const baseK = zoomAnimRef.current?.targetK ?? transformRef.current.k;
      const targetK = Math.max(0.1, Math.min(8, baseK * factor));
      zoomAnimRef.current = { targetK, cx, cy };
      requestRender();
    };

    const onMouseLeave = () => {
      hoveredRef.current = null;
      dragRef.current = null;
      panRef.current = null;
      requestRender();
    };

    // ── touch helpers ────────────────────────────────────────────────────────

    const getTouchPos = (t: Touch) => {
      const rect = canvas.getBoundingClientRect();
      return { cx: t.clientX - rect.left, cy: t.clientY - rect.top };
    };

    const onTouchStart = (e: TouchEvent) => {
      e.preventDefault();
      requestRender();

      if (e.touches.length === 2) {
        // Begin pinch — cancel any ongoing pan/drag
        dragRef.current = null;
        panRef.current = null;
        zoomAnimRef.current = null;
        const a = getTouchPos(e.touches[0]);
        const b = getTouchPos(e.touches[1]);
        pinchRef.current = {
          dist: Math.hypot(b.cx - a.cx, b.cy - a.cy),
          cx: (a.cx + b.cx) / 2,
          cy: (a.cy + b.cy) / 2,
        };
        return;
      }

      if (e.touches.length === 1) {
        pinchRef.current = null;
        const { cx, cy } = getTouchPos(e.touches[0]);
        const hit = hitTest(cx, cy);
        if (hit) {
          dragRef.current = { node: hit, startX: cx, startY: cy };
        } else {
          panRef.current = {
            startX: cx,
            startY: cy,
            ox: transformRef.current.x,
            oy: transformRef.current.y,
          };
        }
      }
    };

    const onTouchMove = (e: TouchEvent) => {
      e.preventDefault();

      if (e.touches.length === 2 && pinchRef.current) {
        const a = getTouchPos(e.touches[0]);
        const b = getTouchPos(e.touches[1]);
        const newDist = Math.hypot(b.cx - a.cx, b.cy - a.cy);
        const midCx = (a.cx + b.cx) / 2;
        const midCy = (a.cy + b.cy) / 2;
        const factor = newDist / pinchRef.current.dist;
        const t = transformRef.current;
        const newK = Math.max(0.1, Math.min(8, t.k * factor));
        // Also pan with the midpoint so two-finger drag works simultaneously
        const panDx = midCx - pinchRef.current.cx;
        const panDy = midCy - pinchRef.current.cy;
        transformRef.current = {
          k: newK,
          x: midCx - ((pinchRef.current.cx - t.x) / t.k) * newK + panDx,
          y: midCy - ((pinchRef.current.cy - t.y) / t.k) * newK + panDy,
        };
        pinchRef.current = { dist: newDist, cx: midCx, cy: midCy };
        return;
      }

      if (e.touches.length === 1) {
        const { cx, cy } = getTouchPos(e.touches[0]);

        if (panRef.current) {
          transformRef.current.x =
            panRef.current.ox + (cx - panRef.current.startX);
          transformRef.current.y =
            panRef.current.oy + (cy - panRef.current.startY);
          return;
        }

        if (dragRef.current) {
          const { k, x, y } = transformRef.current;
          const wx = (cx - x) / k;
          const wy = (cy - y) / k;
          dragRef.current.node.x = wx;
          dragRef.current.node.y = wy;
          dragRef.current.node.fx = wx;
          dragRef.current.node.fy = wy;
          simRef.current?.alphaTarget(0.1).restart();
        }
      }
    };

    const onTouchEnd = (e: TouchEvent) => {
      e.preventDefault();
      requestRender();

      if (e.touches.length >= 1) {
        // One finger lifted while two were down — transition to single-finger pan
        pinchRef.current = null;
        const { cx, cy } = getTouchPos(e.touches[0]);
        panRef.current = {
          startX: cx,
          startY: cy,
          ox: transformRef.current.x,
          oy: transformRef.current.y,
        };
        return;
      }

      // All fingers lifted
      pinchRef.current = null;
      const last = e.changedTouches[0];
      const { cx, cy } = getTouchPos(last);

      if (dragRef.current) {
        const moved =
          Math.abs(cx - dragRef.current.startX) > 8 ||
          Math.abs(cy - dragRef.current.startY) > 8;

        if (!moved) {
          const node = dragRef.current.node;
          if (lastClickSlugRef.current === node.slug) {
            void navigate(`/n/${node.slug}`);
            lastClickSlugRef.current = null;
          } else {
            selectedRef.current =
              selectedRef.current?.slug === node.slug ? null : node;
            lastClickSlugRef.current = node.slug;
            setTimeout(() => {
              if (lastClickSlugRef.current === node.slug)
                lastClickSlugRef.current = null;
            }, 500);
          }
        }

        dragRef.current.node.fx = null;
        dragRef.current.node.fy = null;
        simRef.current?.alphaTarget(0).restart();
        dragRef.current = null;
      } else if (panRef.current) {
        const moved =
          Math.abs(cx - panRef.current.startX) > 8 ||
          Math.abs(cy - panRef.current.startY) > 8;
        if (!moved) {
          selectedRef.current = null;
          lastClickSlugRef.current = null;
        }
        panRef.current = null;
      }
    };

    canvas.addEventListener("mousemove", onMouseMove);
    canvas.addEventListener("mousedown", onMouseDown);
    window.addEventListener("mousemove", onWindowMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    canvas.addEventListener("wheel", onWheel, { passive: false });
    canvas.addEventListener("mouseleave", onMouseLeave);
    canvas.addEventListener("touchstart", onTouchStart, { passive: false });
    canvas.addEventListener("touchmove", onTouchMove, { passive: false });
    canvas.addEventListener("touchend", onTouchEnd, { passive: false });

    return () => {
      canvas.removeEventListener("mousemove", onMouseMove);
      canvas.removeEventListener("mousedown", onMouseDown);
      window.removeEventListener("mousemove", onWindowMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
      canvas.removeEventListener("wheel", onWheel);
      canvas.removeEventListener("mouseleave", onMouseLeave);
      canvas.removeEventListener("touchstart", onTouchStart);
      canvas.removeEventListener("touchmove", onTouchMove);
      canvas.removeEventListener("touchend", onTouchEnd);
    };
  }, [hitTest, navigate, requestRender]);

  // ── tag filter toggle ────────────────────────────────────────────────────────

  const [filterOpen, setFilterOpen] = useState(false);

  const toggleTag = useCallback((tag: string) => {
    setActiveTags((prev) => {
      const next = new Set(prev);
      if (next.has(tag)) next.delete(tag);
      else next.add(tag);
      return next;
    });
  }, []);

  // ── render ───────────────────────────────────────────────────────────────────

  // NOTE: canvas is always in the DOM so that refs are valid on mount and
  // effects (resize observer, event listeners) attach correctly.  Loading and
  // error states are rendered as absolutely-positioned overlays instead.

  const tagChips = allTags.length > 0 && (
    <div className="graph-tag-filter">
      {allTags.map((tag) => {
        const hue = tagHue(tag);
        const active = activeTags.has(tag);
        return (
          <button
            key={tag}
            className={`graph-tag-chip${active ? " active" : ""}`}
            style={
              active
                ? ({ "--chip-hue": String(hue) } as React.CSSProperties)
                : undefined
            }
            onClick={() => toggleTag(tag)}
          >
            {active && (
              <span
                className="graph-tag-dot"
                style={{ background: `hsl(${hue}, 60%, 58%)` }}
              />
            )}
            {tag}
          </button>
        );
      })}
    </div>
  );

  return (
    <div className="graph-page">
      <div className="graph-header">
        <p className="graph-eyebrow">Vault · Knowledge Graph</p>

        <div className="graph-header-row">
          <h1 className="graph-title">Graph</h1>
          <div className="graph-meta-strip">
            <span className="graph-meta-item">
              <span className="graph-meta-num">{nodeCount}</span>
              <span className="graph-meta-lbl">nodes</span>
            </span>
            <span className="graph-meta-sep" />
            <span className="graph-meta-item">
              <span className="graph-meta-num">{edgeCount}</span>
              <span className="graph-meta-lbl">edges</span>
            </span>
          </div>

          {allTags.length > 0 && (
            <button
              className={`graph-filter-toggle${activeTags.size > 0 ? " has-active" : ""}${filterOpen ? " is-open" : ""}`}
              onClick={() => setFilterOpen((o) => !o)}
              aria-expanded={filterOpen}
            >
              <span className="graph-filter-toggle-label">
                Tags
                {activeTags.size > 0 && (
                  <span className="graph-filter-badge">{activeTags.size}</span>
                )}
              </span>
              <span className="graph-filter-caret" aria-hidden>
                ▾
              </span>
            </button>
          )}
        </div>

        {/* Desktop: always visible inline. Mobile: hidden, rendered as overlay below. */}
        <div className="graph-tags-desktop">{tagChips}</div>

        <p className="graph-hint">
          Scroll to zoom · Drag background to pan · Click node to select ·
          Double-click to open
        </p>
      </div>

      {/* Mobile-only filter overlay — sits on top of canvas, doesn't push it */}
      {filterOpen && (
        <button
          className="graph-filter-backdrop"
          aria-label="Close filter"
          onClick={() => setFilterOpen(false)}
        />
      )}
      <div className="graph-tags-mobile" aria-hidden={!filterOpen}>
        {tagChips}
      </div>

      <div className="graph-canvas-wrap" ref={wrapRef}>
        <canvas ref={canvasRef} className="graph-canvas" />

        {loading && (
          <div className="graph-overlay">
            <div className="graph-loading-pulse" />
            <div className="graph-loading-label">Mapping your vault…</div>
          </div>
        )}

        {!loading && (error || !graphData) && (
          <div className="graph-overlay">
            <StateBlock
              title="Graph Unavailable"
              description={error ?? "Could not load graph data."}
            />
          </div>
        )}
      </div>
    </div>
  );
}
