/**
 * How a closed hold reads on screen.
 *
 * A hold is closed in three ways and no others (wardryx invariant 52): a person
 * decides it (`grant` or `deny`), wardryx closes it as `expired` once it has
 * waited `WARDRYX_HOLD_TTL`, or the held agent withdraws it (`withdrawn`). The
 * console used to know two outcomes and painted everything that was not a grant
 * as a refusal, so a hold nobody answered, or one its agent took back, read as
 * a person saying no.
 *
 * Neither `expired` nor `withdrawn` is a refusal by a person and neither is an
 * approval: no token was minted and the held call stays refused. They get a
 * neutral tone, never the refusal red and never the grant green. A decision
 * value this build has never heard of reads as `Closed (<value>)`, neutral too,
 * so a newer wardryx cannot make the history unrenderable.
 *
 * This is the type of a decision that can be READ. What a person can SEND is
 * `Decision` in `policyTypes.ts`, which stays `grant | deny`: nothing in the
 * console closes a hold by hand, an administrator included.
 */
import type { Approval } from "../policyTypes";
import { visibleText } from "./policyExport";

export type ApprovalOutcomeKind = "pending" | "grant" | "deny" | "expired" | "withdrawn" | "closed";

export interface ApprovalOutcome {
  kind: ApprovalOutcomeKind;
  /** What the badge says. */
  label: string;
  /** A CSS color expression for the badge's `--tone`. */
  tone: string;
}

export const EXPIRED_LABEL = "Expired, nobody decided in time";
export const WITHDRAWN_LABEL = "Withdrawn by the agent";

/** Neither the refusal red nor the grant green. */
export const NEUTRAL_TONE = "var(--dim)";

/** Longest piece of a decision value that is echoed back in a label. */
const VALUE_SHOWN = 48;

function closedWith(value: unknown): string {
  if (value === null || value === undefined || value === "") return "Closed";
  if (typeof value !== "string") return "Closed (unreadable)";
  const text = visibleText(value);
  const shown = text.length > VALUE_SHOWN ? `${text.slice(0, VALUE_SHOWN)}...` : text;
  return `Closed (${shown})`;
}

export function approvalOutcome(a: Pick<Approval, "pending" | "decision">): ApprovalOutcome {
  if (a.pending) return { kind: "pending", label: "pending", tone: "var(--sev-medium)" };
  switch (a.decision) {
    case "grant":
      return { kind: "grant", label: "grant", tone: "var(--sev-low)" };
    case "deny":
      return { kind: "deny", label: "deny", tone: "var(--sev-critical)" };
    case "expired":
      return { kind: "expired", label: EXPIRED_LABEL, tone: NEUTRAL_TONE };
    case "withdrawn":
      return { kind: "withdrawn", label: WITHDRAWN_LABEL, tone: NEUTRAL_TONE };
    default:
      return { kind: "closed", label: closedWith(a.decision), tone: NEUTRAL_TONE };
  }
}

/** Ten years in seconds: past this a number is not a wait, it is a fault. */
const LONGEST_WAIT_SECONDS = 10 * 365 * 24 * 3600;

function seconds(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0 || value > LONGEST_WAIT_SECONDS) return null;
  return Math.floor(value);
}

/** `86400` is `24h`, `95` is `1m 35s`, `0` is `0s`. */
export function formatWait(totalSeconds: number): string {
  const h = Math.floor(totalSeconds / 3600);
  const m = Math.floor((totalSeconds % 3600) / 60);
  const s = totalSeconds % 60;
  const parts: string[] = [];
  if (h > 0) parts.push(`${h}h`);
  if (m > 0) parts.push(`${m}m`);
  if (s > 0 || parts.length === 0) parts.push(`${s}s`);
  return parts.join(" ");
}

/**
 * The one line the event feed adds under `approval_expired` and
 * `approval_withdrawn`, or `null` for any other event. The numbers are the
 * producer's and are read only when they are plain, non-negative, finite
 * numbers; a field that is missing or wrong is left out and the rest still
 * reads.
 */
export function approvalClosureLine(event: { type: string; data: unknown }): string | null {
  if (event.type !== "approval_expired" && event.type !== "approval_withdrawn") return null;
  const data =
    typeof event.data === "object" && event.data !== null && !Array.isArray(event.data)
      ? (event.data as Record<string, unknown>)
      : {};
  const expired = event.type === "approval_expired";
  const details: string[] = [];
  const waited = seconds(data.waited_seconds);
  if (waited !== null) details.push(`waited ${formatWait(waited)}`);
  const limit = expired ? seconds(data.ttl_seconds) : null;
  if (limit !== null) details.push(`limit ${formatWait(limit)}`);
  const head = expired ? EXPIRED_LABEL : WITHDRAWN_LABEL;
  return details.length > 0 ? `${head} (${details.join(", ")})` : head;
}
