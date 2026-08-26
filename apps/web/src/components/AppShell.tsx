import { useCallback, useEffect, useRef, useState } from "react";
import type { CopilotExplainRequest } from "../copilotTypes";
import { usePopover } from "../lib/popover";
import type { ApprovalAlert } from "../lib/notifications";
import { muteKey } from "../lib/notifications";
import { useApprovalNotifications } from "../lib/useApprovalNotifications";
import type { MailLink } from "../lib/mailLink";
import { mailLinkFrom, mailLinkNotice, parseMailLink } from "../lib/mailLink";
import type { ViewId } from "../lib/views";
import { Agent360 } from "./Agent360";
import { AgentDetailCard } from "./AgentDetailCard";
import { AppHeader } from "./AppHeader";
import { BusExplorer } from "./BusExplorer";
import { CopilotView } from "./CopilotView";
import { CryptoView } from "./CryptoView";
import { DelegationGraphView } from "./DelegationGraphView";
import { DrillsView } from "./DrillsView";
import { EvidenceView } from "./EvidenceView";
import { IdentityView } from "./IdentityView";
import { MemoryView } from "./MemoryView";
import { MoneyView } from "./MoneyView";
import { OnboardView } from "./OnboardView";
import { AnomaliesView } from "./AnomaliesView";
import { Incident360 } from "./Incident360";
import type { UnifiedIncident } from "../lib/incidents";
import { OverviewView } from "./OverviewView";
import { PolicyView } from "./PolicyView";
import { PostureView } from "./PostureView";
import { EgressView } from "./EgressView";
import { StatsView } from "./StatsView";
import { QualityView } from "./QualityView";
import { RemoteView } from "./RemoteView";
import { RoutinesView } from "./RoutinesView";
import { RunReplayView } from "./RunReplayView";
import { UnitCard } from "./UnitCard";
import { UserCard } from "./UserCard";
import { isUserId } from "../lib/graph";
import { userHandle } from "../lib/agentRecord";
import { WatchDock } from "./WatchDock";

/** How long a notification's deep-link target stays "focused" (drives
 * `ApprovalsInbox.tsx`'s scroll-to + `.approval-focused` highlight) before
 * clearing itself - long enough for the operator to notice and locate the
 * row after navigating over, short enough that it reads as "what just
 * happened" rather than a permanent marker. The row stays fully visible and
 * interactive either way; only the highlight fades. */
const FOCUS_HIGHLIGHT_MS = 6_000;

/** Agent 360 compare (below this width there is no honest way to show two
 * 720px-wide cards - see `Agent360.tsx`'s own width comment - side by side
 * without one running off-screen), so the shell falls back to rendering
 * only the most-recently-focused card. An approximate, product-picked
 * threshold, not derived from the 720px card width itself. */
const COMPARE_MIN_WIDTH = 1200;

/** Left-rail collapse state (Yurii, 2026-07-24): persisted so a reload keeps
 * the operator's own choice rather than always reopening at full width. */
const RAIL_COLLAPSED_KEY = "genaryx.railCollapsed";

function readStoredFlag(key: string): boolean {
  try {
    return localStorage.getItem(key) === "true";
  } catch {
    // Storage unavailable (private mode, disabled) - the rail just always
    // starts expanded for that session rather than failing to render.
    return false;
  }
}

function writeStoredFlag(key: string, value: boolean): void {
  try {
    localStorage.setItem(key, value ? "true" : "false");
  } catch {
    // Best-effort only - the toggle still works for the rest of this
    // session, it just will not be remembered across a reload.
  }
}

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
 *
 * C1 addition (docs/PHASE6-C1.md): `explainRequest` is the same "seed state
 * here, hand it down as a prop" shape as `replayRunId`, for the "Explain
 * with Felyx" affordance - a sibling view (the Money panel's Incidents feed)
 * calls `onExplainIncident(incidentId)`, which switches to the Copilot view
 * AND seeds it with the incident to explain; `CopilotView` itself runs the
 * actual `copilot_explain` round trip and clears the seed when done.
 */
export function AppShell() {
  const [theme, setTheme] = useState<"dark" | "light">("dark");
  const [view, setView] = useState<ViewId>("overview");
  const { open } = usePopover();

  // Left rail collapse/expand (Yurii, 2026-07-24): lives here (not inside
  // `AppHeader.tsx` itself) for the same reason `focusedAgentIds` does -
  // persisted, whole-app chrome state, not a view's own concern.
  const [railCollapsed, setRailCollapsed] = useState<boolean>(() => readStoredFlag(RAIL_COLLAPSED_KEY));
  const onToggleRail = useCallback(() => {
    setRailCollapsed((prev) => {
      const next = !prev;
      writeStoredFlag(RAIL_COLLAPSED_KEY, next);
      return next;
    });
  }, []);

  // Arriving from the alert mail's one link (`/i/{type}:{subject}`, see
  // `lib/mailLink.ts`). Read ONCE, here, on mount rather than in `App.tsx`,
  // and the placement is the point: `WebGate` sits above this component, so a
  // signed-out operator meets the sign-in form first and this effect runs
  // after they are through it. The path is still in the address bar at that
  // moment, so the link survives the gate without anything having to remember
  // it across a navigation.
  const [mailLink, setMailLink] = useState<MailLink | null>(null);
  useEffect(() => {
    // Path first, fragment second: see `mailLinkFrom`. The fragment form is
    // what makes this work on a static deployment, where a file server asked
    // for `/i/budget_exhausted:run-42` answers 404 and the click never reaches
    // this code at all.
    const link = mailLinkFrom(window.location);
    if (link === null) return;
    setMailLink(link);
    // A type this build does not know does not get a guessed panel. The
    // operator lands on the overview, where the incident centre aggregates
    // every plane, and the notice says which id could not be placed.
    if (link.view !== null) setView(link.view);
    // An agent link opens that agent's DETAIL CARD, because that card is where
    // freeze and kill are.
    //
    // It opened Agent 360 until 2026-08-03, and both this comment and
    // `lib/mailLink.ts` said Agent 360 was "where those controls live". Neither
    // was true: `Agent360.tsx` imports `runBlockedState` and `StateBadge` and
    // nothing else from `lib/lifecycle`, so it SHOWS whether an agent is
    // blocked and offers no way to block it. `AgentDetailCard.tsx` is the one
    // that imports `FreezeToggleButton` and `KillRunButton`.
    //
    // The mail that sends an operator here says "(freeze, kill)" beside the
    // link, so the old behaviour ended a two-in-the-morning path at a screen
    // that could not do the thing the mail had just named. Reported by Yurii,
    // 2026-08-03, from the sample on it-rat.com.
    //
    // Centred rather than anchored: there is no click and so no rect to sit
    // beside, and `usePopover` already centres an anchorless window. Agent 360
    // stays one step away through the card's own "open full".
    if (link.kind === "agent") {
      open(<AgentDetailCard agentId={link.subject} onOpenFull={onOpenAgent} />);
    }
    // And the owner link opens the OWNER, for the same reason: the mail says
    // "who is answerable, and what else are they running", and that is one
    // card (`UserCard.tsx`, every agent they own with what those agents spend),
    // not the whole Identity panel the link used to stop at. The panel is
    // still what it lands ON, so the card has its context behind it.
    // Reported by Yurii, 2026-08-03, alongside the agent link.
    if (link.kind === "owner") {
      open(<UserCard handle={link.subject} onOpenFullAgent={onOpenAgent} />);
    }
    // Drop the deep link from the address bar once it has been acted on, so a
    // reload is an ordinary reload rather than a second arrival, and so the
    // id does not sit in the URL for a screenshot to carry away. `replaceState`
    // rather than `pushState`: there is no history entry worth going back to.
    // Clear whichever form it arrived in, and only that one.
    //
    // When the PATH was the link, the path has to go, so the console lands at
    // its own root. When the FRAGMENT was, the path is where the app is
    // actually served from and must survive: a static deployment lives under
    // `/demo/`, and replacing that with `/` would point a reload at the site
    // root instead of the console.
    const cleared = parseMailLink(window.location.pathname) !== null
      ? "/"
      : window.location.pathname + window.location.search;
    try {
      window.history.replaceState(null, "", cleared);
    } catch {
      // Some embedded webviews refuse this. The console is already showing
      // the right panel by then, which is the part that matters.
    }
    // Deliberately mount-only. `open` and `onOpenAgent` are stable for this
    // component's life, and re-running this would re-open the card every time
    // one of them changed identity, on a link that has already been consumed
    // and cleared from the address bar.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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
  //
  // Compare addition: up to TWO agents can be focused at once, so the
  // operator can put two Agent 360 cards side by side. `focusedAgentIds[0]`
  // is the "first"/anchor card - it is pinned nearest the screen edge and is
  // never displaced by opening a new agent. `focusedAgentIds[1]`, when
  // present, is the "second"/compare card immediately to its left - this is
  // the slot a new `onOpenAgent` call fills or replaces. Order carries
  // meaning here; this is not an unordered set of open ids.
  const [focusedAgentIds, setFocusedAgentIds] = useState<string[]>([]);

  // Open/replace rule for the two slots above, deliberately just three
  // branches:
  // - the id is already shown (either slot) -> no-op, and returns the SAME
  //   array reference so React bails out of the re-render - the existing
  //   card never remounts (no lost scroll position, no re-fetch of any of
  //   its five sections).
  // - nothing open -> the id becomes the first/anchor card.
  // - one OR two cards already open -> the id becomes (or replaces) the
  //   SECOND card; the first/anchor card is never displaced by a
  //   newly-opened id. This is the "pin the primary, swap the comparison"
  //   rule: comparing agent A against B, then clicking a delegate C inside
  //   either card, keeps A anchored and swaps C in for B, rather than
  //   silently dropping the agent the operator opened first.
  //
  // It also routes by SCHEME, and that is not a detail of this callback: it is
  // the one place every surface funnels through. The delegation chain and the
  // delegation graph both carry people as well as agents, and until 2026-08-11
  // each caller passed whatever it had straight in, so clicking `n.foster`
  // opened an Agent 360 about a person. Fixing the two callers would have left
  // the third to be written wrong later; fixing the funnel cannot.
  const onOpenAgent = useCallback(
    (id: string) => {
      if (isUserId(id)) {
        open(<UserCard handle={userHandle(id)} onOpenFullAgent={onOpenAgent} />);
        return;
      }
      setFocusedAgentIds((prev) => {
        if (prev.includes(id)) return prev;
        if (prev.length === 0) return [id];
        return [prev[0], id];
      });
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [open],
  );
  // Closes exactly one card - the render site below binds each card's own
  // `onClose` to its own `agentId`, so a card's close button only ever
  // removes itself, never its neighbor. Whichever id remains keeps its
  // existing slot: closing the anchor while a compare card is open leaves
  // the compare card standing alone (it renders pinned to the screen edge,
  // exactly like any single open card - no slide animation to write or
  // maintain), and closing the compare card leaves the anchor exactly as it
  // was. With one card open, closing it clears the overlay entirely.
  const onCloseAgent360 = useCallback((agentId: string) => {
    setFocusedAgentIds((prev) => prev.filter((id) => id !== agentId));
  }, []);
  // Agent 360's "Open <plane> panel" links (docs/PHASE3.md: actions link to
  // the existing panels rather than re-implement a mutation in the card) -
  // switch the active view AND dismiss BOTH cards in one operator gesture.
  const onNavigateFromAgent360 = useCallback((next: ViewId) => {
    setView(next);
    setFocusedAgentIds([]);
  }, []);

  // Below `COMPARE_MIN_WIDTH`, the render site further down only shows the
  // most-recently-focused card (see `visibleAgentIds`). Both ids stay in
  // `focusedAgentIds` regardless of viewport width, so widening the window
  // back past the threshold brings the second card straight back without
  // the operator having to reopen it.
  const [isNarrowForCompare, setIsNarrowForCompare] = useState(() => window.innerWidth < COMPARE_MIN_WIDTH);
  useEffect(() => {
    const onResize = () => setIsNarrowForCompare(window.innerWidth < COMPARE_MIN_WIDTH);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
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
    // Closes both Agent 360 cards when the "Replay" affordance was clicked
    // from inside one of their Money sections - a harmless no-op when
    // neither was open (e.g. the affordance was clicked straight from the
    // Money panel's own RunsTable), mirroring `onNavigateFromAgent360`'s
    // identical double duty.
    setFocusedAgentIds([]);
  }, []);

  // "Explain with Felyx" (C1, docs/PHASE6-C1.md): the Incidents surface (the
  // Money panel today) hands an incident id here rather than calling
  // `copilot_explain` itself, so the Copilot pane stays the one place that
  // owns a Felyx round trip and its transcript - mirrors `onOpenReplay`'s
  // "seed state here, read it as a prop on the target view" shape, except
  // `CopilotView` fully unmounts/remounts on every view switch (unlike
  // `RunReplayView`, no `key={...}` trick is needed to force a fresh look at
  // a new request). `explainNonce` is a plain ever-increasing counter (never
  // reset) rather than a timestamp, so two requests fired within the same
  // millisecond can never collide.
  const explainNonce = useRef(0);
  const [explainRequest, setExplainRequest] = useState<CopilotExplainRequest | null>(null);
  const onExplainIncident = useCallback((incidentId: string) => {
    setExplainRequest({ nonce: explainNonce.current++, incidentId });
    setView("copilot");
  }, []);
  // Passed to `CopilotView` as `onExplainRequestHandled`: called once the
  // request's round trip finishes (success or error), so a later, unrelated
  // remount of the Copilot view never re-fires the same explain call. Stable
  // across renders (`setExplainRequest` is itself a stable setter), which
  // `CopilotView`'s own effect relies on - see its doc comment.
  const onExplainRequestHandled = useCallback(() => setExplainRequest(null), []);

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

  // Both ids stay in `focusedAgentIds` on a narrow viewport (see
  // `isNarrowForCompare` above) - only the render is limited to the single
  // most-recent card here, via a plain slice with no state mutation.
  const visibleAgentIds = isNarrowForCompare ? focusedAgentIds.slice(-1) : focusedAgentIds;

  // The incident currently open in the overlay layer. One at a time, unlike
  // agents: comparing two agents side by side is a real operator gesture and
  // comparing two incidents is not, and each incident card already opens the
  // agents it names beside itself.
  const [focusedIncident, setFocusedIncident] = useState<UnifiedIncident | null>(null);

  // Watch dock (Yurii, 2026-07-24): a pinned unit's row opens the SAME
  // `UnitCard` every other unit link in the app opens (`AgentDetailCard`'s
  // own "business unit" field, `Agent360.tsx`'s eventual equivalent), via
  // this shell's own `usePopover()` - centered (no anchor rect) since the
  // click comes from a narrow side rail with no nearby content to anchor
  // beside; `usePopover`'s own placement already falls back to centering
  // when no anchor is given (see `lib/popover.tsx`).
  const onOpenUnit = useCallback(
    (unitId: string) => {
      open(<UnitCard team={unitId} onOpenFullAgent={onOpenAgent} />);
    },
    [open, onOpenAgent],
  );

  // The owner twin of `onOpenUnit`, for the same reason and to the same card
  // the owner deep-link already opens above. Statistics needs it as a prop
  // rather than inline, because every row of its owner grouping is a way in.
  const onOpenUser = useCallback(
    (handle: string) => {
      open(<UserCard handle={handle} onOpenFullAgent={onOpenAgent} />);
    },
    [open, onOpenAgent],
  );

  return (
    <div className="app">
      <AppHeader
        view={view}
        onSelectView={onSelectView}
        theme={theme}
        onToggleTheme={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
        policyAlertCount={unseenAlerts.length}
        railCollapsed={railCollapsed}
        onToggleRail={onToggleRail}
      />
      <div className="main-col">
        {mailLink !== null && (
          <div className="mail-link-notice" role="status">
            <span>{mailLinkNotice(mailLink)}</span>
            <button type="button" onClick={() => setMailLink(null)} aria-label="Dismiss">
              dismiss
            </button>
          </div>
        )}
        {view === "overview" && (
          <OverviewView
            onOpenAgent={onOpenAgent}
            onSelectView={onSelectView}
            onExplainIncident={onExplainIncident}
            onOpenIncident={setFocusedIncident}
          />
        )}
        {view === "anomalies" && (
          <AnomaliesView onSelectView={onSelectView} onOpenIncident={setFocusedIncident} />
        )}
        {view === "money" && (
          <MoneyView onOpenAgent={onOpenAgent} onOpenReplay={onOpenReplay} onExplainIncident={onExplainIncident} />
        )}
        {view === "policy" && (
          <PolicyView
            focusApprovalId={focusApprovalId}
            mutedKeys={mutedKeys}
            onToggleMuteAgent={onToggleMuteAgent}
            onOpenAgent={onOpenAgent}
          />
        )}
        {view === "identity" && <IdentityView onOpenAgent={onOpenAgent} />}
        {view === "onboard" && <OnboardView />}
        {view === "quality" && <QualityView onOpenAgent={onOpenAgent} />}
        {view === "egress" && <EgressView />}
        {view === "stats" && (
          <StatsView onOpenAgent={onOpenAgent} onOpenUser={onOpenUser} onOpenUnit={onOpenUnit} />
        )}
        {view === "crypto" && <CryptoView />}
        {view === "memory" && <MemoryView onOpenAgent={onOpenAgent} />}
        {view === "drills" && <DrillsView />}
        {view === "evidence" && <EvidenceView />}
        {view === "remote" && <RemoteView />}
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
        {view === "routines" && <RoutinesView />}
        {view === "copilot" && (
          <CopilotView explainRequest={explainRequest} onExplainRequestHandled={onExplainRequestHandled} />
        )}
      </div>

      <WatchDock onOpenAgent={onOpenAgent} onOpenUnit={onOpenUnit} />

      {focusedIncident && (
        <div className="fixed inset-0 z-50 flex justify-end">
          {/* Same overlay chrome as Agent 360's below, and deliberately its own
              layer rather than a third card in that row: an incident opens
              agents, so the two must be able to sit beside each other. */}
          <button
            type="button"
            aria-label="Close Incident 360"
            className="absolute inset-0"
            style={{ background: "color-mix(in srgb, var(--ink) 55%, transparent)", cursor: "default" }}
            onClick={() => setFocusedIncident(null)}
          />
          <div className="relative h-full flex items-stretch" style={{ padding: 12 }}>
            <Incident360
              row={focusedIncident}
              onClose={() => setFocusedIncident(null)}
              onOpenAgent={onOpenAgent}
              onNavigate={onSelectView}
            />
          </div>
        </div>
      )}
      {visibleAgentIds.length > 0 && (
        <div className="fixed inset-0 z-50 flex justify-end">
          {/* Plain overlay chrome, not itself a dialog - each `Agent360`
              below declares its own `role="dialog"`/`aria-label`, since it
              is the actual dialog surface; this wrapper only supplies the
              shared backdrop and the side-by-side layout. One backdrop
              behind however many cards are open (never one per card) - a
              single click always dismisses ALL of them together, matching
              each card's own Escape handler, which does the same when more
              than one is mounted. */}
          <button
            type="button"
            aria-label="Close Agent 360"
            className="absolute inset-0"
            style={{ background: "color-mix(in srgb, var(--ink) 55%, transparent)", cursor: "default" }}
            onClick={() => setFocusedAgentIds([])}
          />
          <div
            className="relative flex items-stretch"
            style={{ height: "100%", flexDirection: "row", flexWrap: "nowrap", justifyContent: "flex-end" }}
          >
            {/* DOM order is reversed (second, then first) so the row reads
                left to right as [second][first], with the first/anchor
                card flush against the screen edge - "the second immediately
                left of the first, both pinned to the right, no overlap".
                `flexDirection: "row"` + `flexWrap: "nowrap"` (each card's own
                `flexShrink: 0` in `Agent360.tsx` does the rest) keeps both
                cards in a single in-flow row at their own declared width,
                never one absolutely-positioned sibling overlapping another.
                `inCompare` below is the OTHER half of the fix: a SOLO card's
                `min(720px, 94vw)` width (see `Agent360.tsx`'s own width
                comment) is already pinned flat at the literal 720px for
                EVERY viewport >=1200px (94vw only undercuts 720px below
                ~766px, well under `COMPARE_MIN_WIDTH`) - so two solo-style
                cards side by side would demand a fixed 1440px, which
                overflows any compare-eligible viewport from 1200px up to
                1440px even though `COMPARE_MIN_WIDTH` already allowed
                compare mode from 1200px. `inCompare` swaps the vw fallback
                to 46vw instead, so two cards total `min(720px, 46vw) x 2`,
                which is <=92vw for every viewport this branch can ever
                render at (verified: 92vw < 100vw always) - both cards stay
                fully visible side by side rather than the second one
                running off the left edge in that 1200-1440px band. */}
            {visibleAgentIds
              .slice()
              .reverse()
              .map((agentId) => (
                <Agent360
                  key={agentId}
                  agentId={agentId}
                  inCompare={visibleAgentIds.length > 1}
                  onClose={() => onCloseAgent360(agentId)}
                  onOpenAgent={onOpenAgent}
                  onNavigate={onNavigateFromAgent360}
                  onOpenReplay={onOpenReplay}
                />
              ))}
          </div>
        </div>
      )}
    </div>
  );
}
