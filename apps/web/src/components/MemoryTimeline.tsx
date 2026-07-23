import { hasBackend, subscribeBackend } from "../lib/transport";
import { useEffect, useState } from "react";
import { fetchRecentEvents } from "../lib/recentEvents";
import { formatTimestamp } from "../lib/format";
import type { UiEvent } from "../types";
import { SeverityBadge } from "./SeverityBadge";

/** Mirrors `QualityDriftStream.tsx`/`DecisionStream.tsx`'s identical
 * `FETCH_LIMIT`/`DISPLAY_LIMIT` role. */
const FETCH_LIMIT = 500;
const DISPLAY_LIMIT = 50;

/** Tauri event name the Rust live feeder (`src-tauri/src/live.rs`) emits on -
 * the SAME event `BusExplorer.tsx`/`DecisionStream.tsx`/`QualityDriftStream.tsx`
 * listen for; payload is one `UiEvent`. */
const LIVE_EVENT = "bus:event";

const ENGRAM_SOURCE = "engram";

const COLUMNS = "96px 74px 150px 190px 1fr";

function isEngramEvent(e: UiEvent): boolean {
  return e.source === ENGRAM_SOURCE;
}

/** Best-effort read of one string field out of an event's untyped `data`
 * payload - never assumes the shape, never throws on a missing/malformed
 * field - mirrors `DecisionStream.tsx`'s identical `dataString` helper. */
function dataString(data: unknown, key: string): string | null {
  if (data && typeof data === "object" && key in (data as Record<string, unknown>)) {
    const value = (data as Record<string, unknown>)[key];
    if (typeof value === "string") return value;
  }
  return null;
}

/**
 * Timeline (docs/PHASE4.md W2 Memory position 4): the live `engram.*` bus
 * events (`memory_written`, `contradiction_found`, `reflection_run`, ...) -
 * a filtered view over the SAME event bus the Bus Explorer tails
 * (`source == "engram"`), mirroring `QualityDriftStream.tsx`/
 * `DecisionStream.tsx`'s exact shape (reuses `fetchRecentEvents` for the
 * initial batch and the `bus:event` Tauri listener for live updates) -
 * deliberately NOT a new backend read.
 */
export function MemoryTimeline({ onOpenAgent }: { onOpenAgent: (agentId: string) => void }) {
  const [events, setEvents] = useState<UiEvent[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    void fetchRecentEvents(FETCH_LIMIT).then((res) => {
      if (cancelled) return;
      setEvents(res.events.filter(isEngramEvent));
      setLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!hasBackend()) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    subscribeBackend<UiEvent>(LIVE_EVENT, (payload) => {
      if (!isEngramEvent(payload)) return;
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

  if (loading) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        loading timeline...
      </div>
    );
  }

  const rows = events.slice(0, DISPLAY_LIMIT);

  if (rows.length === 0) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        no engram events yet.
      </div>
    );
  }

  return (
    <>
      <div
        className="grid gap-3 px-5 py-2"
        style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line)" }}
      >
        {["time", "severity", "type", "agent", "detail"].map((label) => (
          <span
            key={label}
            className="mono"
            style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
          >
            {label}
          </span>
        ))}
      </div>
      {rows.map((e) => {
        const memoryId = dataString(e.data, "memory_id");
        const topic = dataString(e.data, "topic");
        const conflicting = dataString(e.data, "conflicting_memory_id");
        const detail = [
          memoryId ? `memory ${memoryId}` : null,
          topic,
          conflicting ? `conflicts with ${conflicting}` : null,
        ]
          .filter((v): v is string => v !== null)
          .join(" · ");
        return (
          <div key={e.id} className="grid items-center gap-3 px-5 py-2 bus-row" style={{ gridTemplateColumns: COLUMNS }}>
            <span className="mono tabular text-[11px]" style={{ color: "var(--faint)" }}>
              {formatTimestamp(e.ts)}
            </span>
            <SeverityBadge severity={e.severity} />
            <span className="mono truncate text-[12px]" style={{ color: "var(--fg)" }} title={e.type}>
              {e.type}
            </span>
            <button
              type="button"
              className="mono tabular truncate text-[11.5px] text-left"
              title={`Open Agent 360 for ${e.agent_id}`}
              style={{ color: "var(--dim)", background: "none", border: "none", padding: 0, cursor: "pointer" }}
              onClick={() => onOpenAgent(e.agent_id)}
            >
              {e.agent_id}
            </button>
            <span className="mono truncate text-[11.5px]" style={{ color: "var(--faint)" }} title={detail || undefined}>
              {detail || "-"}
            </span>
          </div>
        );
      })}
    </>
  );
}
