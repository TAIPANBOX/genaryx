import type { VerdryxRunSummary } from "../qualityTypes";
import { formatTimestamp, formatUsd } from "../lib/format";

const COLUMNS = "1fr 150px 150px 70px 100px 100px";
const COLUMNS_WITH_UNANSWERED = "1fr 150px 150px 70px 100px 100px 110px";

/** "3 (label_mass_too_low: 2, timeout: 1)", or "0" when nothing was
 * unanswered - the compact cell text; the full sentence lives in the cell's
 * `title` tooltip and in `QualityRunDetail`, where there is room for it. */
function unansweredCell(s: VerdryxRunSummary): string {
  if (s.unanswered_count === 0) return "0";
  return String(s.unanswered_count);
}

function unansweredTitle(s: VerdryxRunSummary): string {
  if (s.unanswered_count === 0) return "no unanswered cases for this run";
  const reasons = s.unanswered_by_reason.map((r) => `${r.reason}: ${r.count}`).join(", ");
  return `${s.unanswered_count} unanswered (${reasons})`;
}

/**
 * Eval-runs history (docs/PHASE4.md W1 position 1): model, started/finished,
 * and the per-run summary (case count, mean score, total cost), newest
 * first. Clicking a row selects it for `QualityRunDetail`.
 *
 * The "unanswered" column only appears when at least one run's store can
 * answer the question (`unanswered_supported`) - a `verdryx.db` written
 * before verdryx's `unanswered` table existed renders the SIX original
 * columns, never a seventh full of `0`s that would look measured.
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

  const showUnanswered = runs.some((r) => r.unanswered_supported);
  const columns = showUnanswered ? COLUMNS_WITH_UNANSWERED : COLUMNS;
  const headers = showUnanswered
    ? ["run", "started", "finished", "cases", "mean score", "total cost", "unanswered"]
    : ["run", "started", "finished", "cases", "mean score", "total cost"];

  return (
    <div style={{ overflowX: "auto" }}>
      <div
        className="grid gap-3 px-5 py-2"
        style={{ gridTemplateColumns: columns, borderBottom: "1px solid var(--line)" }}
      >
        {headers.map((label) => (
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
              gridTemplateColumns: columns,
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
              {s.mean_score !== null ? s.mean_score.toFixed(3) : "unmeasured"}
            </span>
            <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }}>
              {formatUsd(s.total_cost_usd)}
            </span>
            {showUnanswered && (
              <span
                className="mono tabular text-[12px]"
                style={{ color: s.unanswered_count > 0 ? "var(--sev-high)" : "var(--dim)" }}
                title={unansweredTitle(s)}
              >
                {unansweredCell(s)}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}
