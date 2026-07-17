import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useRef, useState } from "react";
import { fetchRecentEvents, type EventsSource } from "../lib/recentEvents";
import type { UiEvent } from "../types";
import { EventRow } from "./EventRow";
import { Header } from "./Header";

/** Comfortably above the ~40-event mock timeline; also the cap applied to
 * the live feed below so the list never grows unbounded. */
const FETCH_LIMIT = 500;

/** Tauri event name the Rust live feeder (`src-tauri/src/live.rs`) emits on;
 * payload is one `UiEvent`, same shape `recent_events` returns. */
const LIVE_EVENT = "bus:event";

const COLUMNS = "84px 108px 190px 1fr 108px 24px";

/**
 * The Bus Explorer: a dense, virtualized live-event list. Rows are windowed
 * with `@tanstack/react-virtual` (only what is on screen, plus overscan,
 * ever mounts) so the list stays smooth however many events the bus has
 * carried. This component owns the whole page: the app header (name + live
 * counter + theme toggle), the column header, and the scrolling list.
 */
export function BusExplorer() {
  const [theme, setTheme] = useState<"dark" | "light">("dark");
  const [events, setEvents] = useState<UiEvent[]>([]);
  const [source, setSource] = useState<EventsSource>("mock");
  const [loading, setLoading] = useState(true);
  const [expanded, setExpanded] = useState<ReadonlySet<number>>(new Set());
  const parentRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

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

  // Live path: the Rust feeder (src-tauri/src/live.rs) emits one `UiEvent`
  // every ~2s once it has appended and ingested a new line. Prepend each as
  // it arrives, capped at FETCH_LIMIT so the list never grows unbounded.
  // Skipped entirely outside a Tauri runtime (plain `vite build`/preview):
  // `listen()` calls into the IPC bridge unconditionally and would reject
  // with no `window.__TAURI_INTERNALS__` to answer it.
  useEffect(() => {
    if (!isTauri()) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    listen<UiEvent>(LIVE_EVENT, (event) => {
      setEvents((prev) => [event.payload, ...prev].slice(0, FETCH_LIMIT));
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
        console.error(`listen(${LIVE_EVENT}) failed:`, err);
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const virtualizer = useVirtualizer({
    count: events.length,
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
    <div className="app">
      <Header
        count={events.length}
        source={source}
        theme={theme}
        onToggleTheme={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
      />

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
              const event = events[virtualRow.index];
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
                  <EventRow event={event} expanded={expanded.has(event.id)} onToggle={() => toggle(event.id)} />
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
