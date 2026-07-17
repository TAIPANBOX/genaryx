import { useState } from "react";
import { cssVar } from "../lib/cssVars";

/**
 * Inline per-row budget editor: `Edit` reveals a number input, `Set` moves
 * to an explicit confirm step (same "always confirm a privileged mutation"
 * rule as `ConfirmButton`), and only then calls `onSubmit`.
 */
export function BudgetEditor({
  runId,
  currentUsd,
  onSubmit,
}: {
  runId: string;
  currentUsd: number | null;
  onSubmit: (runId: string, budgetUsd: number) => Promise<void>;
}) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(currentUsd !== null ? String(currentUsd) : "");
  const [confirming, setConfirming] = useState(false);
  const [pending, setPending] = useState(false);

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
        disabled={pending}
        onChange={(e) => {
          setValue(e.target.value);
          setConfirming(false);
        }}
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
      {confirming ? (
        <>
          <button
            type="button"
            className="badge"
            style={cssVar("tone", "var(--sev-medium)")}
            disabled={pending}
            onClick={() => {
              setPending(true);
              void onSubmit(runId, parsed).then(
                () => {
                  setPending(false);
                  setConfirming(false);
                  setEditing(false);
                },
                () => {
                  setPending(false);
                },
              );
            }}
          >
            {pending ? "Setting..." : `Confirm $${parsed.toFixed(2)}`}
          </button>
          <button
            type="button"
            className="icon-btn"
            style={{ width: "auto", padding: "0 8px", fontSize: 11 }}
            disabled={pending}
            onClick={() => setConfirming(false)}
          >
            Cancel
          </button>
        </>
      ) : (
        <>
          <button
            type="button"
            className="icon-btn"
            disabled={!validAmount}
            style={{ width: "auto", padding: "0 8px", fontSize: 11 }}
            onClick={() => setConfirming(true)}
          >
            Set
          </button>
          <button
            type="button"
            className="icon-btn"
            style={{ width: "auto", padding: "0 8px", fontSize: 11 }}
            onClick={() => setEditing(false)}
          >
            Close
          </button>
        </>
      )}
    </span>
  );
}
