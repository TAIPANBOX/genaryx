import { hasBackend, subscribeBackend } from "../lib/transport";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useMemo, useRef, useState } from "react";
import { fetchBusMode, type BusMode } from "../lib/busStatus";
import { fetchRecentEvents, type EventsSource } from "../lib/recentEvents";
import type { UiEvent } from "../types";
import { EventRow } from "./EventRow";
import { BusStatusBar } from "./Header";
import { usePopover } from "../lib/popover";
import { unitForTeam } from "../lib/views";
import { AgentDetailCard } from "./AgentDetailCard";
import { PinnedEventOverlay } from "./PinnedEventOverlay";
import { RefusedLines } from "./RefusedLines";
import { SortBar, type SortDir } from "./SortBar";

const SEV_RANK: Record<string, number> = { critical: 5, high: 4, medium: 3, low: 2, info: 1 };

const BUS_SORTS = [
  { key: "time", label: "time" },
  { key: "severity", label: "severity" },
  { key: "source", label: "source" },
  { key: "agent", label: "agent" },
  { key: "unit", label: "unit" },
  { key: "user", label: "user" },
  { key: "type", label: "type" },
];

/** Team out of an `agent://org/team/name` id (the "unit" sort maps it through
 * `unitForTeam` so fraud + kyc-aml group under one business unit), and the
 * human out of the event's delegation chain, so the feed can be grouped by
 * either. */
function teamOf(id: string): string {
  const m = /^agent:\/\/[^/]+\/([^/]+)\//.exec(id);
  return m ? m[1] : "";
}
function userOf(e: UiEvent): string {
  const u = e.on_behalf_of?.[0] ?? "";
  const m = /\/([^/]+)$/.exec(u);
  return m ? m[1] : "";
}

function sortEvents(evts: UiEvent[], key: string, dir: SortDir): UiEvent[] {
  const sign = dir === "desc" ? -1 : 1;
  const out = [...evts];
  out.sort((a, b) => {
    let c = 0;
    if (key === "time") c = a.id - b.id;
    else if (key === "severity") c = (SEV_RANK[a.severity ?? ""] ?? 0) - (SEV_RANK[b.severity ?? ""] ?? 0);
    else if (key === "source") c = a.source.localeCompare(b.source);
    else if (key === "agent") c = a.agent_id.localeCompare(b.agent_id);
    else if (key === "unit") c = unitForTeam(teamOf(a.agent_id)).localeCompare(unitForTeam(teamOf(b.agent_id)));
    else if (key === "user") c = userOf(a).localeCompare(userOf(b));
    else if (key === "type") c = a.type.localeCompare(b.type);
    // Stable tiebreak by time so grouped rows still read newest-first within a group.
    return c * sign || (b.id - a.id);
  });
  return out;
}

/** Comfortably above the ~40-event mock timeline; also the cap applied to
 * the live feed below so the list never grows unbounded. */
const FETCH_LIMIT = 500;

/** Bus event name the live feed (`crates/api/src/bus/feed.rs`) emits on, over
 * SSE via `subscribeBackend` (`lib/transport.ts`); payload is one `UiEvent`,
 * same shape `recent_events` returns. */
const LIVE_EVENT = "bus:event";

const COLUMNS = "84px 108px 190px 1fr 108px 24px";

/**
 * The Bus Explorer: a dense, virtualized live-event list. Rows are windowed
 * with `@tanstack/react-virtual` (only what is on screen, plus overscan,
 * ever mounts) so the list stays smooth however many events the bus has
 * carried. One of three views under `AppShell` (alongside Overview and
 * Money); owns its own status strip (`BusStatusBar`), column header, and
 * the scrolling list, but not the app-wide brand/nav/theme chrome, which
 * `AppShell`/`AppHeader` render once above whichever view is active.
 */
export function BusExplorer() {
  const [events, setEvents] = useState<UiEvent[]>([]);
  const [source, setSource] = useState<EventsSource>("mock");
  const [mode, setMode] = useState<BusMode | null>(null);
  const [loading, setLoading] = useState(true);
  const [expanded, setExpanded] = useState<ReadonlySet<number>>(new Set());
  const [pinned, setPinned] = useState<{ event: UiEvent; rect: DOMRect } | null>(null);
  const [sort, setSort] = useState<{ key: string; dir: SortDir }>({ key: "time", dir: "desc" });
  const displayed = useMemo(() => sortEvents(events, sort.key, sort.dir), [events, sort]);
  const parentRef = useRef<HTMLDivElement>(null);
  const { open } = usePopover();

  useEffect(() => {
    let cancelled = false;
    void fetchRecentEvents(FETCH_LIMIT).then((res) => {
      if (cancelled) return;
      setEvents(res.events);
      setSource(res.source);
      setLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // Which bus these rows came from. Decided once at startup in the core and
  // never changed after, so this is fetched once and not polled.
  useEffect(() => {
    let cancelled = false;
    void fetchBusMode().then((m) => {
      if (!cancelled) setMode(m);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // Live path: the Rust feeder (crates/api/src/bus/feed.rs) emits one
  // `UiEvent` every ~2s once it has appended and ingested a new line.
  // Prepend each as it arrives, capped at FETCH_LIMIT so the list never
  // grows unbounded. Skipped entirely with no backend configured (plain
  // `vite build`/preview): the `hasBackend()` guard below returns before
  // ever calling `subscribeBackend`.
  useEffect(() => {
    if (!hasBackend()) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    subscribeBackend<UiEvent>(LIVE_EVENT, (payload) => {
      setEvents((prev) => [payload, ...prev].slice(0, FETCH_LIMIT));
    })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch((err: unknown) => {
        // eslint-disable-next-line no-console
        console.error(`subscribe(${LIVE_EVENT}) failed:`, err);
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const virtualizer = useVirtualizer({
    count: displayed.length,
    getScrollElement: () => parentRef.current,
    // Collapsed-row estimate; `measureElement` below corrects this per-row
    // (including growing/shrinking on expand/collapse) via ResizeObserver.
    estimateSize: () => 41,
    overscan: 12,
  });

  const toggle = (id: number) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };

  return (
    <div className="flex-1 min-h-0 flex flex-col">
      <BusStatusBar count={events.length} source={source} mode={mode} />
      {/* What the bus REFUSED, directly under what it accepted. A refused line
          is absent from every count below it, and the agents it was about look
          idle rather than broken, so this belongs beside the feed and not on a
          page somebody has to know to open. */}
      <RefusedLines />

      <div className="px-4 py-2 shrink-0" style={{ borderBottom: "1px solid var(--line-2)", background: "var(--panel-2)" }}>
        <SortBar options={BUS_SORTS} active={sort.key} dir={sort.dir} onChange={(key, dir) => setSort({ key, dir })} />
      </div>

      <div
        className="grid gap-3 px-4 py-2 shrink-0"
        style={{
          gridTemplateColumns: COLUMNS,
          borderBottom: "1px solid var(--line-2)",
          background: "var(--panel-2)",
        }}
      >
        {["severity", "source", "type", "agent", "time", ""].map((label) => (
          <span
            key={label || "spacer"}
            className="mono"
            style={{ fontSize: 10, letterSpacing: "0.1em", textTransform: "uppercase", color: "var(--faint)" }}
          >
            {label}
          </span>
        ))}
      </div>

      <div ref={parentRef} className="flex-1 min-h-0 overflow-y-auto thin-scroll">
        {loading ? (
          <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
            loading events...
          </div>
        ) : events.length === 0 ? (
          <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
            no events yet.
          </div>
        ) : (
          <div style={{ height: virtualizer.getTotalSize(), position: "relative", width: "100%" }}>
            {virtualizer.getVirtualItems().map((virtualRow) => {
              const event = displayed[virtualRow.index];
              return (
                <div
                  key={event.id}
                  data-index={virtualRow.index}
                  ref={virtualizer.measureElement}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    transform: `translateY(${virtualRow.start}px)`,
                  }}
                >
                  <EventRow
                    event={event}
                    expanded={expanded.has(event.id)}
                    onToggle={() => toggle(event.id)}
                    onSelect={(rect) => setPinned({ event, rect })}
                    selected={pinned?.event.id === event.id}
                  />
                </div>
              );
            })}
          </div>
        )}
      </div>

      {pinned && (
        <PinnedEventOverlay
          event={pinned.event}
          rect={pinned.rect}
          onClose={() => setPinned(null)}
          onOpenAgent={(id, rect) => {
            setPinned(null);
            open(<AgentDetailCard agentId={id} />, { anchor: rect });
          }}
        />
      )}
    </div>
  );
}
