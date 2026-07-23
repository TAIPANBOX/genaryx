import { useCallback, useEffect, useRef, useState } from "react";
import type {
  CopilotAnswer,
  CopilotExplainRequest,
  CopilotStatus,
  CopilotToolInvocation,
  ProposedAction,
  ProposedActionKind,
} from "../copilotTypes";
import {
  askCopilot,
  describeCopilotError,
  explainIncident,
  fetchCopilotStatus,
  logProposalApproved,
} from "../lib/copilot";
import { cssVar } from "../lib/cssVars";
import { formatUsd } from "../lib/format";
import { describeIdentityError, rescan } from "../lib/identity";
import { describeMoneyError, killRun, setBudget } from "../lib/money";
import { decideApproval, describePolicyError } from "../lib/policy";
import type { IdentityError } from "../identityTypes";
import type { MoneyError } from "../moneyTypes";
import type { Decision, PolicyError } from "../policyTypes";
import { ConfirmButton } from "./ConfirmButton";
import { usePopover, PopoverHeader } from "../lib/popover";
import { FelyxConnectCard } from "./FelyxConnectCard";

const FIELD_STYLE = {
  background: "var(--panel)",
  border: "1px solid var(--line-2)",
  borderRadius: 8,
  padding: "9px 12px",
  fontSize: 12.5,
  color: "var(--fg)",
} as const;

/** One proposal card's local lifecycle (C2, docs/PHASE6-C2.md) - `action` is
 * exactly what the crate proposed; `status` starts `"pending"` and moves to
 * `"approved"`/`"dismissed"` once the operator acts on THIS card, so a
 * decided card never re-shows its Approve/Dismiss buttons even though the
 * rest of the transcript (and this message's OTHER proposals, if any) stay
 * put. Local-only, like the rest of this panel's state - there is no
 * `copilot_history` command (see this file's own module doc comment below). */
interface ProposalState {
  action: ProposedAction;
  status: "pending" | "approved" | "dismissed";
}

interface ChatMessage {
  id: number;
  role: "user" | "assistant";
  text: string;
  /** Present only on a successful assistant answer - the evidence surface
   * `ToolTraceSection` renders. `undefined` (not an empty array) for a user
   * message or an error note, so the collapsible section never appears on
   * either. */
  toolTrace?: CopilotToolInvocation[];
  /** Present only on a successful assistant answer that PROPOSED at least
   * one action (C2) - `ProposalCard` renders each as an approve/dismiss
   * card. `undefined` (not an empty array), same convention as `toolTrace`
   * (see [`toProposalState`]). */
  proposals?: ProposalState[];
  /** Set when this "assistant" message is actually `copilot_ask`'s
   * rejection rendered inline (e.g. `CopilotError::NoProvider`'s message) -
   * an honest note, never a crash, styled distinctly from a real answer. */
  isNote?: boolean;
}

/** `answer.proposals` -> this view's own per-card `ProposalState[]`, or
 * `undefined` for an answer that proposed nothing - same "`undefined`, not
 * an empty array" convention `ChatMessage.toolTrace` already follows, so
 * `MessageBubble`'s own `.length > 0` guard never has to special-case an
 * empty array either. Every proposal starts `"pending"`. */
function toProposalState(proposals: ProposedAction[]): ProposalState[] | undefined {
  return proposals.length > 0 ? proposals.map((action) => ({ action, status: "pending" as const })) : undefined;
}

/** `action.target` rendered for display: a `rescan` proposal may carry no
 * specific target (`propose_rescan`'s `target` argument is optional on the
 * crate side - `crates/copilot/src/action.rs`), which still round-trips as
 * `""` here (the field itself is a plain `String`, never `Option`) rather
 * than being omitted. */
function proposalTargetLabel(action: ProposedAction): string {
  return action.target.trim().length > 0 ? action.target : "(fleet-wide)";
}

/** `"Kill run"` / `"Cap budget"` / `"Grant approval"` / `"Deny approval"` /
 * `"Rescan"` - the clear-verb label a proposal card's header uses (spec:
 * "the kind (as a clear verb)"). `grant_deny` reads its own `params.verdict`
 * rather than one generic label for both directions, since granting and
 * denying are opposite operator decisions and must never look the same at a
 * glance. */
function proposalVerb(action: ProposedAction): string {
  switch (action.kind) {
    case "kill":
      return "Kill run";
    case "budget":
      return "Cap budget";
    case "grant_deny":
      return action.params.verdict === "deny" ? "Deny approval" : "Grant approval";
    case "rescan":
      return "Rescan";
  }
}

/** A short "$5 cap" / "verdict: deny" line under a card's header, or `null`
 * when this kind's params have nothing worth a dedicated summary line
 * (`kill`/`rescan` - the target and rationale already say everything). */
function proposalParamsSummary(action: ProposedAction): string | null {
  if (action.kind === "budget") {
    const usdCap = Number(action.params.usd_cap);
    return Number.isFinite(usdCap) ? `${formatUsd(usdCap)} cap` : null;
  }
  if (action.kind === "grant_deny") {
    const verdict = action.params.verdict;
    return typeof verdict === "string" ? `verdict: ${verdict}` : null;
  }
  return null;
}

/** Detail line inside the BREAK-GLASS OVERRIDE modal `ConfirmButton` opens
 * for `kill`/`budget` - mirrors `RunsBoard.tsx`/`BudgetEditor.tsx`'s own
 * `breakGlassDetail` strings exactly, so approving a proposal's ceremony
 * reads identically to a manual click's. `undefined` for `grant_deny`/
 * `rescan`, which never open that modal at all (see `ProposalCard`). */
function breakGlassDetailFor(action: ProposedAction): string | undefined {
  if (action.kind === "kill") return `run ${proposalTargetLabel(action)}`;
  if (action.kind === "budget") {
    const usdCap = Number(action.params.usd_cap);
    return Number.isFinite(usdCap)
      ? `run ${proposalTargetLabel(action)} -> ${formatUsd(usdCap)}`
      : `run ${proposalTargetLabel(action)}`;
  }
  return undefined;
}

/** Human-readable text for whatever `runApproval` rejected with. Each kind
 * routes through a DIFFERENT existing command with its own structured error
 * type (`MoneyError` for kill/budget, `PolicyError` for grant_deny,
 * `IdentityError` for rescan) - `killRun`/`setBudget`/`decideApproval`/
 * `rescan` all normalize a rejection into that exact shape before it ever
 * reaches here (e.g. `lib/money.ts`'s `toMoneyError`), so branching on
 * `kind` to pick the matching `describeXError` is always correct. The one
 * exception is `runApproval`'s own pre-flight `usd_cap` validation, which
 * throws a plain `Error` - `instanceof Error` catches that uniformly before
 * the kind-specific branches ever run. */
function describeApprovalError(kind: ProposedActionKind, err: unknown): string {
  if (err instanceof Error) return err.message;
  if (kind === "kill" || kind === "budget") return describeMoneyError(err as MoneyError);
  if (kind === "grant_deny") return describePolicyError(err as PolicyError);
  return describeIdentityError(err as IdentityError);
}

/**
 * Route an approved proposal into the EXISTING signed mutation its `kind`
 * maps to (C2, docs/PHASE6-C2.md: "Approve routes into the EXISTING signed
 * ceremony... REUSE that code - do not reimplement signing") and return a
 * one-line summary for the transcript. The copilot crate never calls any of
 * these itself (`crates/copilot/src/action.rs`'s doc comment: "There is
 * deliberately no `Act` here") - this is the ONE place in the shell that
 * turns a `ProposedAction` into a real call, and it is always the exact SAME
 * call a manual click elsewhere in this app already makes:
 * - `kill` -> [`killRun`] (`src/lib/money.ts`) -> `money_kill_run`
 *   (`src-tauri/src/money/commands.rs`), the signed `POST /v1/runs/{id}/kill`
 *   `RunsBoard.tsx`'s own Kill button already triggers.
 * - `budget` -> [`setBudget`] -> `money_set_budget`, the same signed
 *   `POST /v1/runs/{id}/budget` `BudgetEditor.tsx` already triggers.
 * - `grant_deny` -> [`decideApproval`] -> `policy_decide_approval`, the same
 *   Wardryx decide call `ApprovalsInbox.tsx`'s Grant/Deny buttons already
 *   trigger.
 * - `rescan` -> [`rescan`] -> `identity_rescan`, the same `idryx detect`
 *   batch call `IdentityView.tsx`'s Rescan button already triggers.
 *
 * `reason` is the break-glass justification `ProposalCard`'s `ConfirmButton`
 * collects for `kill`/`budget` only (`""` for `grant_deny`/`rescan`, which
 * use the non-break-glass ceremony and ignore it, exactly like
 * `ApprovalsInbox.tsx`'s Grant/Deny buttons already do).
 */
async function runApproval(action: ProposedAction, reason: string): Promise<string> {
  switch (action.kind) {
    case "kill": {
      const outcome = await killRun(action.target, reason);
      return outcome.summary;
    }
    case "budget": {
      const usdCap = Number(action.params.usd_cap);
      if (!Number.isFinite(usdCap)) {
        throw new Error(`proposal has no usable "usd_cap" in params (got ${JSON.stringify(action.params)})`);
      }
      const outcome = await setBudget(action.target, usdCap, reason);
      return outcome.summary;
    }
    case "grant_deny": {
      const decision: Decision = action.params.verdict === "deny" ? "deny" : "grant";
      const outcome = await decideApproval(action.target, decision);
      return outcome.summary;
    }
    case "rescan": {
      const alerts = await rescan();
      return `rescan complete - ${alerts.length} alert${alerts.length === 1 ? "" : "s"} found`;
    }
    default: {
      const exhaustive: never = action.kind;
      throw new Error(`unknown proposal kind: ${exhaustive}`);
    }
  }
}

/**
 * The residency banner (Phase 6, C0 - docs/PHASE6.md, itrat-console/13
 * D13.2): the one thing every screen of this panel must make impossible to
 * miss - whether a question just typed here can leave this machine at all.
 * Three honest states, never blended: still checking, no provider
 * configured (muted), a local provider (mint - "nothing leaves this box"),
 * or a remote BYO-key provider (amber - a deliberate, explicit opt-in per
 * `genaryx-copilot`'s residency gate, `crates/copilot/src/residency.rs`).
 */
function ConnectButton({ label, onConnect }: { label: string; onConnect: () => void }) {
  return (
    <button
      type="button"
      onClick={onConnect}
      className="text-[12px]"
      style={{
        padding: "5px 12px",
        borderRadius: 8,
        cursor: "pointer",
        border: "1px solid var(--iris)",
        background: "color-mix(in srgb, var(--iris) 16%, transparent)",
        color: "var(--fg)",
        whiteSpace: "nowrap",
      }}
    >
      {label}
    </button>
  );
}

function ResidencyBanner({ status, onConnect }: { status: CopilotStatus | null; onConnect: () => void }) {
  if (!status) {
    return (
      <div className="d-card px-4 py-3 mono" style={{ fontSize: 12, color: "var(--faint)" }}>
        checking copilot status...
      </div>
    );
  }

  if (!status.enabled) {
    return (
      <div className="d-card px-4 py-3 flex items-center gap-2.5">
        <span
          aria-hidden="true"
          style={{ width: 8, height: 8, borderRadius: "50%", background: "var(--faint)", flex: "0 0 auto" }}
        />
        <span className="text-[12.5px]" style={{ color: "var(--dim)" }}>
          No provider configured{status.disabled_reason ? ` - ${status.disabled_reason}` : ""}
        </span>
        <div className="flex-1" />
        <ConnectButton label="Connect Felyx" onConnect={onConnect} />
      </div>
    );
  }

  const local = status.local === true;
  const tone = local ? "var(--mint)" : "var(--amber)";
  return (
    <div
      className="d-card px-4 py-3 flex items-center gap-2.5"
      style={{ borderColor: `color-mix(in srgb, ${tone} 30%, var(--line))` }}
    >
      <span
        aria-hidden="true"
        style={{
          width: 8,
          height: 8,
          borderRadius: "50%",
          background: tone,
          boxShadow: `0 0 8px ${tone}`,
          flex: "0 0 auto",
        }}
      />
      <span className="mono text-[12.5px]" style={{ color: tone }}>
        {local
          ? `Local: ${status.model ?? "unknown model"} via ${status.provider ?? "unknown provider"} on this machine`
          : `Remote: ${status.provider ?? "unknown provider"} (BYO key)`}
      </span>
      <div className="flex-1" />
      <ConnectButton label="Change" onConnect={onConnect} />
    </div>
  );
}

/** The evidence surface (D13.6): every tool Felyx's loop actually ran for
 * this answer, collapsed by default so a plain-text answer stays the focus,
 * one click away for an operator who wants to check the numbers behind it. */
function ToolTraceSection({ trace }: { trace: CopilotToolInvocation[] }) {
  if (trace.length === 0) return null;
  return (
    <details className="mt-2">
      <summary
        className="mono"
        style={{
          fontSize: 10.5,
          letterSpacing: "0.07em",
          textTransform: "uppercase",
          color: "var(--faint)",
          cursor: "pointer",
        }}
      >
        tools used ({trace.length})
      </summary>
      <div className="flex flex-col gap-1.5 mt-2">
        {trace.map((t, idx) => (
          <div key={`${t.name}-${idx}`} className="flex items-start gap-2" style={{ fontSize: 11 }}>
            <span className="badge" style={cssVar("tone", t.ok ? "var(--sev-low)" : "var(--sev-high)")}>
              {t.ok ? "ok" : "fail"}
            </span>
            <span className="mono" style={{ color: "var(--fg)", flex: "0 0 auto", whiteSpace: "nowrap" }}>
              {t.name}
            </span>
            <span className="mono truncate" style={{ color: "var(--faint)" }} title={t.result_preview}>
              {t.result_preview}
            </span>
          </div>
        ))}
      </div>
    </details>
  );
}

/** `NN% confidence` badge, toned mint/amber/faint by how confident the model
 * claims to be - never a bare number with no visual weight, so a
 * low-confidence proposal reads as less certain at a glance, not just as a
 * smaller digit. */
function ConfidenceChip({ confidence }: { confidence: number }) {
  const pct = Math.round(Math.max(0, Math.min(1, confidence)) * 100);
  const tone = pct >= 75 ? "var(--mint)" : pct >= 45 ? "var(--amber)" : "var(--faint)";
  return (
    <span className="badge" style={cssVar("tone", tone)}>
      {pct}% confidence
    </span>
  );
}

/**
 * One `ProposedAction` rendered as an approve/dismiss card (C2,
 * docs/PHASE6-C2.md): the kind as a clear verb, the target, a short params
 * summary, the rationale, a confidence chip, the evidence refs (monospace,
 * verbatim - the anti-hallucination surface D13.6 calls for), and - only
 * when Wardryx's C2 pre-check actually found one - a muted "Governed by
 * policy" line.
 *
 * Approve reuses `ConfirmButton` exactly as the panel this proposal targets
 * already does: `breakGlass` for `kill`/`budget` (the two
 * genuinely-privileged Cloud-state overrides, same modal ceremony
 * `RunsBoard.tsx`/`BudgetEditor.tsx` already use, right down to the
 * justification it collects), the plain inline confirm for `grant_deny`/
 * `rescan` (same ceremony `ApprovalsInbox.tsx`'s Grant/Deny buttons already
 * use). Dismiss never calls anything at all - it only drops this card's own
 * local state (`onDismiss`); the copilot only proposed, so there is no
 * queued action anywhere to cancel.
 */
function ProposalCard({
  proposal,
  onApprove,
  onDismiss,
}: {
  proposal: ProposalState;
  onApprove: (reason: string) => Promise<void>;
  onDismiss: () => void;
}) {
  const { action, status } = proposal;
  const breakGlass = action.kind === "kill" || action.kind === "budget";
  const confirmLabel =
    action.kind === "kill" ? "Confirm kill" : action.kind === "budget" ? "Confirm budget" : "Confirm approve";
  const paramsSummary = proposalParamsSummary(action);

  return (
    <div
      className="d-card px-3.5 py-3 flex flex-col gap-2"
      style={{
        marginTop: 8,
        background: "var(--panel-2)",
        borderColor: "color-mix(in srgb, var(--iris) 30%, var(--line))",
      }}
    >
      <div className="flex items-center gap-2 flex-wrap">
        <span className="badge" style={cssVar("tone", "var(--iris)")}>
          proposal
        </span>
        <span style={{ fontSize: 12.5, color: "var(--fg)", fontWeight: 600 }}>{proposalVerb(action)}</span>
        <span className="mono truncate" style={{ fontSize: 11.5, color: "var(--dim)" }} title={action.target}>
          {proposalTargetLabel(action)}
        </span>
        <div className="flex-1" />
        <ConfidenceChip confidence={action.confidence} />
      </div>

      {paramsSummary && (
        <span className="mono" style={{ fontSize: 12, color: "var(--fg)" }}>
          {paramsSummary}
        </span>
      )}

      <span style={{ fontSize: 12, color: "var(--dim)", lineHeight: 1.55 }}>{action.rationale}</span>

      {action.evidence_refs.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5">
          <span
            className="mono"
            style={{ fontSize: 10, letterSpacing: "0.06em", textTransform: "uppercase", color: "var(--faint)" }}
          >
            evidence
          </span>
          {action.evidence_refs.map((ref) => (
            <span
              key={ref}
              className="mono"
              style={{
                fontSize: 11,
                color: "var(--dim)",
                background: "var(--panel)",
                border: "1px solid var(--line-2)",
                borderRadius: 5,
                padding: "1.5px 6px",
              }}
            >
              {ref}
            </span>
          ))}
        </div>
      )}

      {action.policy_context.length > 0 && (
        <span className="text-[11px]" style={{ color: "var(--amber)" }}>
          Governed by policy: {action.policy_context.join(", ")}
        </span>
      )}

      <div className="flex items-center justify-end gap-2 mt-1">
        {status === "pending" ? (
          <>
            <button
              type="button"
              className="icon-btn"
              style={{ width: "auto", padding: "0 10px", fontSize: 11 }}
              onClick={onDismiss}
            >
              Dismiss
            </button>
            <ConfirmButton
              label="Approve"
              confirmLabel={confirmLabel}
              tone="var(--mint)"
              breakGlass={breakGlass}
              breakGlassDetail={breakGlassDetailFor(action)}
              onConfirm={onApprove}
            />
          </>
        ) : (
          <span
            className="mono"
            style={{ fontSize: 11, color: status === "approved" ? "var(--mint)" : "var(--faint)" }}
          >
            {status === "approved" ? "approved" : "dismissed"}
          </span>
        )}
      </div>
    </div>
  );
}

/** Felyx's answer lifted out of the chat into a floating card, so it can sit
 * beside the tab it is about while you read it (Yurii's ask: the answer as a
 * movable widget, not only a chat line). Same window chrome as every card. */
function FelyxAnswerCard({ message }: { message: ChatMessage }) {
  return (
    <div className="flex flex-col">
      <PopoverHeader kicker="Felyx" title="Answer" />
      <div style={{ padding: "0 16px 12px" }}>
        <div className="text-[12.5px]" style={{ color: "var(--fg)", lineHeight: 1.6, whiteSpace: "pre-wrap" }}>
          {message.text}
        </div>
      </div>
      {message.toolTrace && message.toolTrace.length > 0 && (
        <div style={{ padding: "10px 16px 14px", borderTop: "1px solid var(--line)" }}>
          <div className="mono text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)", paddingBottom: 6 }}>
            tools Felyx ran
          </div>
          {message.toolTrace.map((t, i) => (
            <div key={i} className="flex items-center gap-2 min-w-0" style={{ padding: "3px 0" }}>
              <span aria-hidden="true" style={{ width: 6, height: 6, borderRadius: "50%", background: t.ok ? "var(--mint)" : "var(--sev-high)", flexShrink: 0 }} />
              <span className="mono text-[11.5px]" style={{ color: "var(--fg)" }}>
                {t.name}
              </span>
              <span className="text-[11px] truncate" style={{ color: "var(--dim)", flex: 1 }}>
                {t.result_preview}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function MessageBubble({
  message,
  onApproveProposal,
  onDismissProposal,
}: {
  message: ChatMessage;
  onApproveProposal: (messageId: number, index: number, reason: string) => Promise<void>;
  onDismissProposal: (messageId: number, index: number) => void;
}) {
  const isUser = message.role === "user";
  const { open } = usePopover();
  return (
    <div className="flex" style={{ justifyContent: isUser ? "flex-end" : "flex-start" }}>
      <div
        className="d-card px-3.5 py-2.5"
        style={{
          maxWidth: "72%",
          background: isUser
            ? "color-mix(in srgb, var(--iris) 12%, var(--panel-2))"
            : message.isNote
              ? "color-mix(in srgb, var(--amber) 8%, var(--panel-2))"
              : "var(--panel-2)",
          borderColor: message.isNote ? "color-mix(in srgb, var(--amber) 30%, var(--line))" : "var(--line)",
        }}
      >
        <span
          className="text-[12.5px]"
          style={{ color: message.isNote ? "var(--dim)" : "var(--fg)", lineHeight: 1.6, whiteSpace: "pre-wrap" }}
        >
          {message.text}
        </span>
        {!isUser && message.proposals && message.proposals.length > 0 && (
          <div className="flex flex-col">
            {message.proposals.map((proposal, index) => (
              <ProposalCard
                key={index}
                proposal={proposal}
                onApprove={(reason) => onApproveProposal(message.id, index, reason)}
                onDismiss={() => onDismissProposal(message.id, index)}
              />
            ))}
          </div>
        )}
        {!isUser && message.toolTrace && <ToolTraceSection trace={message.toolTrace} />}
        {!isUser && !message.isNote && (
          <button
            type="button"
            onClick={(e) => open(<FelyxAnswerCard message={message} />, { anchor: e.currentTarget.getBoundingClientRect(), width: 420 })}
            className="text-[10.5px]"
            style={{ marginTop: 6, padding: "3px 8px", borderRadius: 6, cursor: "pointer", border: "1px solid var(--line-2)", background: "transparent", color: "var(--iris)" }}
          >
            Open as card ↗
          </button>
        )}
      </div>
    </div>
  );
}

/** Preview-only seed conversation (see the seeding effect below). */
const DEMO_MESSAGES: ChatMessage[] = [
  { id: -1, role: "user", text: "Which agent is the runaway, and what did it cost us?" },
  {
    id: -2,
    role: "assistant",
    text:
      "The caught runaway is sre/rca-copilot: it looped on an oversized incident trace, burned past its $1.25 per-run ceiling 26 times across shards, and tripped budget_exhausted and fanout_explosion. sre-oncall killed it break-glass; its all-time spend is $41.60. The top legitimate spender is finops/unit-economics-analyst at $77.46 (Opus, modelling unit cost), inside budget at 79% utilisation.",
    toolTrace: [
      { name: "money_incidents", ok: true, result_preview: "7 open; worst fanout_explosion x12 on rca-copilot" },
      { name: "list_runs", ok: true, result_preview: "42 runs, 1 killed, top spend $77.46" },
    ],
    proposals: [
      {
        action: {
          kind: "budget",
          target: "unit-economics-analyst-live",
          params: { usd_cap: 60 },
          rationale: "The top legitimate spender has no central cap; $60/day bounds it without blocking its weekly unit-cost run.",
          confidence: 0.72,
          evidence_refs: ["unit-economics-analyst-live"],
          policy_context: ["finops-spend-cap"],
        },
        status: "pending",
      },
    ],
  },
  { id: -3, role: "user", text: "How many approvals are pending right now?" },
  {
    id: -4,
    role: "assistant",
    text:
      "Six approvals are awaiting a human decision, all from agents whose policy requires sign-off above a cost threshold: sre/runbook-executor, sre/deploy-guard, finops/commitment-planner, finops/idle-resource-sweeper and platform/api-gateway-tuner. The oldest has waited about nine minutes. None can act until a human grants them.",
    toolTrace: [{ name: "list_approvals", ok: true, result_preview: "6 pending, oldest ~9m" }],
  },
];

/**
 * The Copilot panel (Phase 6, C0 - docs/PHASE6.md, itrat-console/13 D13): a
 * chat pane over Felyx, the read-only analyst copilot. A residency banner
 * (pinned, [`ResidencyBanner`]) always tells the operator where inference
 * runs before they type anything; a scrollable transcript below holds every
 * question and answer for this session (in-memory only - there is no
 * `copilot_history` command); a pinned composer at the bottom sends one
 * question at a time through [`askCopilot`].
 *
 * C0 ships the read path only (`crates/copilot/src/lib.rs`'s own doc
 * comment): with today's default config (`provider = "none"`, no LLM on
 * this box) every question resolves to `CopilotError::NoProvider`'s message,
 * rendered here as an assistant note rather than a toast or a crash - the
 * panel is fully usable and honest about its own C0 state without a real
 * provider ever being configured.
 *
 * C1 (docs/PHASE6-C1.md) adds `explainRequest`: an "Explain with Felyx"
 * hand-off from a sibling view (the Money panel's Incidents feed), threaded
 * down from `AppShell`'s own state - see `copilotTypes.ts`'s
 * `CopilotExplainRequest` doc comment. This view is unmounted whenever the
 * operator navigates away (`AppShell` only renders it while
 * `view === "copilot"`), so a pending request is simply picked up by the
 * effect below the moment this component (re)mounts.
 *
 * C2 (docs/PHASE6-C2.md, "Felyx propose-and-confirm") adds `proposals`: when
 * an `Answer` PROPOSES at least one action, `MessageBubble` renders each as
 * a [`ProposalCard`] below the message text - display data only, the crate
 * holds no signer (`crates/copilot/src/action.rs`'s own doc comment).
 * Approve ([`handleApproveProposal`]) never invents a new mutation path: it
 * calls [`runApproval`], which routes straight into the SAME signed command
 * a manual click on the Money/Policy/Identity panel already triggers, then
 * journals the proposal -> approval audit link via
 * [`logProposalApproved`](../lib/copilot). Dismiss
 * ([`handleDismissProposal`]) only drops the card's own local state - the
 * copilot never queued anything to cancel.
 */
export function CopilotView({
  explainRequest,
  onExplainRequestHandled,
}: {
  explainRequest: CopilotExplainRequest | null;
  onExplainRequestHandled: () => void;
}) {
  const [status, setStatus] = useState<CopilotStatus | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const nextId = useRef(0);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const { open } = usePopover();

  const refreshStatus = useCallback(() => {
    void fetchCopilotStatus().then(setStatus);
  }, []);
  const openConnect = useCallback(() => {
    open(<FelyxConnectCard onConnected={refreshStatus} />, { width: 420 });
  }, [open, refreshStatus]);

  useEffect(() => {
    refreshStatus();
  }, [refreshStatus]);

  // In the preview, once Felyx is connected, seed one short exchange so the
  // panel shows the kind of question it answers and the shape of its output
  // (text + the tools it ran + a proposal it never executes). Preview-only.
  useEffect(() => {
    if (import.meta.env.VITE_GENARYX_MOCK !== "1") return;
    if (!status?.enabled) return;
    setMessages((m) => (m.length > 0 ? m : DEMO_MESSAGES));
  }, [status?.enabled]);

  // Keep the transcript pinned to the newest message, mirroring any chat
  // surface's baseline expectation - runs after every append (a new
  // question, a new answer, or a new error note all count).
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages]);

  // "Explain with Felyx" hand-off (C1): fires once per `explainRequest.nonce`
  // - which in practice means once per mount, since the button that sets it
  // lives on a different view this component is never simultaneously
  // rendered alongside (see this component's own doc comment). Appends the
  // synthetic question first (so the transcript reads like the operator
  // asked it), then runs the exact same fetch/append/error-note shape
  // `send()` below uses, sharing `sending` so the composer disables the same
  // way for either kind of request. `cancelled` guards the state updates (not
  // the `onExplainRequestHandled()` call itself, which must always fire so a
  // later, unrelated remount of this view never re-triggers the same
  // request) in case the operator navigates away before the round trip
  // finishes.
  useEffect(() => {
    if (!explainRequest) return;
    const { incidentId } = explainRequest;
    let cancelled = false;

    setMessages((m) => [
      ...m,
      { id: nextId.current++, role: "user", text: `Explain incident \`${incidentId}\`` },
    ]);
    setSending(true);

    void (async () => {
      try {
        const answer: CopilotAnswer = await explainIncident(incidentId);
        if (!cancelled) {
          setMessages((m) => [
            ...m,
            {
              id: nextId.current++,
              role: "assistant",
              text: answer.text,
              toolTrace: answer.tool_trace,
              proposals: toProposalState(answer.proposals),
            },
          ]);
        }
      } catch (err) {
        if (!cancelled) {
          setMessages((m) => [
            ...m,
            { id: nextId.current++, role: "assistant", text: describeCopilotError(err), isNote: true },
          ]);
        }
      } finally {
        if (!cancelled) setSending(false);
        onExplainRequestHandled();
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [explainRequest, onExplainRequestHandled]);

  const send = useCallback(async () => {
    const question = input.trim();
    if (!question || sending) return;

    setMessages((m) => [...m, { id: nextId.current++, role: "user", text: question }]);
    setInput("");
    setSending(true);
    try {
      const answer: CopilotAnswer = await askCopilot(question);
      setMessages((m) => [
        ...m,
        {
          id: nextId.current++,
          role: "assistant",
          text: answer.text,
          toolTrace: answer.tool_trace,
          proposals: toProposalState(answer.proposals),
        },
      ]);
    } catch (err) {
      // e.g. CopilotError::NoProvider's message with today's default config
      // - an honest note about why there is no answer, never a crash.
      setMessages((m) => [
        ...m,
        { id: nextId.current++, role: "assistant", text: describeCopilotError(err), isNote: true },
      ]);
    } finally {
      setSending(false);
    }
  }, [input, sending]);

  // Drop a card's local state to "dismissed" - never calls anything else.
  // The copilot only proposed; there is no queued action anywhere to cancel
  // (C2, docs/PHASE6-C2.md: "A Reject/dismiss simply drops the card").
  const handleDismissProposal = useCallback((messageId: number, index: number) => {
    setMessages((m) =>
      m.map((msg) =>
        msg.id === messageId
          ? {
              ...msg,
              proposals: msg.proposals?.map((p, i) => (i === index ? { ...p, status: "dismissed" as const } : p)),
            }
          : msg,
      ),
    );
  }, []);

  // Approve one proposal card: run the SAME signed mutation a manual click
  // elsewhere in this app already runs (`runApproval`), mark the card
  // "approved" only once that call has actually succeeded (never
  // optimistically - a rejected mutation must never look approved), then
  // journal the proposal -> approval audit link (`logProposalApproved`, C2's
  // "Audit metadata") and append one transcript note reporting both outcomes
  // honestly. A failed mutation leaves the card "pending" so the operator
  // can retry - mirrors `BudgetEditor.tsx`'s "left open on failure so the
  // operator can retry" contract - and appends its own note instead
  // (`describeApprovalError`), never a crash.
  const handleApproveProposal = useCallback(
    async (messageId: number, index: number, reason: string) => {
      const proposal = messages.find((m) => m.id === messageId)?.proposals?.[index];
      if (!proposal || proposal.status !== "pending") return;
      const { action } = proposal;

      let summary: string;
      try {
        summary = await runApproval(action, reason);
      } catch (err) {
        setMessages((m) => [
          ...m,
          {
            id: nextId.current++,
            role: "assistant",
            text: `Could not approve "${proposalVerb(action)}" on ${proposalTargetLabel(action)}: ${describeApprovalError(action.kind, err)}`,
            isNote: true,
          },
        ]);
        return;
      }

      setMessages((m) =>
        m.map((msg) =>
          msg.id === messageId
            ? {
                ...msg,
                proposals: msg.proposals?.map((p, i) => (i === index ? { ...p, status: "approved" as const } : p)),
              }
            : msg,
        ),
      );

      const { journaled, journal_error } = await logProposalApproved(action.kind, action.target, action.params);

      setMessages((m) => [
        ...m,
        {
          id: nextId.current++,
          role: "assistant",
          text: journaled
            ? `Approved: ${summary} - a human approved Felyx's proposal; the signed mutation and the audit link are both recorded.`
            : `Approved: ${summary} (the proposal-approval audit link was not journaled: ${journal_error ?? "unknown reason"})`,
          isNote: true,
        },
      ]);
    },
    [messages],
  );

  return (
    <div className="flex-1 min-h-0 flex flex-col">
      <div className="px-5 pt-4 pb-2 shrink-0">
        <ResidencyBanner status={status} onConnect={openConnect} />
      </div>

      <div ref={scrollRef} className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-2 flex flex-col gap-3">
        {messages.length === 0 ? (
          <div className="flex-1 min-h-0 flex items-center justify-center">
            <span className="mono text-[12px] text-center" style={{ color: "var(--faint)", maxWidth: 420 }}>
              Ask Felyx about your agent fleet - spend, alerts, runs, approvals. Felyx can read and recommend, never
              act: any change still needs a human to approve and sign it.
            </span>
          </div>
        ) : (
          messages.map((m) => (
            <MessageBubble
              key={m.id}
              message={m}
              onApproveProposal={handleApproveProposal}
              onDismissProposal={handleDismissProposal}
            />
          ))
        )}
      </div>

      <div className="d-card mx-5 mb-4 mt-2 px-3 py-3 flex items-center gap-2 shrink-0">
        <input
          className="mono flex-1"
          style={FIELD_STYLE}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="Ask Felyx..."
          spellCheck={false}
          disabled={sending}
          onKeyDown={(e) => {
            if (e.key === "Enter") void send();
          }}
        />
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
          onClick={() => void send()}
          disabled={sending || input.trim().length === 0}
        >
          {sending ? "Asking..." : "Send"}
        </button>
      </div>
    </div>
  );
}
