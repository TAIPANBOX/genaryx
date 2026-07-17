import { cssVar } from "../lib/cssVars";
import { formatTimestamp, formatUsd } from "../lib/format";
import type { Run } from "../moneyTypes";
import { BudgetEditor } from "./BudgetEditor";
import { ConfirmButton } from "./ConfirmButton";

const COLUMNS = "1fr 130px 90px 90px 60px 60px 150px 268px";

export function RunsTable({
  runs,
  onKill,
  onSetBudget,
  onOpenAgent,
  onReplayRun,
}: {
  runs: Run[];
  /** Break-glass (Phase-2 wave 3B): `reason` is the operator's mandatory,
   * non-empty justification collected by `ConfirmButton`'s `BreakGlassDialog`
   * ceremony below, never a plain confirm. */
  onKill: (runId: string, reason: string) => Promise<void>;
  onSetBudget: (runId: string, budgetUsd: number, reason: string) => Promise<void>;
  /** Phase-3 wave-3 deep link (docs/PHASE3.md W3): opens the Agent 360 card
   * for a row's `agent_id`. */
  onOpenAgent: (agentId: string) => void;
  /** Phase-3 wave-4 deep link (docs/PHASE3.md W4): opens Run Replay seeded
   * with this row's `run_id`. Offered regardless of `killed` - a killed
   * run's history is exactly the kind of thing worth replaying. */
  onReplayRun: (runId: string) => void;
}) {
  if (runs.length === 0) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        no runs yet.
      </div>
    );
  }

  return (
    <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
      <div
        className="grid gap-3 px-4 py-2"
        style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line-2)", background: "var(--panel-2)" }}
      >
        {["run", "agent", "spent", "budget", "calls", "steps", "last seen", ""].map((label) => (
          <span
            key={label || "spacer"}
            className="mono"
            style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
          >
            {label}
          </span>
        ))}
      </div>
      {runs.map((r) => (
        <div key={r.run_id} className="grid items-center gap-3 px-4 py-2.5 bus-row" style={{ gridTemplateColumns: COLUMNS }}>
          <span className="mono truncate text-[12px]" title={r.run_id} style={{ color: "var(--fg)" }}>
            {r.run_id}
          </span>
          {r.agent_id ? (
            <button
              type="button"
              className="mono truncate text-[11.5px] text-left"
              title={`Open Agent 360 for ${r.agent_id}`}
              style={{ color: "var(--dim)", background: "none", border: "none", padding: 0, cursor: "pointer" }}
              onClick={() => onOpenAgent(r.agent_id)}
            >
              {r.agent_id}
            </button>
          ) : (
            <span className="mono truncate text-[11.5px]" style={{ color: "var(--dim)" }}>
              -
            </span>
          )}
          <span className="mono tabular text-[12px]" style={{ color: "var(--fg)" }}>
            {formatUsd(r.spent_usd)}
          </span>
          <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }}>
            {r.budget_usd !== null ? formatUsd(r.budget_usd) : "-"}
          </span>
          <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }}>
            {r.calls}
          </span>
          <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }}>
            {r.steps}
          </span>
          <span className="mono tabular text-[11px]" style={{ color: "var(--faint)" }}>
            {formatTimestamp(r.last_seen)}
          </span>
          <span className="flex items-center gap-1.5 justify-end">
            <button
              type="button"
              className="icon-btn"
              style={{ width: "auto", padding: "0 8px", fontSize: 11 }}
              title={`Replay run ${r.run_id}`}
              onClick={() => onReplayRun(r.run_id)}
            >
              Replay
            </button>
            {r.killed ? (
              <span className="badge" style={cssVar("tone", "var(--faint)")}>
                killed
              </span>
            ) : (
              <>
                <BudgetEditor runId={r.run_id} currentUsd={r.budget_usd} onSubmit={onSetBudget} />
                <ConfirmButton
                  label="Kill"
                  confirmLabel="Confirm kill"
                  tone="var(--sev-critical)"
                  breakGlass
                  breakGlassDetail={`run ${r.run_id} · spent ${formatUsd(r.spent_usd)}`}
                  onConfirm={(reason) => onKill(r.run_id, reason)}
                />
              </>
            )}
          </span>
        </div>
      ))}
    </div>
  );
}
