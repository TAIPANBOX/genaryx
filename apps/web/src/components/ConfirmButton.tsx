import { useState } from "react";
import { cssVar } from "../lib/cssVars";
import { BreakGlassDialog } from "./BreakGlassDialog";

/**
 * A privileged action button that always shows a confirm step before calling
 * `onConfirm` (spec: "a kill/budget action shows a confirm step, then calls
 * the command"). Hand-rolled rather than `window.confirm`: a native dialog
 * is not guaranteed to be enabled in every Tauri webview configuration, and
 * a purpose-built control reads better in a dense table row.
 *
 * Two confirm ceremonies, selected by `breakGlass`:
 * - default: idle -> confirming (an inline Confirm/Cancel pair) -> pending.
 *   Used for non-privileged-override actions (ack an incident, grant/deny an
 *   approval) - `onConfirm` is called with `""`, which every such caller
 *   ignores.
 * - `breakGlass`: idle -> a `BreakGlassDialog` modal that requires a
 *   non-empty operator justification before Confirm is even clickable ->
 *   pending. Used for `money_kill_run`/`money_set_budget` (Phase-2 wave 3B):
 *   both are genuinely-privileged overrides of Cloud state with no Wardryx
 *   precheck in front of them, so their confirm ceremony has to be louder
 *   and demand a reason - see `BreakGlassDialog`'s own doc for why that is a
 *   modal rather than another inline row.
 *
 * Never double-fires either way: `onConfirm` cannot be reached again until
 * the previous call has settled.
 */
export function ConfirmButton({
  label,
  confirmLabel,
  pendingLabel = "Working...",
  tone = "var(--sev-high)",
  onConfirm,
  disabled,
  breakGlass = false,
  breakGlassDetail,
}: {
  label: string;
  confirmLabel?: string;
  pendingLabel?: string;
  tone?: string;
  /** Called with the trimmed operator reason when `breakGlass` is set,
   * otherwise called with `""` - non-break-glass callers take no `reason`
   * parameter of their own and simply ignore it. */
  onConfirm: (reason: string) => Promise<void>;
  disabled?: boolean;
  /** Gate this action behind a BREAK-GLASS OVERRIDE modal that requires a
   * justification instead of the plain inline Confirm/Cancel step below. */
  breakGlass?: boolean;
  /** Extra context shown inside the break-glass modal (e.g. the target run
   * id) - ignored unless `breakGlass` is set. */
  breakGlassDetail?: string;
}) {
  const [confirming, setConfirming] = useState(false);
  const [pending, setPending] = useState(false);

  if (breakGlass) {
    return (
      <>
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 10px", fontSize: 11, color: tone, whiteSpace: "nowrap" }}
          onClick={() => setConfirming(true)}
          disabled={disabled || pending}
        >
          {pending ? pendingLabel : label}
        </button>
        <BreakGlassDialog
          open={confirming}
          title={confirmLabel ?? label}
          detail={breakGlassDetail}
          confirmLabel={confirmLabel ?? "Confirm"}
          tone={tone}
          onCancel={() => setConfirming(false)}
          onConfirm={(reason) => {
            setPending(true);
            return onConfirm(reason).finally(() => {
              setPending(false);
              setConfirming(false);
            });
          }}
        />
      </>
    );
  }

  if (pending) {
    return (
      <button
        type="button"
        className="icon-btn"
        disabled
        style={{ width: "auto", padding: "0 10px", fontSize: 11, whiteSpace: "nowrap" }}
      >
        {pendingLabel}
      </button>
    );
  }

  if (confirming) {
    return (
      <span className="inline-flex items-center gap-1.5">
        <button
          type="button"
          className="badge"
          style={cssVar("tone", tone)}
          onClick={() => {
            setPending(true);
            void onConfirm("").finally(() => {
              setPending(false);
              setConfirming(false);
            });
          }}
        >
          {confirmLabel ?? "Confirm"}
        </button>
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 8px", fontSize: 11 }}
          onClick={() => setConfirming(false)}
        >
          Cancel
        </button>
      </span>
    );
  }

  return (
    <button
      type="button"
      className="icon-btn"
      style={{ width: "auto", padding: "0 10px", fontSize: 11, color: tone, whiteSpace: "nowrap" }}
      onClick={() => setConfirming(true)}
      disabled={disabled}
    >
      {label}
    </button>
  );
}
