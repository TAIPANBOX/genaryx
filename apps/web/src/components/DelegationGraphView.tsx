import type { PointerEvent as ReactPointerEvent } from "react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { fetchAgentGraph, shortAgentLabel } from "../lib/graph";
import { useConsoleStateVersion } from "../lib/consoleState";
import type { EntityLifecycleState } from "../lib/lifecycleTypes";
import type { GraphEdge, LayoutView, NodeKind, PositionedNode } from "../graphTypes";

/** The graph is a live analytic (PHASE3 §5.1: built incrementally in core
 * from the bus, unlike Idryx's load-once snapshot) - unlike `IdentityView`'s
 * deliberate no-auto-refresh, a periodic re-fetch here is the honest
 * behavior. 5s (vs. Money/Policy's 20s) because the demo feeder appends a
 * new event roughly every 2s and the graph is the one view meant to visibly
 * breathe with the bus. */
const REFRESH_INTERVAL_MS = 5_000;

const NODE_KIND_VAR: Record<NodeKind, string> = {
  user: "--src-engram",
  agent: "--src-qryx",
  other: "--faint",
};

/** A blocked node is tinted by its lifecycle so a stopped/frozen/killed agent
 * is visible on the graph, not just its `kind` colour. A live node keeps its
 * kind colour (no override). */
const NODE_LIFECYCLE_VAR: Record<EntityLifecycleState, string | null> = {
  live: null,
  stopped: "--amber",
  frozen: "--iris",
  killed: "--sev-critical",
};

const NODE_KIND_LABEL: Record<NodeKind, string> = {
  user: "user",
  agent: "agent",
  other: "other",
};

/** The graph's nodes are not all agents: it also carries the human `user`
 * roots and any `other` principals (see the legend). Summarise the mix by kind
 * so the header count never claims "N agents" for what includes users. */
function nodeSummary(nodes: readonly { kind: NodeKind }[]): string {
  let agents = 0;
  let users = 0;
  let other = 0;
  for (const n of nodes) {
    if (n.kind === "agent") agents += 1;
    else if (n.kind === "user") users += 1;
    else other += 1;
  }
  const parts = [`${agents} agents`, `${users} users`];
  if (other > 0) parts.push(`${other} other`);
  return parts.join(" · ");
}

const MIN_RADIUS = 5;
const MAX_RADIUS = 22;
const MIN_ZOOM = 0.15;
const MAX_ZOOM = 6;
const ZOOM_STEP = 1.25;

interface Transform {
  x: number;
  y: number;
  scale: number;
}

const IDENTITY_TRANSFORM: Transform = { x: 0, y: 0, scale: 1 };

/** Resolve a `--css-var` to its current computed color string, theme- and
 * light/dark-aware by construction (reads whatever `index.css` currently has
 * in effect for `:root`/`:root[data-theme]`) - canvas drawing cannot use CSS
 * variables directly, so every color used below is resolved once per draw
 * pass through this. */
function resolveVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

/** Event-count -> radius, sqrt-scaled so *area* (not radius) is proportional
 * to event_count - the standard magnitude-encoding convention, avoiding a
 * high-volume node visually dominating out of proportion to its actual
 * share. A graph with every node at 0 events (a pure delegation skeleton,
 * no acting agent seen yet) still renders every node at [`MIN_RADIUS`]. */
function radiusFor(eventCount: number, maxCount: number): number {
  if (maxCount <= 0) return MIN_RADIUS;
  const t = Math.sqrt(Math.max(0, eventCount) / maxCount);
  return MIN_RADIUS + t * (MAX_RADIUS - MIN_RADIUS);
}

/** The 1-hop neighborhood of `id` (both directions) - shared by hover
 * highlighting and Agent-360 focus-mode dimming, so "neighborhood" means the
 * exact same thing in both places. */
function neighborsOf(id: string, edges: readonly GraphEdge[]): Set<string> {
  const out = new Set<string>();
  for (const e of edges) {
    if (e.from === id) out.add(e.to);
    if (e.to === id) out.add(e.from);
  }
  return out;
}

/** One directed edge, drawn as a line stopping short of the target node's
 * boundary plus a small arrowhead - `headLen`/`width` are already
 * screen-constant (pre-divided by the caller's zoom scale), so strokes read
 * the same weight at any zoom level while node/edge *positions* still scale
 * spatially with zoom. */
function drawEdge(
  ctx: CanvasRenderingContext2D,
  from: PositionedNode,
  to: PositionedNode,
  toRadius: number,
  opts: { color: string; alpha: number; width: number; headLen: number },
) {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const dist = Math.hypot(dx, dy);
  if (dist < 0.001) return;
  const ux = dx / dist;
  const uy = dy / dist;
  const endX = to.x - ux * (toRadius + 2);
  const endY = to.y - uy * (toRadius + 2);

  ctx.save();
  ctx.globalAlpha = opts.alpha;
  ctx.strokeStyle = opts.color;
  ctx.fillStyle = opts.color;
  ctx.lineWidth = opts.width;
  ctx.beginPath();
  ctx.moveTo(from.x, from.y);
  ctx.lineTo(endX, endY);
  ctx.stroke();

  const angle = Math.atan2(dy, dx);
  const spread = Math.PI / 7;
  ctx.beginPath();
  ctx.moveTo(endX, endY);
  ctx.lineTo(endX - opts.headLen * Math.cos(angle - spread), endY - opts.headLen * Math.sin(angle - spread));
  ctx.lineTo(endX - opts.headLen * Math.cos(angle + spread), endY - opts.headLen * Math.sin(angle + spread));
  ctx.closePath();
  ctx.fill();
  ctx.restore();
}

function Legend() {
  const items: { kind: NodeKind; label: string }[] = [
    { kind: "user", label: "user" },
    { kind: "agent", label: "agent" },
    { kind: "other", label: "other" },
  ];
  return (
    <div className="flex items-center gap-3">
      {items.map((it) => (
        <span key={it.kind} className="inline-flex items-center gap-1.5 mono text-[10.5px]" style={{ color: "var(--faint)" }}>
          <span
            aria-hidden="true"
            style={{
              display: "inline-block",
              width: 8,
              height: 8,
              borderRadius: "50%",
              background: `var(${NODE_KIND_VAR[it.kind]})`,
            }}
          />
          {it.label}
        </span>
      ))}
      <span className="mono text-[10.5px]" style={{ color: "var(--faint)" }}>
        radius &middot; event volume
      </span>
    </div>
  );
}

/**
 * The delegation graph (docs/PHASE3.md W3, position 3): a Canvas2D-only
 * renderer (no WebGL - the parity trap this whole architecture position
 * exists to avoid) over the core-computed `LayoutView`. Nodes are circles
 * colored by `kind`, radius scaled by `event_count`; edges are directed
 * lines with an arrowhead. Pan (drag) + zoom (wheel, or the +/-/0 toolbar)
 * + hover-to-highlight a node and its edges; clicking a node deep-links to
 * its Agent 360 via `onOpenAgent`.
 *
 * Two modes, one component (so there is exactly one renderer to keep
 * correct, not two):
 * - **Standalone** (`focusAgentId` unset): the "Graph" nav view, the whole
 *   graph fit to the canvas on load.
 * - **Focus** (`focusAgentId` set, `compact`): embedded in Agent 360 as the
 *   Delegation section's mini-focus - fits to the focused node's 1-hop
 *   neighborhood and dims everything outside it, using the SAME node
 *   positions the standalone view would (the core layout is one call,
 *   shared - PHASE3's parity requirement: "the same node positions in both
 *   shells", and here, in both modes of this one shell).
 *
 * Reduced-motion-safe by construction: there is no animation loop or CSS
 * transition anywhere in this component (every redraw is a direct,
 * synchronous response to data or a user gesture), so there is no motion to
 * gate behind `prefers-reduced-motion` in the first place.
 */
export function DelegationGraphView({
  onOpenAgent,
  focusAgentId = null,
  height = 520,
  compact = false,
  fill = false,
}: {
  onOpenAgent: (agentId: string) => void;
  focusAgentId?: string | null;
  /** Fixed pixel height for the canvas panel. Ignored when `fill` is true. */
  height?: number;
  /** Embedded/mini-focus styling (Agent 360): hides the toolbar, tighter
   * chrome. Independent of `fill`/`height` - a compact view can still be a
   * fixed height (the usual case, inside a card) or fill its parent. */
  compact?: boolean;
  /** Fill the parent's box instead of a fixed `height` - used by the
   * standalone "Graph" nav view, whose parent is itself a flex cell sized by
   * the app shell's own layout rather than a number this component could
   * know in advance. The parent must actually constrain height (e.g.
   * `flex-1 min-h-0`) for this to mean anything. */
  fill?: boolean;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const [layout, setLayout] = useState<LayoutView | null>(null);
  // Always the container's ACTUALLY rendered box (both dimensions read from
  // the ResizeObserver below), never echoed from the `height` prop - in
  // `fill` mode there is no prop to echo, and even in fixed-height mode the
  // rendered box is the ground truth (borders/box-sizing could differ from
  // the requested number).
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [transform, setTransform] = useState<Transform>(IDENTITY_TRANSFORM);
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const [asOfMs, setAsOfMs] = useState<number | null>(null);

  const fitKeyRef = useRef<string | null>(null);
  const dragRef = useRef<{ startX: number; startY: number; origin: Transform; moved: boolean } | null>(null);

  const load = useCallback(async () => {
    const lv = await fetchAgentGraph();
    setLayout(lv);
    setAsOfMs(Date.now());
  }, []);

  useEffect(() => {
    void load();
    const id = window.setInterval(() => void load(), REFRESH_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [load]);

  // Re-read immediately when any lifecycle action lands, so a stopped/frozen/
  // killed node re-tints within a beat rather than only on the next 5s poll.
  const consoleVersion = useConsoleStateVersion();
  useEffect(() => {
    void load();
  }, [consoleVersion, load]);

  // Track the container's actual rendered pixel box (starts unmeasured at 0
  // so the fit-to-view effect below never fits against a guessed default).
  // Fires for both the fixed-`height` and `fill` cases identically - this
  // component never needs to know which mode produced the box it is given.
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      setSize({ width: entry.contentRect.width, height: entry.contentRect.height });
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const neighborhood = useMemo(() => {
    if (!focusAgentId || !layout) return null;
    const set = neighborsOf(focusAgentId, layout.edges);
    set.add(focusAgentId);
    return set;
  }, [focusAgentId, layout]);

  const fitToView = useCallback(() => {
    if (!layout || layout.nodes.length === 0 || size.width === 0) return;
    const targets = neighborhood ? layout.nodes.filter((n) => neighborhood.has(n.id)) : layout.nodes;
    const pts = targets.length > 0 ? targets : layout.nodes;
    const minX = Math.min(...pts.map((n) => n.x));
    const maxX = Math.max(...pts.map((n) => n.x));
    const minY = Math.min(...pts.map((n) => n.y));
    const maxY = Math.max(...pts.map((n) => n.y));
    const gw = Math.max(maxX - minX, 1);
    const gh = Math.max(maxY - minY, 1);
    const margin = focusAgentId ? 70 : 50;
    const rawScale = Math.min((size.width - margin * 2) / gw, (size.height - margin * 2) / gh);
    const scale = Math.max(MIN_ZOOM, Math.min(rawScale, focusAgentId ? 2.4 : MAX_ZOOM));
    const cx = (minX + maxX) / 2;
    const cy = (minY + maxY) / 2;
    setTransform({ scale, x: size.width / 2 - cx * scale, y: size.height / 2 - cy * scale });
  }, [layout, neighborhood, focusAgentId, size.width, size.height]);

  // Fit once per distinct (focus target, graph shape) - never on every 5s
  // poll tick, so a manual pan/zoom the operator just did is not clobbered
  // by the next background refresh. "Reset view" (below) bypasses this
  // guard on demand.
  useLayoutEffect(() => {
    if (!layout || layout.nodes.length === 0 || size.width === 0) return;
    const key = `${focusAgentId ?? ""}:${layout.nodes.length}:${layout.edges.length}:${size.width}x${size.height}`;
    if (fitKeyRef.current === key) return;
    fitKeyRef.current = key;
    fitToView();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [layout, size.width, size.height, focusAgentId]);

  // Draw. Runs on every data/view-state change; cheap at pilot scale (tens
  // to hundreds of nodes - PHASE3 §5.3), so no rAF batching is needed.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !layout || size.width === 0) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width = size.width * dpr;
    canvas.height = size.height * dpr;
    canvas.style.width = `${size.width}px`;
    canvas.style.height = `${size.height}px`;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, size.width, size.height);

    if (layout.nodes.length === 0) return;

    const lineColor = resolveVar("--line-2");
    const fgColor = resolveVar("--fg");
    const dimColor = resolveVar("--dim");
    const panelColor = resolveVar("--panel");
    const kindColors: Record<NodeKind, string> = {
      user: resolveVar(NODE_KIND_VAR.user),
      agent: resolveVar(NODE_KIND_VAR.agent),
      other: resolveVar(NODE_KIND_VAR.other),
    };
    // Resolved once per draw (not per node): a blocked node's fill overrides
    // its kind colour so STOPPED/FROZEN/KILLED agents read on the graph too.
    const lifecycleColors: Record<EntityLifecycleState, string | null> = {
      live: null,
      stopped: resolveVar(NODE_LIFECYCLE_VAR.stopped!),
      frozen: resolveVar(NODE_LIFECYCLE_VAR.frozen!),
      killed: resolveVar(NODE_LIFECYCLE_VAR.killed!),
    };

    const maxCount = Math.max(1, ...layout.nodes.map((n) => n.event_count));
    const highlightSet = hoveredId ? neighborsOf(hoveredId, layout.edges) : null;
    const isDim = (id: string): boolean => {
      if (neighborhood) return !neighborhood.has(id);
      if (hoveredId) return id !== hoveredId && !(highlightSet?.has(id) ?? false);
      return false;
    };

    ctx.save();
    ctx.translate(transform.x, transform.y);
    ctx.scale(transform.scale, transform.scale);

    const nodeById = new Map(layout.nodes.map((n) => [n.id, n]));

    for (const e of layout.edges) {
      const from = nodeById.get(e.from);
      const to = nodeById.get(e.to);
      if (!from || !to) continue;
      const dim = isDim(e.from) || isDim(e.to);
      const active = hoveredId !== null && (e.from === hoveredId || e.to === hoveredId);
      drawEdge(ctx, from, to, radiusFor(to.event_count, maxCount), {
        color: active ? fgColor : lineColor,
        alpha: dim ? 0.12 : active ? 0.9 : 0.55,
        width: (active ? 2 : 1.2) / transform.scale,
        headLen: 7 / transform.scale,
      });
    }

    for (const n of layout.nodes) {
      const r = radiusFor(n.event_count, maxCount);
      const dim = isDim(n.id);
      const isFocus = focusAgentId === n.id;
      const isHovered = hoveredId === n.id;

      ctx.globalAlpha = dim ? 0.25 : 1;
      ctx.beginPath();
      ctx.arc(n.x, n.y, r, 0, Math.PI * 2);
      const lifecycleFill = n.lifecycle && n.lifecycle !== "live" ? lifecycleColors[n.lifecycle] : null;
      ctx.fillStyle = lifecycleFill ?? kindColors[n.kind];
      ctx.fill();
      ctx.lineWidth = (isFocus || isHovered ? 2.5 : 1) / transform.scale;
      ctx.strokeStyle = isFocus || isHovered ? fgColor : panelColor;
      ctx.stroke();
      ctx.globalAlpha = 1;

      if (!dim && transform.scale > 0.35) {
        ctx.font = `${11 / transform.scale}px var(--font-m, monospace)`;
        ctx.fillStyle = isHovered || isFocus ? fgColor : dimColor;
        ctx.textAlign = "center";
        ctx.textBaseline = "top";
        ctx.fillText(shortAgentLabel(n.id), n.x, n.y + r + 3 / transform.scale);
      }
    }

    ctx.restore();
  }, [layout, size, transform, hoveredId, neighborhood, focusAgentId]);

  const nodeAt = useCallback(
    (clientX: number, clientY: number): PositionedNode | null => {
      const canvas = canvasRef.current;
      if (!canvas || !layout) return null;
      const rect = canvas.getBoundingClientRect();
      const gx = (clientX - rect.left - transform.x) / transform.scale;
      const gy = (clientY - rect.top - transform.y) / transform.scale;
      const maxCount = Math.max(1, ...layout.nodes.map((n) => n.event_count));
      let best: PositionedNode | null = null;
      let bestDist = Infinity;
      for (const n of layout.nodes) {
        const r = radiusFor(n.event_count, maxCount) + 3;
        const d = Math.hypot(n.x - gx, n.y - gy);
        if (d <= r && d < bestDist) {
          best = n;
          bestDist = d;
        }
      }
      return best;
    },
    [layout, transform],
  );

  const onPointerDown = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    dragRef.current = { startX: e.clientX, startY: e.clientY, origin: transform, moved: false };
    e.currentTarget.setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const drag = dragRef.current;
    if (drag) {
      const dx = e.clientX - drag.startX;
      const dy = e.clientY - drag.startY;
      if (Math.abs(dx) > 2 || Math.abs(dy) > 2) drag.moved = true;
      setTransform({ scale: drag.origin.scale, x: drag.origin.x + dx, y: drag.origin.y + dy });
      return;
    }
    const hit = nodeAt(e.clientX, e.clientY);
    setHoveredId(hit ? hit.id : null);
  };

  const onPointerUp = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const wasDrag = dragRef.current?.moved ?? false;
    dragRef.current = null;
    if (!wasDrag) {
      const hit = nodeAt(e.clientX, e.clientY);
      if (hit) onOpenAgent(hit.id);
    }
  };

  const onPointerLeave = () => {
    dragRef.current = null;
    setHoveredId(null);
  };

  const zoomBy = useCallback((factor: number) => {
    setTransform((prev) => {
      const nextScale = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, prev.scale * factor));
      const cx = size.width / 2;
      const cy = size.height / 2;
      const gx = (cx - prev.x) / prev.scale;
      const gy = (cy - prev.y) / prev.scale;
      return { scale: nextScale, x: cx - gx * nextScale, y: cy - gy * nextScale };
    });
  }, [size.width, size.height]);

  // Wheel-to-zoom needs a native, non-passive listener: React's `onWheel`
  // prop attaches a passive listener, so `preventDefault()` inside it would
  // warn and fail to stop the page from scrolling under the cursor.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const handler = (e: WheelEvent) => {
      e.preventDefault();
      const rect = canvas.getBoundingClientRect();
      const px = e.clientX - rect.left;
      const py = e.clientY - rect.top;
      const factor = Math.exp(-e.deltaY * 0.001);
      setTransform((prev) => {
        const nextScale = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, prev.scale * factor));
        const gx = (px - prev.x) / prev.scale;
        const gy = (py - prev.y) / prev.scale;
        return { scale: nextScale, x: px - gx * nextScale, y: py - gy * nextScale };
      });
    };
    canvas.addEventListener("wheel", handler, { passive: false });
    return () => canvas.removeEventListener("wheel", handler);
  }, []);

  const hoveredNode = hoveredId ? (layout?.nodes.find((n) => n.id === hoveredId) ?? null) : null;

  return (
    <div className={`flex flex-col gap-2${fill ? " flex-1 min-h-0" : ""}`}>
      {!compact && (
        <div className="flex flex-wrap items-center gap-3">
          <Legend />
          <div className="flex-1" />
          <span className="mono text-[10.5px]" style={{ color: "var(--faint)" }}>
            {layout ? `${nodeSummary(layout.nodes)} · ${layout.edges.length} links` : "loading..."}
            {asOfMs !== null ? ` · updated ${new Date(asOfMs).toLocaleTimeString()}` : ""}
          </span>
          <button type="button" className="icon-btn" style={{ width: "auto", padding: "0 8px", fontSize: 11 }} onClick={() => void load()}>
            Refresh
          </button>
          <button type="button" className="icon-btn" style={{ width: "auto", padding: "0 8px", fontSize: 11 }} onClick={fitToView}>
            Reset view
          </button>
          <button type="button" className="icon-btn" style={{ fontSize: 13 }} aria-label="Zoom out" onClick={() => zoomBy(1 / ZOOM_STEP)}>
            &minus;
          </button>
          <button type="button" className="icon-btn" style={{ fontSize: 13 }} aria-label="Zoom in" onClick={() => zoomBy(ZOOM_STEP)}>
            +
          </button>
        </div>
      )}

      <div
        ref={containerRef}
        className={`panel relative${fill ? " flex-1 min-h-0" : ""}`}
        style={{ background: "var(--panel)", height: fill ? "100%" : height, overflow: "hidden" }}
      >
        {layout && layout.nodes.length === 0 ? (
          <div className="absolute inset-0 flex items-center justify-center px-6">
            <span className="mono text-[12px]" style={{ color: "var(--faint)" }}>
              no delegation activity yet.
            </span>
          </div>
        ) : (
          <canvas
            ref={canvasRef}
            role="img"
            aria-label={
              layout
                ? `Delegation graph: ${nodeSummary(layout.nodes)}, ${layout.edges.length} delegation links. Drag to pan, scroll to zoom, click a node to open its Agent 360 card.`
                : "Delegation graph loading"
            }
            style={{ display: "block", cursor: dragRef.current ? "grabbing" : hoveredId ? "pointer" : "grab" }}
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            onPointerLeave={onPointerLeave}
          />
        )}

        {hoveredNode && (
          <div
            className="panel absolute px-2.5 py-1.5 flex flex-col gap-0.5 pointer-events-none"
            style={{ left: 10, bottom: 10, background: "var(--panel-2)", maxWidth: "70%" }}
          >
            <span className="mono truncate text-[11px]" style={{ color: "var(--fg)" }}>
              {hoveredNode.id}
            </span>
            <span className="mono text-[10.5px]" style={{ color: "var(--faint)" }}>
              {NODE_KIND_LABEL[hoveredNode.kind]} &middot; {hoveredNode.event_count} events
            </span>
          </div>
        )}
      </div>
    </div>
  );
}
