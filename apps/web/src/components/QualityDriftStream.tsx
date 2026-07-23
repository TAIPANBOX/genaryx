import { hasBackend, subscribeBackend } from "../lib/transport";
import { useEffect, useState } from "react";
import { fetchRecentEvents } from "../lib/recentEvents";
import { formatTimestamp } from "../lib/format";
import type { UiEvent } from "../types";
import { SeverityBadge } from "./SeverityBadge";

/** Mirrors `DecisionStream.tsx`'s identical `FETCH_LIMIT`/`DISPLAY_LIMIT`
 * role. */
const FETCH_LIMIT = 500;
const DISPLAY_LIMIT = 50;

/** Tauri event name the Rust live feeder (`src-tauri/src/live.rs`) emits on -
 * the SAME event `BusExplorer.tsx`/`DecisionStream.tsx` listen for; payload
 * is one `UiEvent`. */
const LIVE_EVENT = "bus:event";

const VERDRYX_SOURCE = "verdryx";
const DRIFT_TYPE = "quality_drift";

const COLUMNS = "96px 74px 190px 90px 90px 1fr";

function isQualityDrift(e: UiEvent): boolean {
  return e.source === VERDRYX_SOURCE && e.type === DRIFT_TYPE;
}

/** Best-effort read of one field out of an event's untyped `data` payload -
 * never assumes the shape, never throws on a missing/malformed field (the
 * core keeps `data` deliberately untyped end to end) - mirrors
 * `DecisionStream.tsx`'s identical `dataString` helper. */
function dataNumber(data: unknown, key: string): number | null {
  if (data && typeof data === "object" && key in (data as Record<string, unknown>)) {
    const value = (data as Record<string, unknown>)[key];
    if (typeof value === "number") return value;
  }
  return null;
}

function dataString(data: unknown, key: string): string | null {
  if (data && typeof data === "object" && key in (data as Record<string, unknown>)) {
    const value = (data as Record<string, unknown>)[key];
    if (typeof value === "string") return value;
  }
  return null;
}

/**
 * Drift alerts (docs/PHASE4.md W1 position 4): a live, filtered view over
 * the SAME event bus the Bus Explorer tails (`source == "verdryx" && type ==
 * "quality_drift"`) - mirrors `DecisionStream.tsx`'s exact shape (reuses
 * `fetchRecentEvents` for the initial batch and the `bus:event` Tauri
 * listener for live updates; deliberately NOT a new poll or backend read).
 * Verdryx's DB has no drift table of its own - its live drift signal IS the
 * `quality_drift` bus event (`crates/connectors/src/verdryx.rs`'s module
 * doc), so this component is the whole of the Quality panel's drift-alerts
 * feature, not a supplement to a backend read.
 */
export function QualityDriftStream({ onOpenAgent }: { onOpenAgent: (agentId: string) => void }) {
  const [events, setEvents] = useState<UiEvent[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    void fetchRecentEvents(FETCH_LIMIT).then((res) => {
      if (cancelled) return;
      setEvents(res.events.filter(isQualityDrift));
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
      if (!isQualityDrift(payload)) return;
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
        loading drift alerts...
      </div>
    );
  }

  const rows = events.slice(0, DISPLAY_LIMIT);

  if (rows.length === 0) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        no quality drift alerts yet.
      </div>
    );
  }

  return (
    <>
      <div
        className="grid gap-3 px-5 py-2"
        style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line)" }}
      >
        {["time", "severity", "agent", "delta", "verdict", "baseline"].map((label) => (
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
        const delta = dataNumber(e.data, "delta");
        const verdict = dataString(e.data, "verdict");
        const baselineId = dataString(e.data, "baseline_id");
        const meanScore = dataNumber(e.data, "mean_score");
        return (
          <div key={e.id} className="grid items-center gap-3 px-5 py-2 bus-row" style={{ gridTemplateColumns: COLUMNS }}>
            <span className="mono tabular text-[11px]" style={{ color: "var(--faint)" }}>
              {formatTimestamp(e.ts)}
            </span>
            <SeverityBadge severity={e.severity} />
            <button
              type="button"
              className="mono tabular truncate text-[11.5px] text-left"
              title={`Open Agent 360 for ${e.agent_id}`}
              style={{ color: "var(--dim)", background: "none", border: "none", padding: 0, cursor: "pointer" }}
              onClick={() => onOpenAgent(e.agent_id)}
            >
              {e.agent_id}
            </button>
            <span
              className="mono tabular text-[12px]"
              style={{ color: delta !== null && delta < 0 ? "var(--sev-high)" : "var(--fg)" }}
            >
              {delta !== null ? delta.toFixed(3) : "-"}
            </span>
            <span
              className="mono truncate text-[11.5px]"
              style={{ color: verdict === "regressed" ? "var(--sev-high)" : "var(--sev-low)" }}
            >
              {verdict ?? "-"}
            </span>
            <span className="mono truncate text-[11.5px]" style={{ color: "var(--faint)" }} title={baselineId ?? undefined}>
              {baselineId ?? "-"}
              {meanScore !== null ? ` · mean ${meanScore.toFixed(3)}` : ""}
            </span>
          </div>
        );
      })}
    </>
  );
}
