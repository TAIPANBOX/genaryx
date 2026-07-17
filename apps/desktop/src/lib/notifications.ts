import {
  isPermissionGranted,
  onAction,
  registerActionTypes,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import type { UiEvent } from "../types";

/**
 * Wave-3 actionable notifications (docs/PHASE2.md, "Actionable
 * notifications"): pure extraction/mute logic plus the thin wrappers around
 * `@tauri-apps/plugin-notification` this app actually calls. The live-bus
 * subscription itself lives in `useApprovalNotifications.ts`; this module
 * has no React/Tauri-event dependency so its extraction/mute functions stay
 * plain and easy to reason about in isolation.
 *
 * GROUNDED PLATFORM LIMITATION (read from the installed
 * `tauri-plugin-notification` 2.3.3 crate's own source, not assumed):
 * `src/lib.rs`'s `invoke_handler` wires exactly three commands on every
 * platform - `notify`, `request_permission`, `is_permission_granted`.
 * `register_action_types` and the `actionPerformed` event `onAction`
 * listens for are part of the npm package's JS surface (for parity with the
 * plugin's separate MOBILE Kotlin/Swift backend, `src/mobile.rs` - not built
 * by this desktop app) but nothing in `src/desktop.rs` ever registers that
 * command or emits that event. Concretely: `desktop.rs`'s
 * `imp::Notification::show()` calls `notify_rust::Notification::show()` and
 * discards the result (`let _ = notification.show();`) - no
 * `.wait_for_action(..)`, no click callback, nothing wired back to the
 * webview. So on this app (macOS desktop), clicking a raised notification
 * cannot deliver an "approve" vs "deny" (or even a bare "clicked") signal
 * into the app at all; the OS still brings the app to the foreground on
 * click (standard OS behavior for any app-attributed notification,
 * independent of this plugin), but with no way to tell the app WHICH
 * notification was clicked.
 *
 * This is exactly the case PHASE2.md's Wave-3 spec anticipates and
 * explicitly sanctions: "actions where the platform supports them,
 * otherwise a tap that focuses the Policy panel - that fallback is
 * acceptable." [`registerApprovalActions`]/[`subscribeApprovalActionClicks`]
 * below still make the real-actions attempt (so this starts working for
 * free the day the plugin or this app's target platform changes), but the
 * WORKING deep-link path today is `AppShell.tsx`'s in-app "Policy" nav
 * badge: raising a notification also arms that badge, and clicking it
 * performs the exact same "switch to Policy, focus this approval_id"
 * navigation a real notification click would have. Either path funnels
 * through the identical `onApprovalId`/`focusApprovalId` plumbing and NEVER
 * calls a grant/deny function directly - only `ApprovalsInbox.tsx`'s
 * existing `ConfirmButton`-gated `policy_decide_approval` call does that.
 */

const WARDRYX_SOURCE = "wardryx";
const APPROVAL_REQUESTED_TYPE = "approval_requested";

/** The one action-type id every approval notification is raised under -
 * shared between [`registerApprovalActions`] and [`raiseApprovalNotification`]
 * so a click (were one ever delivered) always resolves to a type this app
 * itself registered. */
const APPROVAL_ACTION_TYPE = "wardryx-approval";

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
 * calling hook ever remounts. Never throws: a denial or a platform failure
 * just means [`raiseApprovalNotification`] silently shows nothing later,
 * same fail-closed-to-quiet posture `tray.rs`'s `log_menu_result` takes for
 * other best-effort OS integration calls.
 */
export async function ensureNotificationPermission(): Promise<void> {
  if (permissionRequested) return;
  permissionRequested = true;
  try {
    const granted = await isPermissionGranted();
    if (!granted) {
      await requestPermission();
    }
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("genaryx: notification permission request failed:", err);
  }
}

let actionsRegistered = false;

/**
 * Best-effort real OS action buttons (Review/Approve/Deny, matching the
 * SwiftUI side's PHASE2.md-specified action set) - see this module's doc
 * comment for why this is expected to fail harmlessly on today's desktop
 * build. Idempotent per session; swallows every error rather than letting a
 * platform that does not support this take down the notification feature.
 */
export async function registerApprovalActions(): Promise<void> {
  if (actionsRegistered) return;
  actionsRegistered = true;
  try {
    await registerActionTypes([
      {
        id: APPROVAL_ACTION_TYPE,
        actions: [
          { id: "review", title: "Review" },
          { id: "approve", title: "Approve" },
          { id: "deny", title: "Deny" },
        ],
      },
    ]);
  } catch (err) {
    // eslint-disable-next-line no-console
    console.debug("genaryx: registerActionTypes unavailable on this platform (expected on desktop):", err);
  }
}

/**
 * Raise the native "Approval needed" notification for `alert`. Never
 * throws: a platform/permission failure is logged and swallowed - a
 * notification failing to show must never block or crash the rest of the
 * app (fail-closed-to-quiet, same rationale as [`ensureNotificationPermission`]).
 *
 * SECURITY (PHASE2.md, non-negotiable): this function only ever calls
 * `sendNotification` - it never calls `policy_decide_approval` or any
 * decision path. `extra.approvalId` is carried purely so a real action
 * click (were the platform ever to deliver one, see
 * [`subscribeApprovalActionClicks`]) can be resolved back to a deep-link
 * target, never so it can be auto-decided.
 */
export async function raiseApprovalNotification(alert: ApprovalAlert): Promise<void> {
  const { title, body } = notificationCopy(alert);
  try {
    sendNotification({
      title,
      body,
      actionTypeId: APPROVAL_ACTION_TYPE,
      extra: { approvalId: alert.approvalId, agentId: alert.agentId, runId: alert.runId ?? "" },
    });
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("genaryx: sendNotification failed:", err);
  }
}

/**
 * Best-effort wiring for a real OS notification-action click, if this
 * plugin+platform combination ever delivers one (today it does not on
 * desktop - see this module's doc comment). Returns an unsubscribe
 * function; safe to call even when nothing will ever fire the listener.
 *
 * SECURITY (PHASE2.md, non-negotiable): `onApprovalId` is called with
 * nothing but the approval id to deep-link to, regardless of WHICH action
 * (review/approve/deny) was clicked - never routed to a grant/deny call
 * here. `AppShell.tsx` wires `onApprovalId` to the exact same
 * "switch to Policy, focus this approval_id" function the in-app nav-badge
 * fallback uses, so both paths land on the identical, still fully
 * ConfirmButton-gated, Approvals Inbox row.
 */
export async function subscribeApprovalActionClicks(onApprovalId: (approvalId: string) => void): Promise<() => void> {
  try {
    const listener = await onAction((notification) => {
      const approvalId = notification.extra?.["approvalId"];
      if (typeof approvalId === "string" && approvalId.length > 0) {
        onApprovalId(approvalId);
      }
    });
    return () => {
      void listener.unregister();
    };
  } catch (err) {
    // eslint-disable-next-line no-console
    console.debug("genaryx: onAction unavailable on this platform (expected on desktop):", err);
    return () => {};
  }
}
