import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useRef, useState } from "react";
import { fetchRecentEvents, type EventsSource } from "../lib/recentEvents";
import type { UiEvent } from "../types";
import { EventRow } from "./EventRow";
import { Header } from "./Header";

/** Comfortably above the ~40-event mock timeline; the real bus will hold far
 * more once wired (see the FOLLOW-UP WIRING POINT in src-tauri/src/events.rs). */
const FETCH_LIMIT = 500;

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
