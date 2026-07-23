import { useState } from "react";
import { formatUsd } from "../lib/format";
import { BreakGlassDialog } from "./BreakGlassDialog";

/**
 * Inline per-row budget editor: `Edit` reveals a number input, `Set` opens
 * the BREAK-GLASS OVERRIDE modal (Phase-2 wave 3B - `money_set_budget` is a
 * genuinely-privileged Cloud-state override, same ceremony `ConfirmButton`'s
 * `breakGlass` mode uses for Kill, see `BreakGlassDialog`'s doc), and only a
 * confirm there (with a non-empty operator reason) calls `onSubmit`.
 */
export function BudgetEditor({
  runId,
  currentUsd,
  onSubmit,
}: {
  runId: string;
  currentUsd: number | null;
  /** `reason` is the operator's mandatory break-glass justification,
   * collected by the modal below - never empty by the time this is called. */
  onSubmit: (runId: string, budgetUsd: number, reason: string) => Promise<void>;
}) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(currentUsd !== null ? String(currentUsd) : "");
  // `confirming` also stands in for "pending": `BreakGlassDialog` covers the
  // whole viewport and owns its own internal pending/disabled state while
  // `onSubmit` is in flight, so this only needs one flag for "the modal is
  // open" - not a second one for "and the mutation call inside it hasn't
  // settled yet".
  const [confirming, setConfirming] = useState(false);

  if (!editing) {
    return (
      <button
        type="button"
        className="icon-btn"
        style={{ width: "auto", padding: "0 10px", fontSize: 11, whiteSpace: "nowrap" }}
        onClick={() => {
          setValue(currentUsd !== null ? String(currentUsd) : "");
          setConfirming(false);
          setEditing(true);
        }}
      >
        Budget
      </button>
    );
  }

  const parsed = Number(value);
  const validAmount = value.trim() !== "" && Number.isFinite(parsed) && parsed >= 0;

  return (
    <span className="inline-flex items-center gap-1.5">
      <input
        type="number"
        min={0}
        step="0.01"
        value={value}
        disabled={confirming}
        onChange={(e) => setValue(e.target.value)}
        className="mono tabular"
        style={{
          width: 78,
          fontSize: 11.5,
          background: "var(--panel-2)",
          border: "1px solid var(--line-2)",
          borderRadius: 6,
          padding: "3px 6px",
          color: "var(--fg)",
        }}
      />
      <button
        type="button"
        className="icon-btn"
        disabled={!validAmount || confirming}
        style={{ width: "auto", padding: "0 8px", fontSize: 11 }}
        onClick={() => setConfirming(true)}
      >
        Set
      </button>
      <button
        type="button"
        className="icon-btn"
        style={{ width: "auto", padding: "0 8px", fontSize: 11 }}
        disabled={confirming}
        onClick={() => setEditing(false)}
      >
        Close
      </button>
      <BreakGlassDialog
        open={confirming}
        title="Set budget"
        detail={`run ${runId} -> ${formatUsd(parsed)}`}
        confirmLabel={`Confirm ${formatUsd(parsed)}`}
        tone="var(--sev-medium)"
        onCancel={() => setConfirming(false)}
        onConfirm={(reason) =>
          onSubmit(runId, parsed, reason).then(
            () => {
              setConfirming(false);
              setEditing(false);
            },
            () => {
              // Left open on failure (matches every other mutation's
              // fail-closed contract): the operator can see the error
              // banner `MoneyView` renders and retry without re-entering
              // the amount or the reason from scratch.
            },
          )
        }
      />
    </span>
  );
}
