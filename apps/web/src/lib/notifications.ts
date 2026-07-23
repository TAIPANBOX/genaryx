import type { UiEvent } from "../types";

/**
 * Wave-3 actionable notifications (docs/PHASE2.md, "Actionable
 * notifications"): pure extraction/mute logic plus the thin wrappers around
 * the browser's own `Notification` API this app actually calls. The live-bus
 * subscription itself lives in `useApprovalNotifications.ts`; this module
 * has no React/DOM dependency so its extraction/mute functions stay plain
 * and easy to reason about in isolation.
 *
 * Browser reality, stated plainly: a plain `new Notification(...)` gives one
 * gesture, `click`, and no per-action buttons - those (`actions` on
 * `ServiceWorkerRegistration.showNotification`) belong to a different API
 * this console does not register a service worker for. Permission may be
 * denied, or simply never granted (`Notification.permission` stuck at
 * `"default"` if the operator never responds), and the `Notification`
 * global itself may be absent. Every one of those fails closed to quiet:
 * nothing here ever throws past its own caller, the operator just sees no
 * notification.
 *
 * This IS an upgrade over the old desktop build, which could show a
 * notification but had no way to learn it was the one clicked (no click
 * callback wired to the OS layer at all). On the web,
 * [`raiseApprovalNotification`]'s `onclick` DOES deliver the clicked alert's
 * `approvalId` back into the app, through [`subscribeApprovalActionClicks`]
 * below - though still only ever as a deep-link target, never a decision:
 * the in-app Policy nav-badge (`AppShell.tsx`) remains the working fallback
 * for a denied or unavailable permission.
 */

const WARDRYX_SOURCE = "wardryx";
const APPROVAL_REQUESTED_TYPE = "approval_requested";

/** One actionable "approval needed" alert, extracted from a live
 * `source == "wardryx"`, `type == "approval_requested"` bus event. Mirrors
 * the fields `policyTypes.ts`'s `Approval` carries, but this is a UI-only
 * shape derived from a *bus* event, not a `policy_list_approvals` read -
 * the same "filter over the bus, not a new REST read" convention
 * `DecisionStream.tsx` already follows for the Decision Stream. */
export interface ApprovalAlert {
  approvalId: string;
  agentId: string;
  runId: string | null;
  env: string;
  reason: string | null;
}

/** Best-effort read of one string field out of an event's untyped `data`
 * payload - same helper `DecisionStream.tsx` keeps locally for its own
 * `reason`/`tool_names` reads; duplicated rather than imported (it is not
 * exported there), matching this codebase's own "small helper, small
 * independent copy" precedent (e.g. `policy/env.rs`'s doc comment on why it
 * keeps its own mirror of `money/env.rs`'s descriptor structs). */
function dataString(data: unknown, key: string): string | null {
  if (data && typeof data === "object" && key in (data as Record<string, unknown>)) {
    const value = (data as Record<string, unknown>)[key];
    if (typeof value === "string") return value;
  }
  return null;
}

/**
 * Pure extraction: a raw bus [`UiEvent`] -> an [`ApprovalAlert`], or `null`
 * when this event is not an actionable hold, or carries no `data.approval_id`
 * to de-dupe and deep-link against. The real Wardryx wire contract
 * (docs/PHASE2.md's "Grounded Wardryx contract": "approval_requested
 * (medium, `data.approval_id`)") guarantees this field on a genuine hold;
 * a missing one here means either a foreign/malformed event or (today) this
 * app's own demo seed data, which never actually emits this type on the
 * LIVE feed at all (see `useApprovalNotifications.ts`'s doc comment).
 * Skipping rather than fabricating an id keeps the deep link honest: it can
 * never point at an approval that does not exist.
 */
export function extractApprovalAlert(event: UiEvent): ApprovalAlert | null {
  if (event.source !== WARDRYX_SOURCE || event.type !== APPROVAL_REQUESTED_TYPE) return null;
  const approvalId = dataString(event.data, "approval_id");
  if (!approvalId) return null;
  return {
    approvalId,
    agentId: event.agent_id,
    runId: event.run_id,
    env: event.env,
    reason: dataString(event.data, "reason"),
  };
}

/** What a mute key is scoped to (docs/PHASE2.md Wave 3: "Mute: per agent /
 * per run / per environment"). */
export type MuteKind = "agent" | "run" | "env";

/** One composite mute-set entry, e.g. `"agent:agent://acme/payments"`. */
export function muteKey(kind: MuteKind, id: string): string {
  return `${kind}:${id}`;
}

/** Reverse of `muteKey("agent", id)`: the bare agent id if `key` is an
 * agent-scoped mute key, else `null`. Lets `ApprovalsInbox.tsx` render its
 * muted-agents chip strip straight from the same composite-key set
 * [`isMuted`] reads, with no second "muted agent ids" collection to keep in
 * sync. */
export function agentIdFromMuteKey(key: string): string | null {
  const prefix = muteKey("agent", "");
  return key.startsWith(prefix) ? key.slice(prefix.length) : null;
}

/**
 * Whether `alert` is covered by any key in `muted` - checked across all
 * three kinds even though the v0 UI (`ApprovalsInbox.tsx`'s per-row "mute
 * this agent" toggle) only ever writes `agent:<id>` entries today: "an
 * in-memory mute set is fine for v0" (PHASE2.md) describes the STORAGE, not
 * a ceiling on what it can represent, so a future per-run/per-environment
 * control needs no change here.
 */
export function isMuted(alert: ApprovalAlert, muted: ReadonlySet<string>): boolean {
  if (muted.has(muteKey("agent", alert.agentId))) return true;
  if (alert.runId && muted.has(muteKey("run", alert.runId))) return true;
  if (muted.has(muteKey("env", alert.env))) return true;
  return false;
}

/** "Approval needed" title + "<agent_id> (<reason>)" body - PHASE2.md Wave
 * 3's exact copy. */
export function notificationCopy(alert: ApprovalAlert): { title: string; body: string } {
  return { title: "Approval needed", body: `${alert.agentId} (${alert.reason ?? "no reason given"})` };
}

let permissionRequested = false;

/**
 * Request notification permission exactly once per app session
 * (PHASE2.md: "Request notification permission once on launch") - a
 * module-level guard (not a component ref) so this holds even if the
 * calling hook ever remounts. Never throws: a missing `Notification` global,
 * a denial, or any other platform failure just means
 * [`raiseApprovalNotification`] silently shows nothing later, same
 * fail-closed-to-quiet posture the rest of this module keeps.
 */
export async function ensureNotificationPermission(): Promise<void> {
  if (permissionRequested) return;
  permissionRequested = true;
  if (!("Notification" in window)) return;
  try {
    if (Notification.permission === "default") {
      await Notification.requestPermission();
    }
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("genaryx: notification permission request failed:", err);
  }
}

/**
 * No-op on the web, deliberately: the plain `Notification` API has no
 * per-action buttons outside a service worker (that is a different API,
 * `ServiceWorkerRegistration.showNotification`'s `actions` array, which this
 * console does not register a service worker to use). Kept as an exported
 * no-op so `useApprovalNotifications.ts` needs no special-case branch around
 * calling it; the in-app Policy nav-badge remains the one working deep-link
 * path, exactly as documented on [`raiseApprovalNotification`] below.
 */
export async function registerApprovalActions(): Promise<void> {
  // Intentionally empty - see doc comment above.
}

/** The one click handler [`raiseApprovalNotification`]'s `onclick` consults,
 * set by [`subscribeApprovalActionClicks`]. `null` means no subscriber is
 * listening, in which case a click still focuses the window but delivers no
 * approval id anywhere. */
let approvalClickHandler: ((approvalId: string) => void) | null = null;

/**
 * Raise the browser "Approval needed" notification for `alert`. Never
 * throws: a missing `Notification` global, a denied/default permission, or
 * any other platform failure is logged and swallowed - a notification
 * failing to show must never block or crash the rest of the app
 * (fail-closed-to-quiet, same rationale as [`ensureNotificationPermission`]).
 *
 * SECURITY (PHASE2.md, non-negotiable): this function only ever constructs a
 * `Notification` - it never calls `policy_decide_approval` or any decision
 * path. The `onclick` handler hands [`approvalClickHandler`] nothing but
 * `alert.approvalId`, purely so a click can be resolved to a deep-link
 * target, never so it can be auto-decided.
 */
export async function raiseApprovalNotification(alert: ApprovalAlert): Promise<void> {
  const { title, body } = notificationCopy(alert);
  try {
    if (!("Notification" in window)) return;
    const notification = new Notification(title, { body });
    notification.onclick = () => {
      window.focus();
      approvalClickHandler?.(alert.approvalId);
    };
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("genaryx: Notification failed:", err);
  }
}

/**
 * Wire up `onApprovalId` so a click on a notification raised by
 * [`raiseApprovalNotification`] delivers that notification's approval id
 * back into the app. Returns an unsubscribe function; it clears
 * [`approvalClickHandler`] only if `onApprovalId` is still the handler
 * installed (a later `subscribeApprovalActionClicks` call is left alone,
 * never clobbered by an earlier caller's stale unsubscribe).
 *
 * SECURITY (PHASE2.md, non-negotiable): `onApprovalId` is called with
 * nothing but the approval id to deep-link to - never routed to a
 * grant/deny call here. `AppShell.tsx` wires `onApprovalId` to the exact
 * same "switch to Policy, focus this approval_id" function the in-app
 * nav-badge fallback uses, so both paths land on the identical, still fully
 * `ConfirmButton`-gated, Approvals Inbox row.
 */
export async function subscribeApprovalActionClicks(onApprovalId: (approvalId: string) => void): Promise<() => void> {
  approvalClickHandler = onApprovalId;
  return () => {
    if (approvalClickHandler === onApprovalId) {
      approvalClickHandler = null;
    }
  };
}
