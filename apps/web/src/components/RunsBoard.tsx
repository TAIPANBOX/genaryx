import { formatUsd } from "../lib/format";
import { agentShortName } from "../lib/dashData";
import type { Run } from "../moneyTypes";
import { runBlockedState } from "../lib/lifecycle";
import { LIFECYCLE_BADGE, lifecyclePillClass } from "../lib/lifecycleTypes";
import { cacheHitsLabel, NOT_RECORDED, runModelLabel, runUnitLabel } from "../lib/moneyExport";
import { BudgetEditor } from "./BudgetEditor";
import { ConfirmButton } from "./ConfirmButton";
import { FuseBar } from "./FuseBar";

/** The floor under the run/agent/model column.
 *
 * It used to be `minmax(0, 1.5fr)`, a track a browser is allowed to lay out at
 * zero, and it took it. @measured in the running mock demo on 2026-08-26: in
 * the default 1440px layout with both rails open the runs card is 760px, and
 * the four fixed tracks plus four 14px gaps plus 40px of padding came to
 * exactly 720px, so the run id and the agent name were laid out 0px wide and
 * nothing was drawn in that column at all. At origin/main's widths the same
 * measurement gave 8px, which is the same defect one pixel short of total: the
 * collapse is older than this branch, and widening the numeric column to carry
 * the cache-hit count is what closed the last of it.
 *
 * A floor here would only move the overflow somewhere else, so the board also
 * scrolls (see [`BOARD_MIN_WIDTH_PX`]). Squeezing is the failure mode worth
 * avoiding: a scrollbar tells the operator there is more, a 0px column does
 * not. */
export const RUN_COL_MIN_PX = 220;

/** The board's columns as widths, not as a string, so [`BOARD_MIN_WIDTH_PX`]
 * is derived from them and a column cannot be widened without the board's own
 * minimum following. That drift is exactly what happened above. */
export const RUNS_BOARD_TRACKS: { css: string; minPx: number }[] = [
  { css: `minmax(${RUN_COL_MIN_PX}px, 1.5fr)`, minPx: RUN_COL_MIN_PX },
  { css: "210px", minPx: 210 },
  // 104 rather than 96: the numeric column carries two lines now, calls/steps
  // and the cache-hit count, rather than a fifth column. Cache hits are worth
  // 8px of width, not 100.
  { css: "104px", minPx: 104 },
  { css: "100px", minPx: 100 },
  { css: "250px", minPx: 250 },
];

const COLS = RUNS_BOARD_TRACKS.map((t) => t.css).join(" ");

/** The width below which the board scrolls rather than compresses.
 *
 * The two constants are `.d-th`/`.d-tr`'s own in `index.css` (a 14px column
 * gap and 20px of padding a side), which this component does not own and
 * therefore restates rather than reads. A test re-derives this sum, so if that
 * CSS or a track changes and this does not, the arithmetic is caught here
 * rather than by somebody noticing a missing column on a dashboard. */
export const BOARD_MIN_WIDTH_PX =
  RUNS_BOARD_TRACKS.reduce((n, t) => n + t.minPx, 0) + 14 * (RUNS_BOARD_TRACKS.length - 1) + 20 * 2;

/** The interactive runs board on the Money dashboard: a readable, dashboard-
 * styled table (not the old dense grid) that keeps every operator action -
 * open the agent's 360 (in place), replay, set a budget, and break-glass kill.
 * The caller passes an already-curated, capped slice (the full firehose lives
 * in Bus Explorer), so this never renders thousands of rows. */
export function RunsBoard({
  runs,
  onKill,
  onSetBudget,
  onOpenAgentAt,
  onReplayRun,
}: {
  runs: Run[];
  onKill: (runId: string, reason: string) => Promise<void>;
  onSetBudget: (runId: string, budgetUsd: number, reason: string) => Promise<void>;
  /** Open the agent's detail card in the floating layer, anchored at the given
   * on-screen rect. The WHOLE row is the target (not just the tiny agent name),
   * so a click anywhere on a run opens its agent; the action controls below
   * stop their own clicks from bubbling up to it. */
  onOpenAgentAt: (agentId: string, rect: DOMRect) => void;
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
    // The scroll container and its inner minimum are one unit: without the
    // min-width the rows would shrink back to the container and the scrollbar
    // would never appear. Both row controls expand inline rather than in a
    // popover, and the agent card is a fixed-position portal, so nothing here
    // is clipped by the new overflow context.
    //
    // `contain: inline-size` is the load-bearing half and was NOT obvious.
    // Giving the run track a 220px floor raises this board's min-content width
    // to the full 980, and `.d-main` in index.css sizes its main column from
    // that: @measured on 2026-08-26 the grid went to `982px 360px` inside a
    // 946px container and pushed the Governed savings rail off the right edge
    // of a 1440px window, caption ending at x=1595. `min-width: 0` on the
    // scroller does not stop it. Containment does, by saying this box's inline
    // size is not computed from its contents: measured again, the same grid
    // came back as `570px 360px`, the rail ended at x=1183, the run column
    // stayed 220px and the board scrolled. index.css belongs to another track,
    // so the cure has to live on this side of the boundary anyway.
    <div style={{ overflowX: "auto", contain: "inline-size" }}>
      <div style={{ minWidth: BOARD_MIN_WIDTH_PX }}>
      <div className="d-th" style={{ gridTemplateColumns: COLS }}>
        <span>run · agent · model</span>
        <span>spent / budget</span>
        <span className="r">
          calls · steps
          <br />
          cache hits
        </span>
        <span>status</span>
        <span className="r"> </span>
      </div>
      {runs.map((r) => {
        const frac = r.budget_usd && r.budget_usd > 0 ? r.spent_usd / r.budget_usd : 0;
        // A blocked run (STOPPED/FROZEN/KILLED) shows its precise lifecycle
        // pill; a live run keeps the utilisation-driven live/near/over pill.
        const blocked = runBlockedState(r);
        const status = blocked ? lifecyclePillClass(blocked)! : frac >= 1 ? "over" : frac >= 0.8 ? "near" : "live";
        const statusLabel = blocked
          ? LIFECYCLE_BADGE[blocked].label.toLowerCase()
          : frac >= 1
            ? "over cap"
            : frac >= 0.8
              ? "near cap"
              : "live";
        return (
          <div
            className="d-tr"
            style={{ gridTemplateColumns: COLS, cursor: r.agent_id ? "pointer" : undefined }}
            key={r.run_id}
            role={r.agent_id ? "button" : undefined}
            tabIndex={r.agent_id ? 0 : undefined}
            onClick={r.agent_id ? (e) => onOpenAgentAt(r.agent_id, e.currentTarget.getBoundingClientRect()) : undefined}
            onKeyDown={
              r.agent_id
                ? (e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      onOpenAgentAt(r.agent_id, (e.currentTarget as HTMLElement).getBoundingClientRect());
                    }
                  }
                : undefined
            }
          >
            <div className="d-run">
              <div className="rid" title={r.run_id}>
                {r.run_id}
              </div>
              {r.agent_id ? (
                <span className="rag" title={r.agent_id}>
                  {agentShortName(r.agent_id)}
                </span>
              ) : (
                <span className="rag">-</span>
              )}
              {/* The model priced this run and the unit was charged for it.
                  Both ride on every row of GET /v1/runs and rendered nowhere
                  outside Incident360 until now. An empty unit is the Cloud's
                  identity map answering "none", which is why it says that
                  rather than showing a blank. */}
              <span
                className="rag"
                style={{ fontSize: 9.5, color: "var(--faint)" }}
                title={`model ${runModelLabel(r)} · unit ${runUnitLabel(r)}`}
              >
                {runModelLabel(r)} · {runUnitLabel(r)}
              </span>
            </div>
            <div className="d-spentcell">
              <div className="amt">
                <span>{formatUsd(r.spent_usd)}</span>
                <span className="cap">{r.budget_usd !== null ? formatUsd(r.budget_usd) : "no cap"}</span>
              </div>
              {r.budget_usd !== null && r.budget_usd > 0 && <FuseBar fraction={frac} />}
            </div>
            <div className="d-num cell-r">
              <div>
                {r.calls} · {r.steps}
              </div>
              {/* A cache hit is spend that did not happen, so a run with none
                  is the one worth looking at. 0 says the money plane counted
                  none; "not recorded" says it never sent the field. Printing
                  a 0 for the second would be a measurement nobody took. */}
              <div
                style={{ fontSize: 10.5, color: "var(--faint)", marginTop: 2 }}
                title={
                  cacheHitsLabel(r) === NOT_RECORDED
                    ? "This box sent no cache_hits for this run. Not the same as zero cache hits."
                    : `${cacheHitsLabel(r)} cache hit(s) the money plane counted for this run`
                }
              >
                {cacheHitsLabel(r)}
              </div>
            </div>
            <div>
              <span className={`d-pill ${status}`}>{statusLabel}</span>
            </div>
            <div className="d-acts" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
              <button
                type="button"
                className="icon-btn"
                style={{ width: "auto", padding: "0 9px", fontSize: 11 }}
                title={`Replay run ${r.run_id}`}
                onClick={() => onReplayRun(r.run_id)}
              >
                Replay
              </button>
              {blocked ? (
                <span className={`d-pill ${lifecyclePillClass(blocked)}`}>{LIFECYCLE_BADGE[blocked].label.toLowerCase()}</span>
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
    </div>
  );
}
