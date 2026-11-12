import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { cssVar } from "../lib/cssVars";
import { formatTimestamp, formatUsd } from "../lib/format";
import {
  APPROVAL_TOKEN_HEADER,
  grantedTokenNotice,
  isExpired,
  SINGLE_USE_EXCEPTION,
  tokenPlainLine,
  type GrantedToken,
} from "../lib/approvalToken";
import { useLeaveGuard } from "../lib/leaveGuard";
import { agentIdFromMuteKey, muteKey } from "../lib/notifications";
import { repeatOffers, standingGrantOffer, type RepeatOffer, type StandingGrantOffer } from "../lib/standingGrant";
import type { ConsoleRole } from "../lib/transport";
import {
  approvalArgumentsLabel,
  approvalArgumentsTitle,
  approvalModelLabel,
  approvalOrgLabel,
  approvalPolicyVersionLabel,
  approvalReasonLabel,
  approvalTargetLabel,
  NO_CALL_ON_HOLD,
  NOT_RECORDED,
  visibleText,
} from "../lib/policyExport";
import type { Approval, ApprovalPreview, Decision, PolicyRecord } from "../policyTypes";
import { fetchApprovalPreview } from "../lib/policy";
import { ArgumentPreview } from "./ArgumentPreview";
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

/** One labelled field of a held approval.
 *
 * A value of [`NOT_RECORDED`] is dimmed further and never styled like a
 * figure: the operator deciding this hold has to be able to see at a glance
 * which of these the plane actually told us. */
function Field({ label, value, title }: { label: string; value: string; title?: string }) {
  const absent = value === NOT_RECORDED || value === NO_CALL_ON_HOLD;
  return (
    <div className="flex items-baseline gap-2 min-w-0">
      <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
        {label}
      </span>
      <span
        className={`truncate text-[11.5px]${absent ? "" : " mono tabular"}`}
        style={{ color: absent ? "var(--faint)" : "var(--dim)", fontStyle: absent ? "italic" : undefined }}
        title={title ?? value}
      >
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

/** What the copy button is showing. A failed copy says so: the operator has one
 * chance at this token, and a button that looks done when it is not loses it. */
type CopyState = "idle" | "copied" | "failed";

/**
 * Shown once, right after a Grant: the raw token, which is what reaches the
 * agent, and the decoded claims beside it (PHASE2.md Wave 2 - "show the operator
 * exactly what they authorized"). Wardryx never lets the token be retrieved
 * again, so it stays on screen until the operator dismisses it, independent of
 * the approval's own row moving to history underneath.
 *
 * The token is held by this component's parent in plain state and rendered here
 * as text: `lib/approvalToken.ts` says why nothing else may hold it. Once it has
 * expired it is no use to anybody and is no longer drawn.
 *
 * `onCopied` tells {@link GrantedTokenList} the operator has the token, by the
 * button or by copying the text by hand, which lets the page go without asking.
 * `panelRef` is how the list brings this panel into view and focuses it.
 */
export function GrantedTokenPanel({
  granted,
  onDismiss,
  onCopied,
  panelRef,
}: {
  granted: GrantedToken;
  onDismiss: () => void;
  onCopied?: () => void;
  panelRef?: (panel: HTMLDivElement | null) => void;
}) {
  const { claims, raw } = granted;
  const countdown = useCountdown(claims?.exp_unix ?? 0);
  const [copy, setCopy] = useState<CopyState>("idle");

  const expired = claims !== null && countdown.remainingMs <= 0;

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(raw);
      setCopy("copied");
      onCopied?.();
    } catch {
      // No clipboard outside a secure context, or the browser refused it.
      setCopy("failed");
    }
  };

  return (
    <div
      ref={panelRef}
      tabIndex={-1}
      role="region"
      aria-label={`Approval token for ${granted.approvalId}, shown once`}
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
      {!expired && (
        <div className="flex flex-col gap-1">
          <span className="text-[11px]" style={{ color: "var(--dim)" }}>
            The agent retries the call with this value in the header{" "}
            <code className="mono" style={{ color: "var(--fg)" }}>
              {APPROVAL_TOKEN_HEADER}
            </code>
          </span>
          <div className="flex items-start gap-2">
            <code
              data-testid="approval-token"
              className="mono text-[11.5px] min-w-0 flex-1"
              style={{ color: "var(--fg)", wordBreak: "break-all", userSelect: "all" }}
              onCopy={() => onCopied?.()}
            >
              {raw}
            </code>
            <button
              type="button"
              className="icon-btn"
              style={{ width: "auto", padding: "0 8px", fontSize: 11, whiteSpace: "nowrap" }}
              aria-label="Copy the approval token"
              onClick={() => void onCopy()}
            >
              {copy === "copied" ? "Copied" : "Copy"}
            </button>
          </div>
          {copy === "failed" && (
            <span className="text-[11px]" style={{ color: "var(--sev-high)" }} role="alert">
              The browser refused the copy. Select the token above and copy it by hand.
            </span>
          )}
        </div>
      )}
      {claims !== null && (
        <div className="flex flex-wrap gap-x-5 gap-y-1.5">
          <Field label="agent" value={claims.agent_id} />
          <Field label="run" value={claims.run_id} />
          <Field label="tools" value={claims.tools.length > 0 ? claims.tools.join(", ") : "(none)"} />
        </div>
      )}
      {claims !== null && (
        <div className="flex items-center gap-5">
          <span className="mono tabular text-[13px]" style={{ color: "var(--fg)" }}>
            ceiling {formatUsd(claims.cost_ceiling_usd)}
          </span>
          <span
            className="mono tabular text-[13px]"
            style={{ color: expired ? "var(--sev-critical)" : "var(--fg)" }}
          >
            {expired ? "expired" : `expires in ${countdown.label}`}
          </span>
        </div>
      )}
      <span className="text-[12px]" style={{ color: "var(--fg)" }}>
        {tokenPlainLine(claims)}
      </span>
      <span className="text-[11px]" style={{ color: "var(--faint)" }}>
        {SINGLE_USE_EXCEPTION}
      </span>
    </div>
  );
}

/** The clock a list of tokens reads expiry from. It ticks each second only while
 * there is a token to read it for, and reads the time afresh when one arrives. */
function useNowMs(ticking: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!ticking) return;
    setNow(Date.now());
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [ticking]);
  return now;
}

/**
 * Every token a view holds, made impossible to miss. A grant used to put its
 * panel at the top of the inbox, above whatever part of the page was in view,
 * with nothing that said it was there, and a reload lost it for good: on
 * 2026-11-12 two grants in the lab were spent that way before a third was copied.
 *
 * So a token that arrives is brought into view and given the focus; a notice
 * fixed to the window says a token is on screen for as long as one is alive, and
 * brings it back into view; and while one is alive and not copied, leaving the
 * page or the view asks first (`lib/leaveGuard.ts`). Expired, copied (by the
 * button or by hand) or dismissed, it holds nothing.
 *
 * The tokens stay where they were: in the parent's state, passed down here. This
 * keeps only which ones were copied and which were already brought into view, by
 * approval id, and the notice names no part of a token.
 */
export function GrantedTokenList({
  tokens,
  onDismiss,
}: {
  tokens: readonly GrantedToken[];
  onDismiss: (approvalId: string) => void;
}) {
  const now = useNowMs(tokens.length > 0);
  const [copied, setCopied] = useState<ReadonlySet<string>>(() => new Set());
  const panels = useRef(new Map<string, HTMLDivElement>());
  const shown = useRef(new Set<string>());

  const alive = tokens.filter((g) => !isExpired(g, now));
  const notCopied = alive.filter((g) => !copied.has(g.approvalId));
  useLeaveGuard(notCopied.length > 0);

  const bringIntoView = useCallback((approvalId: string) => {
    const panel = panels.current.get(approvalId);
    if (!panel) return;
    // jsdom has no scrollIntoView; a browser always does.
    panel.scrollIntoView?.({ behavior: "smooth", block: "center" });
    panel.focus({ preventScroll: true });
  }, []);

  useEffect(() => {
    const fresh = tokens.find((g) => !shown.current.has(g.approvalId));
    for (const g of tokens) shown.current.add(g.approvalId);
    if (fresh) bringIntoView(fresh.approvalId);
  }, [tokens, bringIntoView]);

  const markCopied = useCallback((approvalId: string) => {
    setCopied((set) => (set.has(approvalId) ? set : new Set(set).add(approvalId)));
  }, []);

  const pointAt = notCopied[0] ?? alive[0];

  return (
    <>
      {tokens.map((granted) => (
        <GrantedTokenPanel
          key={granted.approvalId}
          granted={granted}
          onDismiss={() => onDismiss(granted.approvalId)}
          onCopied={() => markCopied(granted.approvalId)}
          panelRef={(panel) => {
            if (panel) panels.current.set(granted.approvalId, panel);
            else panels.current.delete(granted.approvalId);
          }}
        />
      ))}
      {pointAt !== undefined &&
        createPortal(
          <div
            data-testid="granted-token-notice"
            role="status"
            className="panel px-3 py-2 flex items-center gap-3"
            style={{
              position: "fixed",
              left: "50%",
              bottom: 16,
              transform: "translateX(-50%)",
              zIndex: 60,
              width: "max-content",
              maxWidth: "calc(100vw - 32px)",
              background: "color-mix(in srgb, var(--sev-low) 16%, var(--panel-2))",
              borderColor: "color-mix(in srgb, var(--sev-low) 60%, var(--line-2))",
              boxShadow: "0 8px 24px rgba(0, 0, 0, 0.35)",
            }}
          >
            <span className="badge" style={cssVar("tone", "var(--sev-low)")}>
              token on screen
            </span>
            <span className="text-[12px]" style={{ color: "var(--fg)" }}>
              {grantedTokenNotice(notCopied.length, alive.length)}
            </span>
            <button
              type="button"
              className="icon-btn"
              style={{ width: "auto", padding: "0 8px", fontSize: 11, whiteSpace: "nowrap" }}
              onClick={() => bringIntoView(pointAt.approvalId)}
            >
              Show the token
            </button>
          </div>,
          document.body,
        )}
    </>
  );
}

/** The always-allow button of one held row, quieter than Grant on purpose: Grant is
 * the answer to this call, and this one outlives it. Off, it stays on the row, so
 * the line under it has something to be about. */
function AlwaysAllowButton({
  offer,
  onAlwaysAllow,
  approvalId,
}: {
  offer: StandingGrantOffer;
  onAlwaysAllow: (id: string) => Promise<void>;
  approvalId: string;
}) {
  if (offer.kind === "none" || offer.kind === "granted") return null;
  return (
    <ConfirmButton
      label="Always allow"
      confirmLabel={offer.kind === "ready" ? `Confirm: always allow ${offer.tool}` : undefined}
      tone="var(--dim)"
      disabled={offer.kind !== "ready"}
      onConfirm={() => onAlwaysAllow(approvalId)}
    />
  );
}

/** What a held row says under its buttons about a standing grant, if anything. */
function standingGrantLine(offer: StandingGrantOffer): string | null {
  switch (offer.kind) {
    case "blocked":
      return offer.line;
    case "granted":
      return `This agent always holds ${offer.tool} now, so its next call passes. This call still waits for an answer.`;
    case "ready":
      return offer.staticKey ? "This agent called with a static key, so Always allow takes one too." : null;
    case "none":
      return null;
  }
}

/** One agent and tool a person keeps granting, offered as a standing grant. */
function RepeatOfferRow({ offer, onAlwaysAllow }: { offer: RepeatOffer; onAlwaysAllow: (id: string) => Promise<void> }) {
  return (
    <div className="panel px-3 py-2.5 flex items-center gap-3" style={{ background: "var(--panel-2)" }}>
      <span className="badge" style={cssVar("tone", "var(--sev-low)")}>
        repeat
      </span>
      <span className="min-w-0 flex-1 text-[11.5px]" style={{ color: "var(--dim)" }}>
        <span className="mono" style={{ color: "var(--fg)" }}>
          {offer.agent_id}
        </span>{" "}
        was granted{" "}
        <span className="mono" style={{ color: "var(--fg)" }}>
          {offer.tool}
        </span>{" "}
        {offer.grants} times. Always allow it?
      </span>
      <ConfirmButton
        label="Always allow"
        confirmLabel={`Confirm: always allow ${offer.tool}`}
        tone="var(--dim)"
        onConfirm={() => onAlwaysAllow(offer.approval_id)}
      />
    </div>
  );
}

/** What the Grant button of a held row may do (CLAUDE.md invariant 48). A grant
 * of a call whose preview was sealed waits until the person opened it; a call
 * whose arguments could not be shown is granted blind, said so on the button
 * and in the record; a policy may forbid that. */
export type GrantGate =
  | { kind: "grant"; label: string; confirm: string }
  | { kind: "blocked"; label: string; why: string };

export function grantGate(approval: Approval, opened: ApprovalPreview | null): GrantGate {
  const state = approval.preview?.state;
  const blindStates = ["truncated", "off", "unavailable"];
  if (state === "sealed") {
    return opened?.pvd
      ? { kind: "grant", label: "Grant", confirm: "Confirm grant of the call shown" }
      : { kind: "blocked", label: "Grant", why: "Open the arguments first: a grant names the preview the person saw." };
  }
  if (state !== undefined && blindStates.includes(state) && approval.tool_call !== null) {
    if (approval.require_preview) {
      return {
        kind: "blocked",
        label: "Grant",
        why: "A policy requires that a person saw the arguments before a grant, and they could not be shown.",
      };
    }
    return { kind: "grant", label: "Grant unseen", confirm: "Confirm grant without seeing the arguments" };
  }
  return { kind: "grant", label: "Grant", confirm: "Confirm grant" };
}

function PendingRow({
  approval,
  offer,
  onDecide,
  onAlwaysAllow,
  highlighted,
  muted,
  onToggleMuteAgent,
  onOpenAgent,
}: {
  approval: Approval;
  offer: StandingGrantOffer;
  onDecide: (id: string, decision: Decision, seenPvd: string | null) => Promise<void>;
  onAlwaysAllow: (id: string) => Promise<void>;
  highlighted: boolean;
  muted: boolean;
  onToggleMuteAgent: (agentId: string) => void;
  onOpenAgent: (agentId: string) => void;
}) {
  const chain = approval.on_behalf_of;
  const grantLine = standingGrantLine(offer);
  // What the person was shown of the call (CLAUDE.md invariant 48): held here,
  // while this row is on screen, and dropped with it.
  const [opened, setOpened] = useState<ApprovalPreview | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);
  const gate = grantGate(approval, opened);
  const openPreview = async () => {
    setOpening(true);
    setOpenError(null);
    try {
      setOpened(await fetchApprovalPreview(approval.approval_id));
    } catch (err) {
      const e = err as { reason?: string; message?: string };
      setOpenError(e.reason ?? e.message ?? "the preview could not be opened");
    } finally {
      setOpening(false);
    }
  };
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

      {/* `model` and `org` reach this row on every hold and rendered nowhere
          until now, which is how a field ends up on the wire "for display
          only" (wardryx.rs) and never displayed. The model is what the
          operator is being asked to spend money on; the org is stamped from
          the authenticated principal that triggered the hold, so it is the
          one field here the requester could not have chosen. Both are shown
          as an explicit absence rather than a dash, because a dash beside
          fourteen other dashes reads as a layout, not as an answer. */}
      <div className="flex flex-wrap gap-x-5 gap-y-1.5">
        <Field label="run" value={approval.run_id} />
        <Field label="tools" value={approval.tool_names.length > 0 ? approval.tool_names.join(", ") : "-"} />
        <Field label="target" value={approvalTargetLabel(approval)} />
        <Field label="arguments" value={approvalArgumentsLabel(approval)} title={approvalArgumentsTitle(approval)} />
        <Field
          label="model"
          value={approvalModelLabel(approval)}
          title={
            approval.model
              ? `Model the held action declared: ${approval.model}. Carried for display; the policy engine does not branch on it.`
              : "This hold carried no model in its context."
          }
        />
        <Field
          label="org"
          value={approvalOrgLabel(approval)}
          title={
            approval.org
              ? `Stamped from the authenticated principal that triggered the hold, not from the request body.`
              : "This hold carried no org in its context."
          }
        />
        <Field
          label="policy_version"
          value={approvalPolicyVersionLabel(approval)}
          title={
            approval.policy_version
              ? `The policy set generation that decided this hold.`
              : "This hold carried no policy version in its context."
          }
        />
        {chain && chain.length > 0 && <Field label="on_behalf_of" value={chain.join(" -> ")} />}
      </div>

      <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
        {approvalReasonLabel(approval)}
      </span>

      {grantLine && (
        <span className="text-[11px]" style={{ color: "var(--faint)" }}>
          {grantLine}
        </span>
      )}

      {approval.preview?.state === "sealed" && opened === null && (
        <div className="flex items-center gap-2">
          <button type="button" className="icon-btn" disabled={opening} onClick={() => void openPreview()}>
            {opening ? "Opening..." : "Show arguments"}
          </button>
          {openError && (
            <span className="text-[11px]" style={{ color: "var(--sev-high)" }}>
              {visibleText(openError)}
            </span>
          )}
        </div>
      )}
      {opened?.document && <ArgumentPreview document={opened.document} pvd={opened.pvd} />}

      <div className="flex items-center gap-2 justify-end">
        <AlwaysAllowButton offer={offer} onAlwaysAllow={onAlwaysAllow} approvalId={approval.approval_id} />
        {gate.kind === "blocked" ? (
          <button type="button" className="icon-btn" disabled title={gate.why}>
            {gate.label}
          </button>
        ) : (
          <ConfirmButton
            label={gate.label}
            confirmLabel={gate.confirm}
            tone="var(--sev-low)"
            onConfirm={() => onDecide(approval.approval_id, "grant", opened?.pvd ?? null)}
          />
        )}
        <ConfirmButton
          label="Deny"
          confirmLabel="Confirm deny"
          tone="var(--sev-critical)"
          onConfirm={() => onDecide(approval.approval_id, "deny", opened?.pvd ?? null)}
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
        {/* The model rides on a decided hold exactly as it does on a pending
            one, and the trail is the half somebody reads back later. */}
        <span className="mono truncate text-[11px]" style={{ color: "var(--faint)" }}>
          {approval.decided_by ?? "unknown"} &middot; {approval.decided_at ? formatTimestamp(approval.decided_at) : "-"}{" "}
          &middot; {approvalModelLabel(approval)}
        </span>
        {/* What was decided on stays readable after the decision: the trail is
            the half somebody reads back later. */}
        <span className="truncate text-[11px]" style={{ color: "var(--dim)" }} title={approvalReasonLabel(approval)}>
          {approvalReasonLabel(approval)}
        </span>
        <span className="mono truncate text-[11px]" style={{ color: "var(--faint)" }} title={approvalArgumentsTitle(approval)}>
          target {approvalTargetLabel(approval)} &middot; arguments {approvalArgumentsLabel(approval)}
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
 * through an explicit `ConfirmButton` ceremony (a passkey ceremony is the
 * hardware-confirmed form, see `PasskeySettings`),
 * and a history list of already-decided approvals underneath.
 *
 * Wave 3 additions (docs/PHASE2.md "Actionable notifications"): `focusApprovalId`
 * scrolls to and briefly highlights the matching row - the working half of
 * the notification deep link (`lib/notifications.ts`'s doc comment explains
 * why this in-app scroll-to, rather than a real OS notification-click
 * callback, is what actually fires in the browser). `mutedKeys`/
 * `onToggleMuteAgent` back the per-row mute toggle that keys
 * `useApprovalNotifications`'s mute set (`lib/notifications.ts`'s
 * `muteKey`/`isMuted` composite-key format, shared verbatim - never a
 * second, differently-shaped "muted" collection) - muting only ever affects
 * whether a FUTURE `approval_requested` raises a notification, never this
 * inbox's own contents or the Grant/Deny path.
 */
export function ApprovalsInbox({
  approvals,
  policies,
  role,
  onDecide,
  onAlwaysAllow,
  grantedTokens,
  onDismissToken,
  focusApprovalId,
  mutedKeys,
  onToggleMuteAgent,
  onOpenAgent,
}: {
  approvals: Approval[];
  /** The stored policies, so a row does not offer what the agent already holds. */
  policies: readonly PolicyRecord[];
  /** The signed-in role, `null` when the console has no session to read. */
  role: ConsoleRole | null | undefined;
  onDecide: (id: string, decision: Decision, seenPvd: string | null) => Promise<void>;
  /** A standing grant (`lib/standingGrant.ts`), after the same ceremony as a grant. */
  onAlwaysAllow: (id: string) => Promise<void>;
  grantedTokens: readonly GrantedToken[];
  onDismissToken: (approvalId: string) => void;
  focusApprovalId: string | null;
  mutedKeys: ReadonlySet<string>;
  onToggleMuteAgent: (agentId: string) => void;
  /** Phase-3 wave-3 deep link (docs/PHASE3.md W3): opens the Agent 360 card
   * for a row's `agent_id`, both pending and decided. */
  onOpenAgent: (agentId: string) => void;
}) {
  const pending = approvals.filter((a) => a.pending);
  const history = approvals.filter((a) => !a.pending);
  const offers = repeatOffers(approvals, policies, role);
  const mutedAgentIds = Array.from(mutedKeys)
    .map(agentIdFromMuteKey)
    .filter((id): id is string => id !== null);

  useEffect(() => {
    if (!focusApprovalId) return;
    document.getElementById(`approval-${focusApprovalId}`)?.scrollIntoView({ behavior: "smooth", block: "center" });
  }, [focusApprovalId, approvals]);

  return (
    <div className="flex flex-col gap-3 px-5">
      <GrantedTokenList tokens={grantedTokens} onDismiss={onDismissToken} />

      <MutedAgentsStrip mutedAgents={mutedAgentIds} onUnmute={onToggleMuteAgent} />

      {offers.map((offer) => (
        <RepeatOfferRow key={`${offer.agent_id} ${offer.tool}`} offer={offer} onAlwaysAllow={onAlwaysAllow} />
      ))}

      {pending.length === 0 ? (
        <div className="py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no pending approvals.
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          {pending.map((a) => (
            <PendingRow
              key={a.approval_id}
              approval={a}
              offer={standingGrantOffer(a, policies, role)}
              onDecide={onDecide}
              onAlwaysAllow={onAlwaysAllow}
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
