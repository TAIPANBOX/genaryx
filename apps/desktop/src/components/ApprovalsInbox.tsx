import { useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { formatTimestamp, formatUsd } from "../lib/format";
import type { Approval, Decision, DecideOutcome } from "../policyTypes";
import { ConfirmButton } from "./ConfirmButton";

/** One just-granted token, kept in the parent's state so it survives the
 * approval moving from the pending queue into history on the next refresh -
 * see `PolicyView.tsx`. */
export interface GrantedToken {
  approvalId: string;
  outcome: DecideOutcome;
}

function Field({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <div className="flex items-baseline gap-2 min-w-0">
      <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
        {label}
      </span>
      <span className="mono tabular truncate text-[11.5px]" style={{ color: "var(--dim)" }} title={title ?? value}>
        {value}
      </span>
    </div>
  );
}

/** Live-ticking countdown to `expUnix`, recomputed every second while
 * mounted - never a value frozen at the moment the token was decoded, since
 * the whole point is showing the operator how much time is actually left
 * right now. */
function useCountdown(expUnix: number): { remainingMs: number; label: string } {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);

  const remainingMs = Math.max(0, expUnix * 1000 - now);
  const totalSeconds = Math.floor(remainingMs / 1000);
  const mm = String(Math.floor(totalSeconds / 60)).padStart(2, "0");
  const ss = String(totalSeconds % 60).padStart(2, "0");
  return { remainingMs, label: `${mm}:${ss}` };
}

/**
 * Shown once, right after a Grant: the decoded `approval_token` claims
 * (PHASE2.md Wave 2 - "show the operator exactly what they authorized").
 * This is the ONLY time the token's contents are ever visible - Wardryx
 * never lets it be retrieved again - so it stays on screen until the
 * operator dismisses it, independent of the approval's own row moving to
 * history underneath.
 */
function GrantedTokenPanel({ granted, onDismiss }: { granted: GrantedToken; onDismiss: () => void }) {
  const token = granted.outcome.token;
  const countdown = useCountdown(token?.exp_unix ?? 0);
  if (!token) return null;

  const expired = countdown.remainingMs <= 0;

  return (
    <div
      className="panel px-4 py-3 flex flex-col gap-2"
      style={{
        background: "color-mix(in srgb, var(--sev-low) 10%, var(--panel-2))",
        borderColor: "color-mix(in srgb, var(--sev-low) 45%, var(--line-2))",
      }}
    >
      <div className="flex items-center gap-2">
        <span className="badge" style={cssVar("tone", "var(--sev-low)")}>
          token minted
        </span>
        <span className="mono text-[12px]" style={{ color: "var(--fg)" }}>
          {granted.approvalId}
        </span>
        <div className="flex-1" />
        <button type="button" className="icon-btn" style={{ width: "auto", padding: "0 8px", fontSize: 11 }} onClick={onDismiss}>
          Dismiss
        </button>
      </div>
      <div className="flex flex-wrap gap-x-5 gap-y-1.5">
        <Field label="agent" value={token.agent_id} />
        <Field label="run" value={token.run_id} />
        <Field label="tools" value={token.tools.length > 0 ? token.tools.join(", ") : "(none)"} />
      </div>
      <div className="flex items-center gap-5">
        <span className="mono tabular text-[13px]" style={{ color: "var(--fg)" }}>
          ceiling {formatUsd(token.cost_ceiling_usd)}
        </span>
        <span
          className="mono tabular text-[13px]"
          style={{ color: expired ? "var(--sev-critical)" : "var(--fg)" }}
        >
          {expired ? "expired" : `expires in ${countdown.label}`}
        </span>
      </div>
      <span className="text-[11px]" style={{ color: "var(--faint)" }}>
        Single-use if the server enforces WARDRYX_APPROVAL_SINGLE_USE - this console cannot verify a token&rsquo;s
        replay state, and this token cannot be shown again once dismissed.
      </span>
    </div>
  );
}

function PendingRow({
  approval,
  onDecide,
}: {
  approval: Approval;
  onDecide: (id: string, decision: Decision) => Promise<void>;
}) {
  const chain = approval.on_behalf_of;
  return (
    <div className="panel px-3 py-2.5 flex flex-col gap-2" style={{ background: "var(--panel-2)" }}>
      <div className="flex items-center gap-3">
        <span className="badge" style={cssVar("tone", "var(--sev-medium)")}>
          hold
        </span>
        <span className="mono truncate text-[12px]" title={approval.agent_id} style={{ color: "var(--fg)" }}>
          {approval.agent_id}
        </span>
        <span className="mono tabular text-[11px]" style={{ color: "var(--faint)" }}>
          {formatTimestamp(approval.requested_at)}
        </span>
        <div className="flex-1" />
        <span className="mono tabular text-[12.5px]" style={{ color: "var(--fg)" }}>
          {approval.est_cost_usd !== null ? formatUsd(approval.est_cost_usd) : "-"}
        </span>
      </div>

      <div className="flex flex-wrap gap-x-5 gap-y-1.5">
        <Field label="run" value={approval.run_id} />
        <Field label="tools" value={approval.tool_names.length > 0 ? approval.tool_names.join(", ") : "-"} />
        <Field label="policy_version" value={approval.policy_version ?? "-"} />
        {chain && chain.length > 0 && <Field label="on_behalf_of" value={chain.join(" -> ")} />}
      </div>

      {approval.reason && (
        <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
          {approval.reason}
        </span>
      )}

      <div className="flex items-center gap-2 justify-end">
        <ConfirmButton
          label="Grant"
          confirmLabel="Confirm grant"
          tone="var(--sev-low)"
          onConfirm={() => onDecide(approval.approval_id, "grant")}
        />
        <ConfirmButton
          label="Deny"
          confirmLabel="Confirm deny"
          tone="var(--sev-critical)"
          onConfirm={() => onDecide(approval.approval_id, "deny")}
        />
      </div>
    </div>
  );
}

function HistoryRow({ approval }: { approval: Approval }) {
  const granted = approval.decision === "grant";
  return (
    <div className="panel px-3 py-2 flex items-center gap-3" style={{ background: "var(--panel-2)", opacity: 0.85 }}>
      <span className="badge" style={cssVar("tone", granted ? "var(--sev-low)" : "var(--sev-critical)")}>
        {approval.decision ?? "decided"}
      </span>
      <div className="flex flex-col min-w-0 flex-1">
        <span className="mono truncate text-[12px]" title={approval.agent_id} style={{ color: "var(--fg)" }}>
          {approval.agent_id}
        </span>
        <span className="mono truncate text-[11px]" style={{ color: "var(--faint)" }}>
          {approval.decided_by ?? "unknown"} &middot; {approval.decided_at ? formatTimestamp(approval.decided_at) : "-"}
        </span>
      </div>
      <span className="mono tabular text-[11.5px]" style={{ color: "var(--dim)" }}>
        {approval.est_cost_usd !== null ? formatUsd(approval.est_cost_usd) : "-"}
      </span>
    </div>
  );
}

/**
 * The Approvals Inbox (PHASE2.md Wave 2): the queue of holds
 * (`pending == true`) with full context (who/what/cost/why/chain), Grant/Deny
 * through an explicit `ConfirmButton` ceremony (this shell's substitute for
 * SwiftUI's Touch ID gate - the hardware gate is a Wave-3 upgrade here),
 * and a history list of already-decided approvals underneath.
 */
export function ApprovalsInbox({
  approvals,
  onDecide,
  grantedToken,
  onDismissToken,
}: {
  approvals: Approval[];
  onDecide: (id: string, decision: Decision) => Promise<void>;
  grantedToken: GrantedToken | null;
  onDismissToken: () => void;
}) {
  const pending = approvals.filter((a) => a.pending);
  const history = approvals.filter((a) => !a.pending);

  return (
    <div className="flex flex-col gap-3">
      {grantedToken && <GrantedTokenPanel granted={grantedToken} onDismiss={onDismissToken} />}

      {pending.length === 0 ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no pending approvals.
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          {pending.map((a) => (
            <PendingRow key={a.approval_id} approval={a} onDecide={onDecide} />
          ))}
        </div>
      )}

      {history.length > 0 && (
        <div className="flex flex-col gap-2">
          <span
            className="mono"
            style={{ fontSize: 10, letterSpacing: "0.1em", textTransform: "uppercase", color: "var(--faint)" }}
          >
            History
          </span>
          {history.map((a) => (
            <HistoryRow key={a.approval_id} approval={a} />
          ))}
        </div>
      )}
    </div>
  );
}
