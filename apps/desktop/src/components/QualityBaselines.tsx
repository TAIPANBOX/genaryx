import { formatTimestamp } from "../lib/format";
import type { VerdryxBaseline, VerdryxRunSummary } from "../qualityTypes";

const COLUMNS = "1fr 1fr 100px 170px";

/**
 * Saved baselines (docs/PHASE4.md W1 position 3): label, mean_score,
 * created_at, and the source run it was snapshotted from. The source run is
 * resolved by joining against the currently-loaded runs list; a baseline
 * whose run has since scrolled out of that list (or was never loaded) still
 * shows its raw `eval_run_id`, never a fabricated label.
 */
export function QualityBaselines({
  baselines,
  runs,
}: {
  baselines: VerdryxBaseline[];
  runs: VerdryxRunSummary[] | null;
}) {
  if (baselines.length === 0) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        no saved baselines yet.
      </div>
    );
  }

  return (
    <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
      <div
        className="grid gap-3 px-4 py-2"
        style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line-2)", background: "var(--panel-2)" }}
      >
        {["label", "source run", "mean score", "created"].map((label) => (
          <span
            key={label}
            className="mono"
            style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
          >
            {label}
          </span>
        ))}
      </div>
      {baselines.map((b) => {
        const sourceRun = runs?.find((r) => r.run.id === b.eval_run_id)?.run ?? null;
        return (
          <div key={b.id} className="grid items-center gap-3 px-4 py-2.5 bus-row" style={{ gridTemplateColumns: COLUMNS }}>
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
          </div>
        );
      })}
    </div>
  );
}
