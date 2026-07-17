import { useState } from "react";
import { cssVar } from "../lib/cssVars";

/**
 * A privileged action button that always shows an inline confirm step
 * before calling `onConfirm` (spec: "a kill/budget action shows a confirm
 * step, then calls the command"). Hand-rolled rather than `window.confirm`:
 * a native dialog is not guaranteed to be enabled in every Tauri webview
 * configuration, and an inline control reads better in a dense table row.
 *
 * Three states: idle -> confirming (Confirm/Cancel) -> pending (disabled,
 * mid-flight). Never double-fires: `onConfirm` cannot be reached again until
 * the previous call has settled.
 */
export function ConfirmButton({
  label,
  confirmLabel,
  pendingLabel = "Working...",
  tone = "var(--sev-high)",
  onConfirm,
  disabled,
}: {
  label: string;
  confirmLabel?: string;
  pendingLabel?: string;
  tone?: string;
  onConfirm: () => Promise<void>;
  disabled?: boolean;
}) {
  const [confirming, setConfirming] = useState(false);
  const [pending, setPending] = useState(false);

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
            void onConfirm().finally(() => {
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
