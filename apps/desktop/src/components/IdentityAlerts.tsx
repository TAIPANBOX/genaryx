import { useMemo, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { DETECTOR_IDS, type IdryxAlert } from "../identityTypes";
import { SeverityBadge } from "./SeverityBadge";

const COLUMNS = "90px 190px 1fr 150px 1fr";

/** Task-scoped severity filter set (docs/PHASE3.md W2: "severity-filter
 * chips (critical/high/medium/low)"). Idryx can also emit `info`/`none`
 * (see `IdryxAlert.severity`'s doc); those rows still render in the table
 * (via `SeverityBadge`'s own tolerant fallback), they are just not one of
 * the four toggle chips here. */
const FILTER_SEVERITIES = ["critical", "high", "medium", "low"] as const;

const SEVERITY_TONE: Record<(typeof FILTER_SEVERITIES)[number], string> = {
  critical: "var(--sev-critical)",
  high: "var(--sev-high)",
  medium: "var(--sev-medium)",
  low: "var(--sev-low)",
};

/** Severity-filter chips: empty selection means no filter (show every
 * severity); clicking a chip narrows the list to the union of selected
 * severities - same convention `IdentityList.tsx`'s type filter uses. */
function SeverityFilterChips({
  active,
  onToggle,
}: {
  active: ReadonlySet<string>;
  onToggle: (s: string) => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label="Filter by severity">
      {FILTER_SEVERITIES.map((s) => {
        const on = active.has(s);
        return (
          <button
            key={s}
            type="button"
            className="badge"
            style={{ ...cssVar("tone", SEVERITY_TONE[s]), cursor: "pointer", opacity: on ? 1 : 0.4 }}
            aria-pressed={on}
            onClick={() => onToggle(s)}
          >
            {s}
          </button>
        );
      })}
    </div>
  );
}

function formatAlertTime(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

/**
 * The alert stream (docs/PHASE3.md W2): the 21 detectors from
 * `GET /api/alerts` (or the freshest `identity_rescan` result once one has
 * run), with severity-filter chips AND a detector-id filter, plus the
 * Rescan action. Read-only: there is no decision/mutation here at all,
 * unlike `ApprovalsInbox`'s Grant/Deny - Rescan only ever recomputes and
 * replaces this list.
 */
export function IdentityAlerts({
  alerts,
  onRescan,
  rescanning,
  rescanAvailable,
}: {
  alerts: IdryxAlert[];
  onRescan: () => void;
  rescanning: boolean;
  rescanAvailable: boolean;
}) {
  const [activeSeverities, setActiveSeverities] = useState<ReadonlySet<string>>(new Set());
  const [detector, setDetector] = useState<string>("");

  const toggleSeverity = (s: string) => {
    setActiveSeverities((prev) => {
      const next = new Set(prev);
      if (next.has(s)) next.delete(s);
      else next.add(s);
      return next;
    });
  };

  const rows = useMemo(
    () =>
      alerts.filter((a) => {
        if (activeSeverities.size > 0 && !activeSeverities.has(a.severity)) return false;
        if (detector !== "" && a.detector !== detector) return false;
        return true;
      }),
    [alerts, activeSeverities, detector],
  );

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-3">
        <SeverityFilterChips active={activeSeverities} onToggle={toggleSeverity} />
        <select
          className="mono"
          aria-label="Filter by detector"
          value={detector}
          onChange={(e) => setDetector(e.target.value)}
          style={{
            background: "var(--panel-2)",
            color: "var(--dim)",
            border: "1px solid var(--line-2)",
            borderRadius: 8,
            fontSize: 11,
            padding: "4px 8px",
          }}
        >
          <option value="">(all detectors)</option>
          {DETECTOR_IDS.map((d) => (
            <option key={d} value={d}>
              {d}
            </option>
          ))}
        </select>
        <div className="flex-1" />
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 10px", fontSize: 11, whiteSpace: "nowrap" }}
          onClick={onRescan}
          disabled={!rescanAvailable || rescanning}
          title={
            rescanAvailable
              ? "Recompute the 21 detectors now (idryx detect)"
              : "Rescan needs the idryx binary at ~/.taipan/bin/idryx, which was not found"
          }
        >
          {rescanning ? "Rescanning..." : "Rescan"}
        </button>
      </div>

      {alerts.length === 0 ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no alerts in this snapshot.
        </div>
      ) : rows.length === 0 ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no alerts match the selected filters.
        </div>
      ) : (
        <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
          <div
            className="grid gap-3 px-4 py-2"
            style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line-2)", background: "var(--panel-2)" }}
          >
            {["severity", "detector", "identity", "time", "summary"].map((label) => (
              <span
                key={label}
                className="mono"
                style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
              >
                {label}
              </span>
            ))}
          </div>
          {rows.map((a, idx) => (
            <div
              key={`${a.detector}-${a.identity}-${a.time}-${idx}`}
              className="grid items-center gap-3 px-4 py-2 bus-row"
              style={{ gridTemplateColumns: COLUMNS }}
            >
              <SeverityBadge severity={a.severity} />
              <span className="mono truncate text-[11.5px]" title={a.detector} style={{ color: "var(--fg)" }}>
                {a.detector}
              </span>
              <span className="mono truncate text-[11.5px]" title={a.identity} style={{ color: "var(--dim)" }}>
                {a.identity}
              </span>
              <span className="mono tabular text-[11px]" style={{ color: "var(--faint)" }}>
                {formatAlertTime(a.time)}
              </span>
              <span className="truncate text-[11.5px]" title={a.summary} style={{ color: "var(--dim)" }}>
                {a.summary}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
