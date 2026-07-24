import { hasBackend, subscribeBackend } from "./transport";
import { useEffect, useRef } from "react";
import type { UiEvent } from "../types";
import {
  type ApprovalAlert,
  ensureNotificationPermission,
  extractApprovalAlert,
  isMuted,
  raiseApprovalNotification,
  registerApprovalActions,
  subscribeApprovalActionClicks,
} from "./notifications";

/** Bus event name the live feed (`crates/api/src/bus/feed.rs`) emits on -
 * the SAME event `DecisionStream.tsx`/`BusExplorer.tsx` listen for. */
const LIVE_EVENT = "bus:event";

/**
 * Wave-3 actionable notifications (docs/PHASE2.md): watches the SAME live
 * bus feed the Decision Stream filters (the `bus:event` listener - see
 * `DecisionStream.tsx`'s doc comment for why that is "the existing event
 * pipeline", not a new poll) for `source == "wardryx"`,
 * `type == "approval_requested"` events, and raises a native "Approval
 * needed" notification for each one that is not muted and has not already
 * been observed this session (`notifications.ts`'s `extractApprovalAlert`/
 * `isMuted`/`raiseApprovalNotification`).
 *
 * Deliberately reacts ONLY to live arrivals, never the initial
 * `fetchRecentEvents` backfill batch other bus-consuming views also read:
 * a notification is meant to alert the operator to something that JUST
 * happened, and the backfilled batch (present the instant any view first
 * reads the bus) can include `approval_requested` rows that are already
 * resolved - this app's own demo seed data pairs every `approval_requested`
 * with an `approval_granted` a few rows later (`crates/api/src/events.rs`'s
 * `seeds()`). Re-alerting on history every time a view mounts would be
 * exactly the kind of spurious re-notification the Wave-3 de-dupe rule
 * ("never re-raised on a list refresh") rules out.
 *
 * Honest limitation, worth restating here (see `notifications.ts`'s doc
 * comment for the grounded, source-read reason): this app's bundled
 * demo/dev live feeder (`live.rs`'s `feeder_line`) cycles through
 * `policy_allow`/`quality_score`/`sim_run`/`memory_written` and never emits
 * `approval_requested` on its synthetic ~2s tick, so this watcher will not
 * fire spontaneously against the bundled demo data alone - it fires
 * correctly the moment a REAL Wardryx hold reaches this same bus (the
 * production wiring this demo stands in for). This mirrors every other
 * wardryx-flavored panel in this app (`DecisionStream`, `ApprovalsInbox`),
 * all of which are equally "correct against a real backend, quiet against
 * the bundled demo feeder alone" for the same reason.
 */
export function useApprovalNotifications({
  muted,
  onAlert,
  onActionApprovalId,
}: {
  /** Latest mute-key set (docs/PHASE2.md: "Mute: per agent / per run / per
   * environment") - read through a ref internally so the one live-listener
   * subscription below (mounted once) always sees the current value rather
   * than a stale closure over the value at mount time. */
  muted: ReadonlySet<string>;
  /** Called once per newly-observed, non-muted alert. `AppShell.tsx` uses
   * this to arm the Policy nav badge / pending deep-link target. */
  onAlert: (alert: ApprovalAlert) => void;
  /** Called if a real OS notification action/click ever reaches this app
   * (see `notifications.ts`'s doc comment for why that does not happen on
   * today's desktop plugin) - always just the `approval_id` to deep-link
   * to, NEVER a decision; wiring it straight to a grant/deny would violate
   * PHASE2.md's non-negotiable rule. */
  onActionApprovalId: (approvalId: string) => void;
}): void {
  const mutedRef = useRef(muted);
  mutedRef.current = muted;

  const onAlertRef = useRef(onAlert);
  onAlertRef.current = onAlert;

  const onActionRef = useRef(onActionApprovalId);
  onActionRef.current = onActionApprovalId;

  // Permission + best-effort action-type registration: once per app
  // session, and only with a real backend configured (a plain `vite
  // dev`/browser preview has no live bus to raise a notification from -
  // mirrors every other `hasBackend()` guard in this app, e.g.
  // `BusExplorer.tsx`'s live-listener effect).
  useEffect(() => {
    if (!hasBackend()) return;
    void ensureNotificationPermission();
    void registerApprovalActions();
  }, []);

  // Best-effort action-click subscription (see `subscribeApprovalActionClicks`'s
  // doc comment - inert on today's desktop plugin, kept for when it is not).
  useEffect(() => {
    if (!hasBackend()) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    void subscribeApprovalActionClicks((approvalId) => onActionRef.current(approvalId)).then((fn) => {
      if (cancelled) {
        void fn();
        return;
      }
      unlisten = fn;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // The actual watcher: one live-bus subscription for the app's lifetime,
  // with a session-scoped `seen` set closed over by that single
  // subscription (so de-dupe survives exactly as long as the listener
  // does, with no separate ref/state needed for it).
  useEffect(() => {
    if (!hasBackend()) return;
    const seen = new Set<string>();
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    subscribeBackend<UiEvent>(LIVE_EVENT, (payload) => {
      const alert = extractApprovalAlert(payload);
      if (!alert || seen.has(alert.approvalId)) return;
      seen.add(alert.approvalId);
      if (isMuted(alert, mutedRef.current)) return;
      onAlertRef.current(alert);
      void raiseApprovalNotification(alert);
    })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch((err: unknown) => {
        // eslint-disable-next-line no-console
        console.error(`subscribe(${LIVE_EVENT}) failed (approval notifications):`, err);
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);
}
