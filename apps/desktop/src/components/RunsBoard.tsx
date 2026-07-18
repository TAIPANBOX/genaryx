import { formatUsd } from "../lib/format";
import { agentShortName } from "../lib/dashData";
import type { Run } from "../moneyTypes";
import { BudgetEditor } from "./BudgetEditor";
import { ConfirmButton } from "./ConfirmButton";
import { FuseBar } from "./FuseBar";

const COLS = "minmax(0, 1.5fr) 210px 96px 100px 250px";

/** The interactive runs board on the Money dashboard: a readable, dashboard-
 * styled table (not the old dense grid) that keeps every operator action -
 * open the agent's 360 (in place), replay, set a budget, and break-glass kill.
 * The caller passes an already-curated, capped slice (the full firehose lives
 * in Bus Explorer), so this never renders thousands of rows. */
export function RunsBoard({
  runs,
  onKill,
  onSetBudget,
  onOpenAgent,
  onReplayRun,
}: {
  runs: Run[];
  onKill: (runId: string, reason: string) => Promise<void>;
  onSetBudget: (runId: string, budgetUsd: number, reason: string) => Promise<void>;
  onOpenAgent: (agentId: string) => void;
  onReplayRun: (runId: string) => void;
}) {
  if (runs.length === 0) {
    return (
      <div className="mono" style={{ fontSize: 12, color: "var(--faint)", padding: "24px 20px" }}>
        no runs yet.
      </div>
    );
  }
  return (
    <div>
      <div className="d-th" style={{ gridTemplateColumns: COLS }}>
        <span>run · agent</span>
        <span>spent / budget</span>
        <span className="r">calls · steps</span>
        <span>status</span>
        <span className="r"> </span>
      </div>
      {runs.map((r) => {
        const frac = r.budget_usd && r.budget_usd > 0 ? r.spent_usd / r.budget_usd : 0;
        const status = r.killed ? "dead" : frac >= 1 ? "over" : frac >= 0.8 ? "near" : "live";
        const statusLabel = r.killed ? "killed" : frac >= 1 ? "over cap" : frac >= 0.8 ? "near cap" : "live";
        return (
          <div className="d-tr" style={{ gridTemplateColumns: COLS }} key={r.run_id}>
            <div className="d-run">
              <div className="rid" title={r.run_id}>
                {r.run_id}
              </div>
              {r.agent_id ? (
                <button
                  type="button"
                  className="rag"
                  title={`Open Agent 360 for ${r.agent_id}`}
                  onClick={() => onOpenAgent(r.agent_id)}
                >
                  {agentShortName(r.agent_id)}
                </button>
              ) : (
                <span className="rag">-</span>
              )}
            </div>
            <div className="d-spentcell">
              <div className="amt">
                <span>{formatUsd(r.spent_usd)}</span>
                <span className="cap">{r.budget_usd !== null ? formatUsd(r.budget_usd) : "no cap"}</span>
              </div>
              {r.budget_usd !== null && r.budget_usd > 0 && <FuseBar fraction={frac} />}
            </div>
            <div className="d-num cell-r">
              {r.calls} · {r.steps}
            </div>
            <div>
              <span className={`d-pill ${status}`}>{statusLabel}</span>
            </div>
            <div className="d-acts">
              <button
                type="button"
                className="icon-btn"
                style={{ width: "auto", padding: "0 9px", fontSize: 11 }}
                title={`Replay run ${r.run_id}`}
                onClick={() => onReplayRun(r.run_id)}
              >
                Replay
              </button>
              {r.killed ? (
                <span className="d-pill dead">killed</span>
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
            </div>
          </div>
        );
      })}
    </div>
  );
}
