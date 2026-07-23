import { useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";

/** Warning-triangle glyph, inline SVG (no raster, matching every other icon
 * in this app - `AppHeader.tsx`'s Sun/Moon/BrandMark, `ApprovalsInbox.tsx`'s
 * BellIcon). Deliberately always `--sev-critical`, independent of the
 * dialog's own `tone` prop: the alarm identity of a break-glass override is
 * constant, only the specific action's confirm button varies by severity. */
function WarningIcon() {
  return (
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" aria-hidden="true">
      <path
        d="M12 3.5 21.5 20h-19L12 3.5Z"
        stroke="var(--sev-critical)"
        strokeWidth="1.8"
        strokeLinejoin="round"
      />
      <path d="M12 9.5v5" stroke="var(--sev-critical)" strokeWidth="1.8" strokeLinecap="round" />
      <circle cx="12" cy="17.25" r="1" fill="var(--sev-critical)" />
    </svg>
  );
}

/**
 * The BREAK-GLASS OVERRIDE ceremony (Phase-2 wave 3B): a full-viewport modal
 * that gates a genuinely-privileged mutation (kill / set budget) behind a
 * mandatory, non-empty operator justification. Deliberately its own modal
 * rather than another inline confirm row like `ConfirmButton`'s default
 * idle -> confirming -> pending flow: a dense table row has no room for the
 * "loud, unmistakable" treatment this ceremony is supposed to have, and the
 * reason text deserves more space than a table cell can spare.
 *
 * Hand-rolled (no dialog/modal dependency, matching `ConfirmButton`'s own
 * "not `window.confirm`, a Tauri webview cannot be assumed to support it"
 * reasoning) and `position: fixed` (escapes any scrolling ancestor, matching
 * `index.css`'s ambient backdrop which uses the same technique).
 *
 * `onConfirm` only ever fires with a trimmed, non-empty `reason` - the
 * Confirm button stays disabled until one exists, and Escape/backdrop-click
 * cancel instead of submitting. Still, `money::commands::money_kill_run`/
 * `money_set_budget` re-check on the Rust side before ever calling the
 * Cloud (`require_break_glass_reason`): this dialog is the ceremony, not the
 * authority.
 */
export function BreakGlassDialog({
  open,
  title,
  detail,
  confirmLabel,
  tone = "var(--sev-critical)",
  onCancel,
  onConfirm,
}: {
  open: boolean;
  /** What is being overridden, e.g. "Kill run" / "Set budget". */
  title: string;
  /** Extra context, e.g. the target run id or the amount about to be set. */
  detail?: string;
  confirmLabel: string;
  /** Colors the Confirm button only - the surrounding chrome (icon, title,
   * border) always reads as critical, see `WarningIcon`'s doc. */
  tone?: string;
  onCancel: () => void;
  onConfirm: (reason: string) => Promise<void>;
}) {
  const [reason, setReason] = useState("");
  const [pending, setPending] = useState(false);

  // Every fresh open starts from a blank reason - a leftover reason from a
  // cancelled or already-confirmed dialog must never silently carry over to
  // a different run/budget the next time this opens.
  useEffect(() => {
    if (open) setReason("");
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !pending) onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, pending, onCancel]);

  if (!open) return null;

  const trimmed = reason.trim();
  const canConfirm = trimmed.length > 0 && !pending;

  return (
    <div
      role="presentation"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget && !pending) onCancel();
      }}
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 1000,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        padding: 24,
        background: "rgba(5, 8, 12, 0.62)",
        backdropFilter: "blur(3px)",
        WebkitBackdropFilter: "blur(3px)",
      }}
    >
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="break-glass-dialog-title"
        className="panel"
        style={{
          width: 440,
          maxWidth: "100%",
          maxHeight: "calc(100vh - 48px)",
          overflowY: "auto",
          background: "var(--panel)",
          border: "1px solid color-mix(in srgb, var(--sev-critical) 55%, var(--line-2))",
          boxShadow:
            "0 0 0 1px color-mix(in srgb, var(--sev-critical) 25%, transparent), 0 24px 60px rgba(0, 0, 0, 0.55)",
          borderRadius: "var(--rad)",
          padding: "18px 20px",
          display: "flex",
          flexDirection: "column",
          gap: 13,
        }}
      >
        <div className="flex items-center gap-2">
          <WarningIcon />
          <span
            id="break-glass-dialog-title"
            className="mono"
            style={{
              fontSize: 12,
              fontWeight: 700,
              letterSpacing: "0.1em",
              textTransform: "uppercase",
              color: "var(--sev-critical)",
            }}
          >
            Break-Glass Override
          </span>
        </div>

        <div className="flex flex-col gap-1">
          <span style={{ fontSize: 13.5, color: "var(--fg)" }}>{title}</span>
          {detail && (
            <span className="mono text-[11.5px]" style={{ color: "var(--dim)" }}>
              {detail}
            </span>
          )}
        </div>

        <span className="text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.65 }}>
          This is an ungoverned operator action, not a Wardryx-approved decision. It will be
          journaled to the audit trail together with your reason below, and shown on the Bus tab
          as a <code className="mono">break_glass</code> command.
        </span>

        <label className="flex flex-col gap-1.5">
          <span
            className="mono"
            style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
          >
            Justification (required)
          </span>
          <textarea
            autoFocus
            rows={2}
            value={reason}
            disabled={pending}
            onChange={(e) => setReason(e.target.value)}
            placeholder="Why is this override necessary?"
            className="mono"
            style={{
              resize: "vertical",
              fontSize: 12,
              lineHeight: 1.5,
              padding: "8px 10px",
              background: "var(--panel-2)",
              border: "1px solid color-mix(in srgb, var(--sev-critical) 30%, var(--line-2))",
              borderRadius: "var(--rad-s)",
              color: "var(--fg)",
            }}
          />
        </label>

        <div className="flex items-center justify-end gap-2">
          <button
            type="button"
            className="icon-btn"
            style={{ width: "auto", padding: "0 12px", fontSize: 11.5 }}
            disabled={pending}
            onClick={onCancel}
          >
            Cancel
          </button>
          <button
            type="button"
            className="badge"
            style={{
              ...cssVar("tone", tone),
              padding: "7px 14px",
              fontSize: 11,
              cursor: canConfirm ? "pointer" : "not-allowed",
              opacity: canConfirm ? 1 : 0.5,
            }}
            disabled={!canConfirm}
            onClick={() => {
              setPending(true);
              void onConfirm(trimmed).finally(() => setPending(false));
            }}
          >
            {pending ? "Working..." : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
