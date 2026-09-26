import { formatTimestamp } from "../lib/format";
import type { VerdryxBaselineSummary, VerdryxRunSummary } from "../qualityTypes";

const COLUMNS = "1fr 1fr 100px 170px";
const COLUMNS_WITH_UNANSWERED = "1fr 1fr 100px 170px 110px";

/**
 * Saved baselines (docs/PHASE4.md W1 position 3): label, mean_score,
 * created_at, and the source run it was snapshotted from. The source run is
 * resolved by joining against the currently-loaded runs list; a baseline
 * whose run has since scrolled out of that list (or was never loaded) still
 * shows its raw `eval_run_id`, never a fabricated label.
 *
 * A baseline taken from a run that had unanswered cases shows that too (an
 * "unanswered" column, same rule as `QualityRunsList` - present only when at
 * least one baseline's source store can answer the question).
 */
export function QualityBaselines({
  baselines,
  runs,
}: {
  baselines: VerdryxBaselineSummary[];
  runs: VerdryxRunSummary[] | null;
}) {
  if (baselines.length === 0) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        no saved baselines yet.
      </div>
    );
  }

  const showUnanswered = baselines.some((b) => b.unanswered_supported);
  const columns = showUnanswered ? COLUMNS_WITH_UNANSWERED : COLUMNS;
  const headers = showUnanswered
    ? ["label", "source run", "mean score", "created", "unanswered"]
    : ["label", "source run", "mean score", "created"];

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
      {baselines.map(({ baseline: b, unanswered_count, unanswered_by_reason, unanswered_supported }) => {
        const sourceRun = runs?.find((r) => r.run.id === b.eval_run_id)?.run ?? null;
        return (
          <div key={b.id} className="grid items-center gap-3 px-5 py-2.5 bus-row" style={{ gridTemplateColumns: columns }}>
            <span className="mono truncate text-[12px]" style={{ color: "var(--fg)" }} title={b.id}>
              {b.label.trim().length > 0 ? b.label : "(unlabeled)"}
            </span>
            <span className="mono truncate text-[11.5px]" style={{ color: "var(--dim)" }} title={b.eval_run_id}>
              {sourceRun ? `${sourceRun.model} · ${sourceRun.id}` : b.eval_run_id}
            </span>
            <span className="mono tabular text-[12px]" style={{ color: "var(--fg)" }}>
              {b.mean_score.toFixed(3)}
            </span>
            <span className="mono tabular text-[11px]" style={{ color: "var(--faint)" }}>
              {formatTimestamp(b.created_at)}
            </span>
            {showUnanswered && (
              <span
                className="mono tabular text-[12px]"
                style={{ color: unanswered_count > 0 ? "var(--sev-high)" : "var(--dim)" }}
                title={
                  !unanswered_supported
                    ? "this baseline's source store predates unanswered accounting"
                    : unanswered_count === 0
                      ? "no unanswered cases in the source run"
                      : `${unanswered_count} unanswered in the source run (${unanswered_by_reason
                          .map((r) => `${r.reason}: ${r.count}`)
                          .join(", ")})`
                }
              >
                {unanswered_supported ? unanswered_count : "-"}
              </span>
            )}
          </div>
        );
      })}
    </div>
  );
}
