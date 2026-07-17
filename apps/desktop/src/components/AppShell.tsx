import { useCallback, useEffect, useRef, useState } from "react";
import type { ApprovalAlert } from "../lib/notifications";
import { muteKey } from "../lib/notifications";
import { useApprovalNotifications } from "../lib/useApprovalNotifications";
import type { ViewId } from "../lib/views";
import { Agent360 } from "./Agent360";
import { AppHeader } from "./AppHeader";
import { BusExplorer } from "./BusExplorer";
import { DelegationGraphView } from "./DelegationGraphView";
import { IdentityView } from "./IdentityView";
import { MoneyView } from "./MoneyView";
import { OverviewView } from "./OverviewView";
import { PolicyView } from "./PolicyView";
import { PostureView } from "./PostureView";
import { RunReplayView } from "./RunReplayView";

/** How long a notification's deep-link target stays "focused" (drives
 * `ApprovalsInbox.tsx`'s scroll-to + `.approval-focused` highlight) before
 * clearing itself - long enough for the operator to notice and locate the
 * row after navigating over, short enough that it reads as "what just
 * happened" rather than a permanent marker. The row stays fully visible and
 * interactive either way; only the highlight fades. */
const FOCUS_HIGHLIGHT_MS = 6_000;

/**
 * App root: owns the theme (persisted to `document.documentElement.dataset`
 * the same way the Bus Explorer's header used to on its own) and the active
 * view, and renders the persistent `AppHeader` plus whichever view is
 * selected. The `.app` class (ambient backdrop + full-height flex column,
 * see `index.css`) lives here instead of inside `BusExplorer`, since it is a
 * whole-app concern shared by every view, not a Bus-specific one.
 *
 * Wave-3 addition (docs/PHASE2.md "Actionable notifications" +
 * "Posture-lite"): this is also where `useApprovalNotifications` mounts -
 * the one place guaranteed to stay alive across every view switch, which a
 * background alert watcher needs. It owns the two pieces of state the
 * notification deep link and per-agent mute span across views:
 *
 * - `mutedKeys` - the in-memory mute set (docs/PHASE2.md: "an in-memory mute
 *   set is fine for v0"), read by the watcher on every live event and
 *   written by `ApprovalsInbox.tsx`'s per-row mute toggle. Lives here (not
 *   inside `PolicyView`) because a mute must keep suppressing notifications
 *   even while the operator is looking at a different view.
 * - `focusApprovalId` / `unseenAlerts` - the deep-link target and the
 *   Policy nav badge count. A new alert arms both but does NOT itself
 *   switch views (raising a background alert must never yank focus away
 *   from whatever the operator is doing); only an explicit tap - the nav
 *   badge, or a real OS notification-action click were one ever delivered
 *   (`onActionApprovalId`) - navigates. `unseenAlerts` clears the moment
 *   Policy becomes the active view (the badge's own "you looked" signal);
 *   `focusApprovalId` clears itself after [`FOCUS_HIGHLIGHT_MS`] regardless
 *   of view, so the highlight always fades even if the operator was already
 *   on Policy when it was set.
 */
export function AppShell() {
  const [theme, setTheme] = useState<"dark" | "light">("dark");
  const [view, setView] = useState<ViewId>("overview");

  const [mutedKeys, setMutedKeys] = useState<ReadonlySet<string>>(new Set());
  const [unseenAlerts, setUnseenAlerts] = useState<readonly ApprovalAlert[]>([]);
  const [focusApprovalId, setFocusApprovalId] = useState<string | null>(null);
  const focusTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  // Phase-3 wave-3 addition (docs/PHASE3.md "W3 - graph + 360"): the whole
  // deep-link mechanism is this one piece of state, owned here for the same
  // reason `focusApprovalId` is - Agent 360 must be reachable "from anywhere"
  // (a graph node, an Identity/Money/Policy row, ...), so it is rendered as
  // an overlay on top of whichever `view` is active rather than being a view
  // itself. Setting it never changes `view`; closing it never clears `view`
  // either, so opening a 360 card and dismissing it always returns the
  // operator to exactly the panel they were on.
  const [focusedAgentId, setFocusedAgentId] = useState<string | null>(null);
  const onOpenAgent = useCallback((agentId: string) => setFocusedAgentId(agentId), []);
  const onCloseAgent360 = useCallback(() => setFocusedAgentId(null), []);
  // Agent 360's "Open <plane> panel" links (docs/PHASE3.md: actions link to
  // the existing panels rather than re-implement a mutation in the card) -
  // switch the active view AND dismiss the overlay in one operator gesture.
  const onNavigateFromAgent360 = useCallback((next: ViewId) => {
    setView(next);
    setFocusedAgentId(null);
  }, []);

  // Phase-3 wave-4 addition (docs/PHASE3.md "W4 - replay + posture"): Run
  // Replay is a real nav view (`views.ts`), not an overlay like Agent 360 -
  // but it still needs a deep-link seed (which run to open straight into),
  // so `replayRunId` plays the same role `focusApprovalId` does for Policy:
  // state owned here, read by `RunReplayView` as its `presetRunId` prop.
  // `key={replayRunId ?? "picker"}` at the render site below remounts the
  // view fresh on every new entry-point call, so a stale preset from a
  // PREVIOUS replay session can never leak into a new one.
  const [replayRunId, setReplayRunId] = useState<string | null>(null);
  const onOpenReplay = useCallback((runId: string) => {
    setReplayRunId(runId);
    setView("replay");
    // Closes Agent 360 when the "Replay" affordance was clicked from inside
    // its Money section - a harmless no-op when there was no overlay open
    // (e.g. the affordance was clicked straight from the Money panel's own
    // RunsTable), mirroring `onNavigateFromAgent360`'s identical double duty.
    setFocusedAgentId(null);
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  // The badge clears the moment the operator actually looks at Policy -
  // matches how an unread count normally works, independent of the
  // separate highlight-fade timer below.
  useEffect(() => {
    if (view === "policy" && unseenAlerts.length > 0) setUnseenAlerts([]);
  }, [view, unseenAlerts.length]);

  const armFocus = useCallback((approvalId: string) => {
    setFocusApprovalId(approvalId);
    if (focusTimer.current !== undefined) window.clearTimeout(focusTimer.current);
    focusTimer.current = window.setTimeout(() => setFocusApprovalId(null), FOCUS_HIGHLIGHT_MS);
  }, []);

  useEffect(() => () => {
    if (focusTimer.current !== undefined) window.clearTimeout(focusTimer.current);
  }, []);

  const onAlert = useCallback(
    (alert: ApprovalAlert) => {
      setUnseenAlerts((prev) => [...prev, alert]);
      armFocus(alert.approvalId);
    },
    [armFocus],
  );

  // A real OS notification-action click, were one ever delivered (see
  // `lib/notifications.ts`'s doc comment for why that does not happen on
  // today's desktop plugin) - an explicit operator interaction, so unlike
  // `onAlert` this DOES navigate.
  const onActionApprovalId = useCallback(
    (approvalId: string) => {
      setView("policy");
      armFocus(approvalId);
    },
    [armFocus],
  );

  useApprovalNotifications({ muted: mutedKeys, onAlert, onActionApprovalId });

  const onToggleMuteAgent = useCallback((agentId: string) => {
    setMutedKeys((prev) => {
      const key = muteKey("agent", agentId);
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  }, []);

  // Clicking the Policy nav item (badge or not) always re-focuses the
  // latest pending alert, if any - the in-app realization of "tap
  // [the notification] to focus the Policy panel" (PHASE2.md).
  const onSelectView = useCallback(
    (next: ViewId) => {
      setView(next);
      if (next === "policy" && unseenAlerts.length > 0) {
        armFocus(unseenAlerts[unseenAlerts.length - 1].approvalId);
      }
    },
    [armFocus, unseenAlerts],
  );

  return (
    <div className="app">
      <AppHeader
        view={view}
        onSelectView={onSelectView}
        theme={theme}
        onToggleTheme={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
        policyAlertCount={unseenAlerts.length}
      />
      {view === "overview" && <OverviewView />}
      {view === "money" && <MoneyView onOpenAgent={onOpenAgent} onOpenReplay={onOpenReplay} />}
      {view === "policy" && (
        <PolicyView
          focusApprovalId={focusApprovalId}
          mutedKeys={mutedKeys}
          onToggleMuteAgent={onToggleMuteAgent}
          onOpenAgent={onOpenAgent}
        />
      )}
      {view === "identity" && <IdentityView onOpenAgent={onOpenAgent} />}
      {view === "graph" && (
        <div className="flex-1 min-h-0 px-5 py-4 flex flex-col gap-3">
          <DelegationGraphView onOpenAgent={onOpenAgent} fill />
        </div>
      )}
      {view === "replay" && (
        <RunReplayView key={replayRunId ?? "picker"} presetRunId={replayRunId} onOpenAgent={onOpenAgent} />
      )}
      {view === "posture" && <PostureView />}
      {view === "bus" && <BusExplorer />}

      {focusedAgentId && (
        <Agent360
          key={focusedAgentId}
          agentId={focusedAgentId}
          onClose={onCloseAgent360}
          onOpenAgent={onOpenAgent}
          onNavigate={onNavigateFromAgent360}
          onOpenReplay={onOpenReplay}
        />
      )}
    </div>
  );
}
