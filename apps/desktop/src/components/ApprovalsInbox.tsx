import { useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { formatTimestamp, formatUsd } from "../lib/format";
import { agentIdFromMuteKey, muteKey } from "../lib/notifications";
import type { Approval, Decision, DecideOutcome } from "../policyTypes";
import { ConfirmButton } from "./ConfirmButton";

/** Bell / bell-with-slash glyph for the per-row mute toggle (docs/PHASE2.md
 * Wave 3: "a small mute control in the UI is enough") - inline SVG, no
 * raster, matching every other icon in this app (`AppHeader.tsx`'s
 * Sun/Moon/BrandMark). */
function BellIcon({ muted }: { muted: boolean }) {
  return (
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" aria-hidden="true">
      <path
        d="M6 10a6 6 0 1 1 12 0c0 3.4 1 5 1.5 5.5H4.5C5 15 6 13.4 6 10Z"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinejoin="round"
      />
      <path d="M10 18.5a2 2 0 0 0 4 0" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" />
      {muted && <path d="M3.5 3.5 20.5 20.5" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" />}
    </svg>
  );
}

/** Mute/unmute this row's `agent_id` for future approval notifications
 * (`lib/notifications.ts`'s `muteKey("agent", ...)`) - never touches the
 * approval itself, purely a notification-side preference. */
function MuteToggle({ agentId, muted, onToggle }: { agentId: string; muted: boolean; onToggle: (agentId: string) => void }) {
  return (
    <button
      type="button"
      className="icon-btn"
      style={{ width: 24, height: 24, color: muted ? "var(--sev-medium)" : "var(--faint)" }}
      title={muted ? `Unmute notifications for ${agentId}` : `Mute notifications for ${agentId}`}
      aria-label={muted ? `Unmute notifications for ${agentId}` : `Mute notifications for ${agentId}`}
      aria-pressed={muted}
      onClick={() => onToggle(agentId)}
    >
      <BellIcon muted={muted} />
    </button>
  );
}

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
  highlighted,
  muted,
  onToggleMuteAgent,
  onOpenAgent,
}: {
  approval: Approval;
  onDecide: (id: string, decision: Decision) => Promise<void>;
  highlighted: boolean;
  muted: boolean;
  onToggleMuteAgent: (agentId: string) => void;
  onOpenAgent: (agentId: string) => void;
}) {
  const chain = approval.on_behalf_of;
  return (
    <div
      id={`approval-${approval.approval_id}`}
      className={`panel px-3 py-2.5 flex flex-col gap-2${highlighted ? " approval-focused" : ""}`}
      style={{ background: "var(--panel-2)" }}
    >
      <div className="flex items-center gap-3">
        <span className="badge" style={cssVar("tone", "var(--sev-medium)")}>
          hold
        </span>
        <button
          type="button"
          className="mono truncate text-[12px] text-left"
          title={`Open Agent 360 for ${approval.agent_id}`}
          style={{ color: "var(--fg)", background: "none", border: "none", padding: 0, cursor: "pointer" }}
          onClick={() => onOpenAgent(approval.agent_id)}
        >
          {approval.agent_id}
        </button>
        <MuteToggle agentId={approval.agent_id} muted={muted} onToggle={onToggleMuteAgent} />
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

function HistoryRow({
  approval,
  highlighted,
  onOpenAgent,
}: {
  approval: Approval;
  highlighted: boolean;
  onOpenAgent: (agentId: string) => void;
}) {
  const granted = approval.decision === "grant";
  return (
    <div
      id={`approval-${approval.approval_id}`}
      className={`panel px-3 py-2 flex items-center gap-3${highlighted ? " approval-focused" : ""}`}
      style={{ background: "var(--panel-2)", opacity: 0.85 }}
    >
      <span className="badge" style={cssVar("tone", granted ? "var(--sev-low)" : "var(--sev-critical)")}>
        {approval.decision ?? "decided"}
      </span>
      <div className="flex flex-col min-w-0 flex-1">
        <button
          type="button"
          className="mono truncate text-[12px] text-left"
          title={`Open Agent 360 for ${approval.agent_id}`}
          style={{ color: "var(--fg)", background: "none", border: "none", padding: 0, cursor: "pointer" }}
          onClick={() => onOpenAgent(approval.agent_id)}
        >
          {approval.agent_id}
        </button>
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

/** "muted: agent-a [x] agent-b [x]" strip - only rendered when at least one
 * agent is muted, so the common (nothing muted) case adds no chrome. */
function MutedAgentsStrip({ mutedAgents, onUnmute }: { mutedAgents: readonly string[]; onUnmute: (agentId: string) => void }) {
  if (mutedAgents.length === 0) return null;
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
        notifications muted
      </span>
      {mutedAgents.map((agentId) => (
        <button
          key={agentId}
          type="button"
          className="chip"
          style={{ ...cssVar("dot", "var(--sev-medium)"), cursor: "pointer" }}
          title={`Unmute notifications for ${agentId}`}
          onClick={() => onUnmute(agentId)}
        >
          <span className="dot" aria-hidden="true" />
          <span className="mono truncate" style={{ maxWidth: 220 }}>
            {agentId}
          </span>
          <span aria-hidden="true">&times;</span>
        </button>
      ))}
    </div>
  );
}

/**
 * The Approvals Inbox (PHASE2.md Wave 2): the queue of holds
 * (`pending == true`) with full context (who/what/cost/why/chain), Grant/Deny
 * through an explicit `ConfirmButton` ceremony (this shell's substitute for
 * SwiftUI's Touch ID gate - the hardware gate is a Wave-3 upgrade here),
 * and a history list of already-decided approvals underneath.
 *
 * Wave 3 additions (docs/PHASE2.md "Actionable notifications"): `focusApprovalId`
 * scrolls to and briefly highlights the matching row - the working half of
 * the notification deep link (`lib/notifications.ts`'s doc comment explains
 * why this in-app scroll-to, rather than a real OS notification-click
 * callback, is what actually fires on this desktop build). `mutedKeys`/
 * `onToggleMuteAgent` back the per-row mute toggle that keys
 * `useApprovalNotifications`'s mute set (`lib/notifications.ts`'s
 * `muteKey`/`isMuted` composite-key format, shared verbatim - never a
 * second, differently-shaped "muted" collection) - muting only ever affects
 * whether a FUTURE `approval_requested` raises a notification, never this
 * inbox's own contents or the Grant/Deny path.
 */
export function ApprovalsInbox({
  approvals,
  onDecide,
  grantedToken,
  onDismissToken,
  focusApprovalId,
  mutedKeys,
  onToggleMuteAgent,
  onOpenAgent,
}: {
  approvals: Approval[];
  onDecide: (id: string, decision: Decision) => Promise<void>;
  grantedToken: GrantedToken | null;
  onDismissToken: () => void;
  focusApprovalId: string | null;
  mutedKeys: ReadonlySet<string>;
  onToggleMuteAgent: (agentId: string) => void;
  /** Phase-3 wave-3 deep link (docs/PHASE3.md W3): opens the Agent 360 card
   * for a row's `agent_id`, both pending and decided. */
  onOpenAgent: (agentId: string) => void;
}) {
  const pending = approvals.filter((a) => a.pending);
  const history = approvals.filter((a) => !a.pending);
  const mutedAgentIds = Array.from(mutedKeys)
    .map(agentIdFromMuteKey)
    .filter((id): id is string => id !== null);

  useEffect(() => {
    if (!focusApprovalId) return;
    document.getElementById(`approval-${focusApprovalId}`)?.scrollIntoView({ behavior: "smooth", block: "center" });
  }, [focusApprovalId, approvals]);

  return (
    <div className="flex flex-col gap-3">
      {grantedToken && <GrantedTokenPanel granted={grantedToken} onDismiss={onDismissToken} />}

      <MutedAgentsStrip mutedAgents={mutedAgentIds} onUnmute={onToggleMuteAgent} />

      {pending.length === 0 ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no pending approvals.
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          {pending.map((a) => (
            <PendingRow
              key={a.approval_id}
              approval={a}
              onDecide={onDecide}
              highlighted={a.approval_id === focusApprovalId}
              muted={mutedKeys.has(muteKey("agent", a.agent_id))}
              onToggleMuteAgent={onToggleMuteAgent}
              onOpenAgent={onOpenAgent}
            />
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
            <HistoryRow
              key={a.approval_id}
              approval={a}
              highlighted={a.approval_id === focusApprovalId}
              onOpenAgent={onOpenAgent}
            />
          ))}
        </div>
      )}
    </div>
  );
}
