import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { fetchRecentEvents } from "../lib/recentEvents";
import { formatTimestamp } from "../lib/format";
import type { UiEvent } from "../types";
import { SeverityBadge } from "./SeverityBadge";

/** Comfortably above what a single session accumulates; also the cap
 * applied to the live feed so the list never grows unbounded - mirrors
 * `BusExplorer.tsx`'s identical `FETCH_LIMIT` role. */
const FETCH_LIMIT = 500;

/** How many wardryx rows the stream actually renders - this is a compact
 * panel section, not the full Bus Explorer, so it shows only the most
 * recent slice of whatever `FETCH_LIMIT` currently holds. */
const DISPLAY_LIMIT = 50;

/** Tauri event name the Rust live feeder (`src-tauri/src/live.rs`) emits
 * on - the SAME event `BusExplorer.tsx` listens for; payload is one
 * `UiEvent`. */
const LIVE_EVENT = "bus:event";

const WARDRYX_SOURCE = "wardryx";

const COLUMNS = "96px 74px 168px 190px 1fr 200px";

function isWardryx(e: UiEvent): boolean {
  return e.source === WARDRYX_SOURCE;
}

/** Best-effort read of one string field out of an event's untyped `data`
 * payload - never assumes the shape, never throws on a missing/malformed
 * field (the core keeps `data` deliberately untyped end to end). */
function dataString(data: unknown, key: string): string | null {
  if (data && typeof data === "object" && key in (data as Record<string, unknown>)) {
    const value = (data as Record<string, unknown>)[key];
    if (typeof value === "string") return value;
  }
  return null;
}

/** Same as `dataString`, for a string-array field (`tool_names`). */
function dataStringArray(data: unknown, key: string): string[] | null {
  if (data && typeof data === "object" && key in (data as Record<string, unknown>)) {
    const value = (data as Record<string, unknown>)[key];
    if (Array.isArray(value)) {
      const strings = value.filter((v): v is string => typeof v === "string");
      if (strings.length === value.length) return strings;
    }
  }
  return null;
}

/**
 * A live, filtered view over the SAME event bus the Bus Explorer tails
 * (`source == "wardryx"`: `policy_allow/deny`, `approval_*`,
 * `policy_updated` - docs/PHASE2.md's Wave-2 data contract). Reuses
 * `fetchRecentEvents` for the initial batch and the `bus:event` Tauri
 * listener for live updates, exactly like `BusExplorer.tsx` - deliberately
 * NOT a new poll or REST read; this component only ever filters what the
 * existing pipeline already delivers.
 *
 * The row's severity badge (allow=info / deny=high / hold=medium, per
 * PHASE2.md) comes straight from the event's own `severity` field via the
 * shared `SeverityBadge` - the bus events themselves already carry the
 * correct severity per source/type, so there is no separate mapping to
 * maintain here.
 */
export function DecisionStream() {
  const [events, setEvents] = useState<UiEvent[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    void fetchRecentEvents(FETCH_LIMIT).then((res) => {
      if (cancelled) return;
      setEvents(res.events.filter(isWardryx));
      setLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!isTauri()) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    listen<UiEvent>(LIVE_EVENT, (event) => {
      if (!isWardryx(event.payload)) return;
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

  if (loading) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        loading decision stream...
      </div>
    );
  }

  const rows = events.slice(0, DISPLAY_LIMIT);

  if (rows.length === 0) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        no wardryx decisions yet.
      </div>
    );
  }

  return (
    <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
      <div
        className="grid gap-3 px-4 py-2"
        style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line-2)", background: "var(--panel-2)" }}
      >
        {["time", "severity", "type", "agent", "reason", "tools"].map((label) => (
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
        const reason = dataString(e.data, "reason");
        const toolNames = dataStringArray(e.data, "tool_names");
        return (
          <div key={e.id} className="grid items-center gap-3 px-4 py-2 bus-row" style={{ gridTemplateColumns: COLUMNS }}>
            <span className="mono tabular text-[11px]" style={{ color: "var(--faint)" }}>
              {formatTimestamp(e.ts)}
            </span>
            <SeverityBadge severity={e.severity} />
            <span className="mono truncate text-[12px]" style={{ color: "var(--fg)" }} title={e.type}>
              {e.type}
            </span>
            <span
              className="mono tabular truncate text-[11.5px]"
              style={{ color: "var(--dim)" }}
              title={e.agent_id}
            >
              {e.agent_id}
            </span>
            <span
              className="truncate text-[11.5px]"
              style={{ color: "var(--dim)" }}
              title={reason ?? undefined}
            >
              {reason ?? "-"}
            </span>
            <span
              className="mono truncate text-[11px]"
              style={{ color: "var(--faint)" }}
              title={toolNames ? toolNames.join(", ") : undefined}
            >
              {toolNames && toolNames.length > 0 ? toolNames.join(", ") : "-"}
            </span>
          </div>
        );
      })}
    </div>
  );
}
