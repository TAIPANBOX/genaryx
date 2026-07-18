import type { VerdryxRunSummary } from "../qualityTypes";
import { formatTimestamp, formatUsd } from "../lib/format";

const COLUMNS = "1fr 150px 150px 70px 100px 100px";

/**
 * Eval-runs history (docs/PHASE4.md W1 position 1): model, started/finished,
 * and the per-run summary (case count, mean score, total cost), newest
 * first. Clicking a row selects it for `QualityRunDetail`.
 */
export function QualityRunsList({
  runs,
  selectedRunId,
  onSelect,
}: {
  runs: VerdryxRunSummary[];
  selectedRunId: string | null;
  onSelect: (runId: string) => void;
}) {
  if (runs.length === 0) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        no eval runs in verdryx.db yet.
      </div>
    );
  }

  return (
    <div style={{ overflowX: "auto" }}>
      <div
        className="grid gap-3 px-5 py-2"
        style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line)" }}
      >
        {["run", "started", "finished", "cases", "mean score", "total cost"].map((label) => (
          <span
            key={label}
            className="mono"
            style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
          >
            {label}
          </span>
        ))}
      </div>
      {runs.map((s) => {
        const active = s.run.id === selectedRunId;
        return (
          <button
            key={s.run.id}
            type="button"
            onClick={() => onSelect(s.run.id)}
            className="grid items-center gap-3 px-5 py-2.5 bus-row w-full text-left"
            style={{
              gridTemplateColumns: COLUMNS,
              background: active ? "color-mix(in srgb, var(--accent) 8%, transparent)" : "transparent",
              border: "none",
              cursor: "pointer",
            }}
          >
            <span className="mono truncate text-[12px]" title={`${s.run.model} - ${s.run.id}`} style={{ color: "var(--fg)" }}>
              {s.run.model} <span style={{ color: "var(--faint)" }}>&middot; {s.run.id}</span>
            </span>
            <span className="mono tabular text-[11px]" style={{ color: "var(--dim)" }}>
              {formatTimestamp(s.run.started_at)}
            </span>
            <span className="mono tabular text-[11px]" style={{ color: "var(--dim)" }}>
              {s.run.finished_at ? formatTimestamp(s.run.finished_at) : "in progress"}
            </span>
            <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }}>
              {s.case_count}
            </span>
            <span className="mono tabular text-[12px]" style={{ color: "var(--fg)" }}>
              {s.mean_score !== null ? s.mean_score.toFixed(3) : "n/a"}
            </span>
            <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }}>
              {formatUsd(s.total_cost_usd)}
            </span>
          </button>
        );
      })}
    </div>
  );
}
