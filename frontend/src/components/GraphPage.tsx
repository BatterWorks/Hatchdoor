import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import {
  forceSimulation,
  forceLink,
  forceManyBody,
  forceCenter,
  forceCollide,
  type Simulation,
  type SimulationNodeDatum,
  type SimulationLinkDatum,
} from "d3-force";

import type { GraphData, GraphNode } from "../types";
import { StateBlock } from "./ui";

// ── simulation types ─────────────────────────────────────────────────────────

interface SimNode extends SimulationNodeDatum {
  slug: string;
  title: string;
  primary_tag: string | null;
  backlink_count: number;
  x: number;
  y: number;
}

interface SimLink extends SimulationLinkDatum<SimNode> {
  source: SimNode;
  target: SimNode;
}

// ── helpers ───────────────────────────────────────────────────────────────────

const BASE_RADIUS = 4;
const SCALE_FACTOR = 2.8;

function nodeRadius(backlinks: number): number {
  return BASE_RADIUS + Math.log(backlinks + 1) * SCALE_FACTOR;
}

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
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
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
  const simRef = useRef<Simulation<SimNode, SimLink> | null>(null);
  const dragRef = useRef<{ node: SimNode; startX: number; startY: number } | null>(null);
  const panRef = useRef<{ startX: number; startY: number; ox: number; oy: number } | null>(null);

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
        const res = await fetch("/api/graph");
        if (!res.ok) throw new Error(`Graph fetch failed: ${res.status}`);
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
    return () => { cancelled = true; };
  }, []);

  // ── hit test ────────────────────────────────────────────────────────────────

  const hitTest = useCallback((cx: number, cy: number): SimNode | null => {
    const { x, y, k } = transformRef.current;
    const wx = (cx - x) / k;
    const wy = (cy - y) / k;
    let best: SimNode | null = null;
    let bestDist = Infinity;
    for (const node of simNodesRef.current) {
      const r = nodeRadius(node.backlink_count);
      const d = Math.hypot(node.x - wx, node.y - wy);
      if (d <= r + 2 && d < bestDist) {
        best = node;
        bestDist = d;
      }
    }
    return best;
  }, []);

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
        // Re-centre world origin whenever the canvas is resized.
        transformRef.current = { x: cssW / 2, y: cssH / 2, k: transformRef.current.k };
      }
    }

    const W = canvas.width / dpr;
    const H = canvas.height / dpr;
    ctx.save();
    ctx.scale(dpr, dpr);

    const { x, y, k } = transformRef.current;
    const activeTags = activeTagsRef.current;
    const hovered = hoveredRef.current;
    const selected = selectedRef.current;

    // theme colors
    const bgColor = cssVar("--bg");
    const paperColor = cssVar("--paper");
    const ruleColor = cssVar("--rule");
    const inkColor = cssVar("--ink");
    const mutedColor = cssVar("--muted");
    const hotColor = cssVar("--hot");

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
      ctx.beginPath(); ctx.moveTo(gx, 0); ctx.lineTo(gx, H); ctx.stroke();
    }
    for (let gy = gridOffY; gy < H; gy += gridStep) {
      ctx.beginPath(); ctx.moveTo(0, gy); ctx.lineTo(W, gy); ctx.stroke();
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
        if (link.source.slug === selected.slug) connectedSlugs.add(link.target.slug);
        if (link.target.slug === selected.slug) connectedSlugs.add(link.source.slug);
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
        if (srcConn && tgtConn) { alpha = 0.55; color = hotColor; }
        else alpha = 0.04;
      } else if (hovered) {
        if (src.slug === hovered.slug || tgt.slug === hovered.slug) {
          alpha = 0.6; color = hotColor;
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
      ctx.lineWidth = selected && connectedSlugs.has(src.slug) && connectedSlugs.has(tgt.slug) ? 1.5 / k : 1 / k;
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
      ctx.strokeStyle = isSelected ? hotColor : isHovered ? inkColor : paperColor;
      ctx.lineWidth = (isSelected || isHovered ? 2 : 1) / k;
      ctx.globalAlpha = isHovered || isSelected ? 1 : alpha * 0.6;
      ctx.stroke();
      ctx.globalAlpha = 1;
    }

    // labels for hovered/selected node (show always at zoom >= 0.8)
    const labelCandidates: SimNode[] = [];
    if (hovered) labelCandidates.push(hovered);
    if (selected && selected !== hovered) labelCandidates.push(selected);
    if (k >= 0.8) {
      // show labels for highly-linked nodes
      for (const node of nodes) {
        if (node.backlink_count >= 5 && isVisible(node)) {
          if (!labelCandidates.find((n) => n.slug === node.slug)) {
            labelCandidates.push(node);
          }
        }
      }
    }

    for (const node of labelCandidates) {
      const r = nodeRadius(node.backlink_count);
      const isHov = node.slug === hovered?.slug;
      const isSel = node.slug === selected?.slug;
      const fontSize = Math.max(10, Math.min(14, 11 / k));

      ctx.save();
      ctx.font = `500 ${fontSize}px "Inter Tight", system-ui, sans-serif`;
      ctx.textAlign = "center";
      ctx.textBaseline = "top";

      const label = node.title.length > 28 ? node.title.slice(0, 26) + "…" : node.title;
      const metrics = ctx.measureText(label);
      const padX = 5 / k;
      const padY = 3 / k;
      const bw = metrics.width + padX * 2;
      const bh = fontSize + padY * 2;
      const bx = node.x - bw / 2;
      const by = node.y + r + 4 / k;

      ctx.globalAlpha = isSel ? 1 : isHov ? 0.95 : 0.75;
      ctx.fillStyle = paperColor;
      ctx.fillRect(bx, by, bw, bh);
      ctx.strokeStyle = ruleColor;
      ctx.lineWidth = 1 / k;
      ctx.strokeRect(bx, by, bw, bh);
      ctx.fillStyle = isSel ? hotColor : inkColor;
      ctx.fillText(label, node.x, by + padY);
      ctx.globalAlpha = 1;
      ctx.restore();
    }

    ctx.restore();
    ctx.restore();
  }, []);

  // ── animation loop ───────────────────────────────────────────────────────────

  const startLoop = useCallback(() => {
    const tick = () => {
      render();
      rafRef.current = requestAnimationFrame(tick);
    };
    rafRef.current = requestAnimationFrame(tick);
  }, [render]);

  // ── simulation setup ─────────────────────────────────────────────────────────

  useEffect(() => {
    if (!graphData) return;

    // Nodes live in world space centred at (0,0). The canvas transform maps
    // world (0,0) → canvas centre. Do NOT use canvas pixel dimensions here —
    // using them caused a double-shift that put every node off-screen.
    const spread = 500;
    const nodes: SimNode[] = graphData.nodes.map((n) => ({
      ...n,
      x: (Math.random() - 0.5) * spread,
      y: (Math.random() - 0.5) * spread,
    })) as SimNode[];

    const nodeBySlug = new Map<string, SimNode>(nodes.map((n) => [n.slug, n]));

    const links: SimLink[] = graphData.edges
      .map((e) => {
        const source = nodeBySlug.get(e.source);
        const target = nodeBySlug.get(e.target);
        if (!source || !target) return null;
        return { source, target } as SimLink;
      })
      .filter((l): l is SimLink => l !== null);

    simNodesRef.current = nodes;
    simLinksRef.current = links;

    // Centre transform on the canvas. The canvas is already sized by the
    // ResizeObserver so clientWidth/Height are reliable here.
    const canvas = canvasRef.current;
    const W = canvas?.clientWidth ?? 800;
    const H = canvas?.clientHeight ?? 600;
    transformRef.current = { x: W / 2, y: H / 2, k: 0.9 };

    simRef.current?.stop();
    const sim = forceSimulation<SimNode>(nodes)
      .force("link", forceLink<SimNode, SimLink>(links).id((d) => d.slug).distance(60).strength(0.4))
      .force("charge", forceManyBody<SimNode>().strength(-180).distanceMax(400))
      .force("center", forceCenter<SimNode>(0, 0))
      .force("collide", forceCollide<SimNode>().radius((d) => nodeRadius(d.backlink_count) + 4))
      .alphaDecay(0.02);

    simRef.current = sim;

    return () => { sim.stop(); };
  }, [graphData]);

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
        transformRef.current = { x: rect.width / 2, y: rect.height / 2, k: 0.9 };
      }
    };

    resize();
    const ro = new ResizeObserver(resize);
    ro.observe(wrap);
    return () => ro.disconnect();
  }, []);

  // ── start render loop ────────────────────────────────────────────────────────

  useEffect(() => {
    startLoop();
    return () => { cancelAnimationFrame(rafRef.current); };
  }, [startLoop]);

  // ── mouse/touch events ───────────────────────────────────────────────────────

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const getPos = (e: MouseEvent) => {
      const rect = canvas.getBoundingClientRect();
      return { cx: e.clientX - rect.left, cy: e.clientY - rect.top };
    };

    const onMouseMove = (e: MouseEvent) => {
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

      const hit = hitTest(cx, cy);
      if (hit !== hoveredRef.current) {
        hoveredRef.current = hit;
        canvas.style.cursor = hit ? "pointer" : "grab";
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
            selectedRef.current = selectedRef.current?.slug === node.slug ? null : node;
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
    };

    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const { cx, cy } = getPos(e);
      const factor = e.deltaY < 0 ? 1.1 : 0.9;
      const t = transformRef.current;
      const newK = Math.max(0.1, Math.min(8, t.k * factor));
      transformRef.current = {
        k: newK,
        x: cx - ((cx - t.x) / t.k) * newK,
        y: cy - ((cy - t.y) / t.k) * newK,
      };
    };

    const onMouseLeave = () => {
      hoveredRef.current = null;
      dragRef.current = null;
      panRef.current = null;
    };

    canvas.addEventListener("mousemove", onMouseMove);
    canvas.addEventListener("mousedown", onMouseDown);
    window.addEventListener("mouseup", onMouseUp);
    canvas.addEventListener("wheel", onWheel, { passive: false });
    canvas.addEventListener("mouseleave", onMouseLeave);

    return () => {
      canvas.removeEventListener("mousemove", onMouseMove);
      canvas.removeEventListener("mousedown", onMouseDown);
      window.removeEventListener("mouseup", onMouseUp);
      canvas.removeEventListener("wheel", onWheel);
      canvas.removeEventListener("mouseleave", onMouseLeave);
    };
  }, [hitTest, navigate]);

  // ── tag filter toggle ────────────────────────────────────────────────────────

  const toggleTag = useCallback((tag: string) => {
    setActiveTags((prev) => {
      const next = new Set(prev);
      if (next.has(tag)) next.delete(tag);
      else next.add(tag);
      return next;
    });
  }, []);

  // ── render ───────────────────────────────────────────────────────────────────

  if (loading) {
    return (
      <div className="graph-loading">
        <div className="graph-loading-pulse" />
        <div className="graph-loading-label">Mapping your vault…</div>
      </div>
    );
  }

  if (error || !graphData) {
    return (
      <StateBlock
        title="Graph Unavailable"
        description={error ?? "Could not load graph data."}
      />
    );
  }

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
        </div>
        {allTags.length > 0 && (
          <div className="graph-tag-filter">
            {allTags.map((tag) => {
              const hue = tagHue(tag);
              const active = activeTags.has(tag);
              return (
                <button
                  key={tag}
                  className={`graph-tag-chip${active ? " active" : ""}`}
                  style={active ? {
                    "--chip-hue": String(hue),
                  } as React.CSSProperties : undefined}
                  onClick={() => toggleTag(tag)}
                >
                  {active && <span className="graph-tag-dot" style={{ background: `hsl(${hue}, 60%, 58%)` }} />}
                  {tag}
                </button>
              );
            })}
          </div>
        )}
        <p className="graph-hint">
          Scroll to zoom · Drag background to pan · Click node to select · Double-click to open
        </p>
      </div>

      <div className="graph-canvas-wrap" ref={wrapRef}>
        <canvas ref={canvasRef} className="graph-canvas" />
      </div>
    </div>
  );
}
