import { describeQualityError } from "../lib/quality";
import { formatUsd } from "../lib/format";
import type { QualityError, VerdryxRunSummary, VerdryxScore } from "../qualityTypes";
import { StatTile } from "./StatTile";

const COLUMNS = "1fr 90px 90px 110px";

/**
 * Run detail (docs/PHASE4.md W1 position 2): the run-summary header (mean
 * score, total tokens, total cost, case count - mean shown as "n/a" when
 * `mean_score` is null, never `0`) plus the per-case scores table.
 */
export function QualityRunDetail({
  summary,
  scores,
  error,
}: {
  summary: VerdryxRunSummary | null;
  scores: VerdryxScore[] | null;
  error: QualityError | null;
}) {
  if (!summary) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        select a run above to see its detail.
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="grid gap-3" style={{ gridTemplateColumns: "repeat(4, minmax(0, 1fr))" }}>
        <StatTile label="Mean score" value={summary.mean_score !== null ? summary.mean_score.toFixed(3) : "n/a"} />
        <StatTile label="Cases" value={String(summary.case_count)} />
        <StatTile label="Total tokens" value={summary.total_tokens.toLocaleString()} />
        <StatTile label="Total cost" value={formatUsd(summary.total_cost_usd)} />
      </div>

      {error && (
        <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
          {describeQualityError(error)}
        </div>
      )}

      {scores === null ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          loading scores...
        </div>
      ) : scores.length === 0 ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no per-case scores for this run.
        </div>
      ) : (
        <div style={{ overflowX: "auto" }}>
          <div
            className="grid gap-3 px-5 py-2"
            style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line)" }}
          >
            {["case", "value", "tokens", "cost"].map((label) => (
              <span
                key={label}
                className="mono"
                style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
              >
                {label}
              </span>
            ))}
          </div>
          {scores.map((s) => (
            <div key={s.id} className="grid items-center gap-3 px-5 py-2 bus-row" style={{ gridTemplateColumns: COLUMNS }}>
              <span className="mono truncate text-[12px]" style={{ color: "var(--fg)" }} title={s.case_id}>
                {s.case_id}
              </span>
              <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }}>
                {s.value.toFixed(3)}
              </span>
              <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }}>
                {s.tokens}
              </span>
              <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }}>
                {formatUsd(s.cost_usd)}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
