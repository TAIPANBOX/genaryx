import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { userHandle } from "../lib/agentRecord";
import { blockAgent } from "../lib/agentActions";
import { cssVar } from "../lib/cssVars";
import { agentTeam, spendByAgent, usd0 } from "../lib/dashData";
import { blockUnit, blockUser, fetchUnitRecord, fetchUserRecord, type UnitRecord, type UserRecord } from "../lib/entityRecords";
import { formatUsd } from "../lib/format";
import { shortAgentLabel } from "../lib/graph";
import { fetchIdentities } from "../lib/identity";
import { fetchRuns, killRun } from "../lib/money";
import {
  agentBlockedStateFromRuns,
  FreezeToggleButton,
  isUnitStopped,
  isUserStopped,
  KillRunButton,
  StateBadge,
  StopStartButton,
} from "../lib/lifecycle";
import { useConsoleStateVersion } from "../lib/consoleState";
import { useLifecycleBlocks } from "../lib/lifecycleBlocks";
import { usePopover } from "../lib/popover";
import { useIdentityStatus } from "../lib/useIdentityStatus";
import { useMoneyStatus } from "../lib/useMoneyStatus";
import { prettyUnit, unitForTeam } from "../lib/views";
import type { IdryxIdentity } from "../identityTypes";
import type { Run } from "../moneyTypes";
import { UserCard } from "./UserCard";

/**
 * The Watch dock: a right-hand rail where an operator pins agents and/or
 * business units, so their spend and status stay visible at a glance without
 * opening the full Agent 360 card or Unit card every time. Sits to the right
 * of `.main-col` as the last flex child of `.app` (`AppShell.tsx`), mirroring
 * the left rail's own collapsible chrome (`AppHeader.tsx`) but on the other
 * edge of the screen.
 *
 * Pinned ids persist to localStorage as two flat `string[]` (agents, units)
 * rather than one mixed list, so neither needs a tagged shape to round-trip.
 * A pin/unpin from anywhere (this dock's own "x", or the "Watch" button on
 * `Agent360.tsx`/`UnitCard.tsx`) writes through the same helpers below and
 * broadcasts a plain `window` event so every mounted reader picks it up in
 * the SAME tab - the native `storage` event only fires in OTHER tabs, and
 * this dock and a pin button are routinely open together in one tab.
 *
 * Data: agents reuse the exact `fetchRuns()` + `spendByAgent()` pair
 * `MoneyView.tsx`/`OverviewView.tsx` already read their own "spend by agent"
 * from (same fetch, same helper, same 20s refresh cadence) - no new data
 * path. Units reuse the SAME `runs` fetch too (Yurii, 2026-07-24:
 * [`unitSpendFromRuns`]), grouped client-side by `agentTeam()` (the same
 * `agent://org/team/name` parse `spendByAgent`'s own `AgentSpend.team` field
 * already does) - this is the only unit-spend source that answers on a REAL
 * box: `unit_record` (`fetchUnitRecord`, still read below) is a preview-only
 * mock command with no `crates/api`/`crates/web` handler at all, so it
 * resolves `null` there (see its own doc comment in `lib/entityRecords.ts`)
 * and this dock used to show a bare "-" for spend on a real box as a result.
 * `fetchUnitRecord` stays wired as a MOCK-ONLY enrichment layered on top (its
 * richer `owners`/per-agent `blocked` detail has no runs-derived
 * equivalent), preferred only where it actually resolves.
 *
 * A pinned id absent from whatever the box currently returns (a stale pin,
 * or - for the seed below - a demo id this backend was never going to know
 * about) renders its bare id/name with a muted dash for spend and status,
 * never a crash and never a fabricated number. Same tolerance for a unit's
 * used/cap bar: there is no unit-level budget field or command anywhere
 * (mock or real), so the "cap" is the sum of the constituent LIVE runs' own
 * `budget_usd` (a real, already-fetched number, just not literally a
 * unit-level one) when at least one such run carries a budget - the bar
 * simply does not render otherwise, never against a fabricated ceiling.
 *
 * Lifecycle actions (Yurii, 2026-07-24): each row's controls come from the
 * shared `lib/lifecycle.tsx`, the SAME state-driven toggles + state badge the
 * Agent/Unit/User cards use, so the model reads and behaves identically in the
 * dock and in every card. An agent row gets `KillRunButton` (break-glass, the
 * same `killRun`/`money_kill_run` ceremony `RunsBoard.tsx` uses, targeting
 * [`topRunForAgent`]'s live-run pick - `money_kill_run` has a real
 * `crates/web/src/dispatch.rs` handler, so it works end to end on a real box)
 * plus `FreezeToggleButton` (Freeze <-> Unfreeze, `blockAgent`/`agent_block`,
 * a plain confirm). A unit row gets `StopStartButton` (Stop <-> Start,
 * `blockUnit`/`unit_block`), and a user row gets the same (Stop <-> Start,
 * `blockUser`/`user_block`) - users had no per-row action before this. Each
 * row also shows the LIVE/STOPPED/FROZEN/KILLED badge (a blocked agent's badge
 * replaces the utilisation pill; a live one keeps live/near/over). The mutation
 * broadcasts app-wide (`notifyConsoleStateChanged`, inside the command
 * wrappers) so every open panel re-reads within a beat.
 *
 * `agent_block`/`unit_block`/`user_block` are REAL on a box as of 2026-07-24
 * (`crates/web/src/lifecycle.rs`): each writes a deny-all wardryx policy per
 * affected agent, so the block is actually enforced by the PDP and survives a
 * console restart (the policies are the durable record). The box answers the
 * mutation with `null` - it keeps no per-entity record to echo - so these rows
 * read their state from two projections instead: `money_runs` stamps a blocked
 * agent's runs with `Run.lifecycle`, and `lifecycle_blocks` serves the block
 * sets themselves (see `useLifecycleBlocks`), which is what a unit's or user's
 * Stop/Start needs since `unit_record`/`user_record` stay preview-only. In the
 * mock every command mutates the one lifecycle store and every read reflects
 * it, so the demo works end to end the same way.
 *
 * Users (Yurii, 2026-07-24): the third pinnable kind, mirroring agents and
 * units end to end - same localStorage shape (`USERS_KEY`), same seed
 * tolerance, same `WatchToggleButton`/`WatchRow` machinery. `Run`
 * (`moneyTypes.ts`) carries no owner/on_behalf_of field at all, so unlike
 * agents there is no per-user grouping straight off `fetchRuns()` - the
 * join instead goes through `fetchIdentities()`/`identity_list_identities`
 * (`IdryxIdentity.owner`), which - like `money_runs` - has a REAL
 * `crates/web/src/dispatch.rs` handler, so [`userSpendFromRuns`] resolves
 * genuine spend and agent count on a real box too, exactly the way
 * [`unitSpendFromRuns`] does for units, not only on the preview.
 * `fetchUserRecord`/`user_record` (still read below) stays mock-only,
 * layered on top only where the identity join has nothing for a pinned
 * handle, never the primary source - same role it already played for
 * units. `UserCard` is the one card this file imports directly rather than
 * receiving an `onOpen*` callback from `AppShell.tsx` the way agents/units
 * do: `AppShell.tsx` is out of scope for this change, so the open path
 * runs through this file's own `usePopover()` call instead, centered
 * exactly like `onOpenUnit`'s own centered `open(<UnitCard .../>)` call.
 * `UserCard` importing `WatchToggleButton` back from this same file makes
 * a cycle, but the same cycle already exists among `AgentDetailCard`/
 * `UnitCard`/`UserCard` today and works fine there, since nothing on
 * either side is touched at module-evaluation time, only inside a render
 * or click handler.
 *
 * Per-section collapse (Yurii, 2026-07-24): each of the three groups above
 * (Agents/Units/Users) is independently collapsible via its own clickable
 * header (`WatchSectionHeader` - chevron, label, live count), not only the
 * dock as a whole. This is a different axis from the existing whole-dock
 * `collapsed`/`DOCK_COLLAPSED_KEY`: collapsing, say, Units says nothing
 * about Agents or Users, and a fully collapsed dock still hides every
 * section's header too (down to the 44px rail), which collapsing all three
 * sections individually does not. Persisted one flag per section
 * (`*_SECTION_COLLAPSED_KEY`, own `useSectionCollapsed` hook), default
 * expanded, so a first-run operator still sees every pinned row.
 */

const AGENTS_KEY = "genaryx.watch.agents";
const UNITS_KEY = "genaryx.watch.units";
const USERS_KEY = "genaryx.watch.users";
const DOCK_COLLAPSED_KEY = "genaryx.watchDock";
const WATCH_CHANGED_EVENT = "genaryx:watch-changed";

/** Per-SECTION collapse (Yurii, 2026-07-24: "each group collapsible on its
 * own, not just the whole dock"), one key per pinned kind, deliberately
 * separate from `DOCK_COLLAPSED_KEY` above - collapsing the Agents group
 * says nothing about Units or Users, and collapsing every section is still a
 * different state than collapsing the whole dock (the header/count stays
 * visible per section; the whole-dock collapse hides everything, header
 * included, down to a 44px rail). Default expanded, same as the whole dock:
 * `localStorage.getItem(key) === "true"` is false the first time any of
 * these has never been written. */
const AGENTS_SECTION_COLLAPSED_KEY = "genaryx.watch.collapsed.agents";
const UNITS_SECTION_COLLAPSED_KEY = "genaryx.watch.collapsed.units";
const USERS_SECTION_COLLAPSED_KEY = "genaryx.watch.collapsed.users";

/** Drag-resizable dock width (Yurii, 2026-07-24), alongside the existing
 * collapse/expand toggle above - the mirror image of `AppHeader.tsx`'s own
 * `railWidth` on the opposite edge of the screen, same clamp-and-persist
 * shape and own localStorage key. */
const DOCK_WIDTH_KEY = "genaryx.watchDock.width";
const DOCK_MIN_WIDTH = 260;
const DOCK_MAX_WIDTH = 560;
const DOCK_DEFAULT_WIDTH = 260;

function clampWidth(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function readStoredWidth(key: string, fallback: number, min: number, max: number): number {
  try {
    const raw = localStorage.getItem(key);
    if (raw === null) return fallback;
    const parsed = Number(raw);
    return Number.isFinite(parsed) ? clampWidth(parsed, min, max) : fallback;
  } catch {
    return fallback;
  }
}

function writeStoredWidth(key: string, value: number): void {
  try {
    localStorage.setItem(key, String(value));
  } catch {
    // Storage unavailable - the resize still applies for the rest of this
    // session via React state, it just will not survive a reload, same
    // best-effort tolerance every other localStorage write in this app has.
  }
}

/** Demo seed (Yurii, 2026-07-24): only applied the very first time this app
 * runs on a given browser profile, so a fresh screenshot never shows an
 * empty dock. These ids do not need to resolve against any particular
 * backend's fleet - an unresolved pin is an explicitly supported, honest
 * render state (see the module doc comment above), not a bug. */
// The it-rat.com "Live demo" runs this same console under the mock transport
// (VITE_GENARYX_MOCK), against the meridian.io simulated fleet in
// `lib/mockPreview.ts`. The seed below therefore branches: in the demo it
// pins entities that actually resolve to spend in that simulated world, so the
// right-hand dock lands populated (agents near-cap, top units, real owners);
// on a real box it keeps the original org's ids, which resolve there instead.
const IS_MOCK_DEMO = import.meta.env.VITE_GENARYX_MOCK === "1";
const SEED_AGENT_IDS = IS_MOCK_DEMO
  ? [
      "agent://meridian.io/sre/rca-copilot",
      "agent://meridian.io/finops/unit-economics-analyst",
      "agent://meridian.io/sre/runbook-executor",
    ]
  : [
      "agent://meridian.example/kyc-aml/aml-case-copilot",
      "agent://meridian.example/treasury/cashflow-forecaster",
      "agent://meridian.example/treasury/reconciliation-batch",
    ];
const SEED_UNIT_IDS = IS_MOCK_DEMO
  ? ["finops", "sre", "data"]
  : ["corporate-banking", "financial-crime", "finops"];
const SEED_USER_HANDLES = IS_MOCK_DEMO ? ["w.zhang", "j.carter"] : ["d.hayes"];

function readIdArray(key: string): string[] {
  try {
    const raw = localStorage.getItem(key);
    if (raw === null) return [];
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

function writeIdArray(key: string, ids: string[]): void {
  try {
    localStorage.setItem(key, JSON.stringify(ids));
  } catch {
    // localStorage unavailable (private mode, quota) - the pin still applies
    // for the rest of this session via React state, it just will not survive
    // a reload. Best-effort only, same tolerance this app already gives
    // every other browser-storage read/write.
  }
  window.dispatchEvent(new Event(WATCH_CHANGED_EVENT));
}

let seedChecked = false;
/** Seeds all three sets together, and only the first time this module ever
 * runs in a session, and only when EVERY key is simultaneously absent. An
 * operator who has unpinned every agent but kept a watched unit (or vice
 * versa) has a real, present empty array for the other kind, not an absent
 * one, so the seed can never quietly reappear once dismissed - the same
 * tolerance now extended to users rather than re-litigated. */
function ensureSeeded(): void {
  if (seedChecked) return;
  seedChecked = true;
  try {
    const allAbsent =
      localStorage.getItem(AGENTS_KEY) === null &&
      localStorage.getItem(UNITS_KEY) === null &&
      localStorage.getItem(USERS_KEY) === null;
    // Real console: seed only on a truly fresh profile (respects a later
    // dismissal). Demo: re-seed on every load so each visitor lands on the
    // same populated dock regardless of what a previous visitor left behind.
    if (IS_MOCK_DEMO || allAbsent) {
      localStorage.setItem(AGENTS_KEY, JSON.stringify(SEED_AGENT_IDS));
      localStorage.setItem(UNITS_KEY, JSON.stringify(SEED_UNIT_IDS));
      localStorage.setItem(USERS_KEY, JSON.stringify(SEED_USER_HANDLES));
    }
  } catch {
    // no storage available - nothing to seed into, dock just starts empty.
  }
}

function getWatchedAgentIds(): string[] {
  ensureSeeded();
  return readIdArray(AGENTS_KEY);
}
function getWatchedUnitIds(): string[] {
  ensureSeeded();
  return readIdArray(UNITS_KEY);
}
function getWatchedUserHandles(): string[] {
  ensureSeeded();
  return readIdArray(USERS_KEY);
}
function toggleWatchedAgent(agentId: string): void {
  const cur = getWatchedAgentIds();
  writeIdArray(AGENTS_KEY, cur.includes(agentId) ? cur.filter((x) => x !== agentId) : [...cur, agentId]);
}
function toggleWatchedUnit(unitId: string): void {
  const cur = getWatchedUnitIds();
  writeIdArray(UNITS_KEY, cur.includes(unitId) ? cur.filter((x) => x !== unitId) : [...cur, unitId]);
}
function toggleWatchedUser(handle: string): void {
  const cur = getWatchedUserHandles();
  writeIdArray(USERS_KEY, cur.includes(handle) ? cur.filter((x) => x !== handle) : [...cur, handle]);
}

/** Re-renders whenever any pinned set changes, from any source (this
 * dock's own "x", a "Watch" button elsewhere, or another mounted instance of
 * this same hook). */
function useWatchedIds(): { agentIds: string[]; unitIds: string[]; userHandles: string[] } {
  const [agentIds, setAgentIds] = useState<string[]>(() => getWatchedAgentIds());
  const [unitIds, setUnitIds] = useState<string[]>(() => getWatchedUnitIds());
  const [userHandles, setUserHandles] = useState<string[]>(() => getWatchedUserHandles());
  useEffect(() => {
    const onChange = () => {
      setAgentIds(getWatchedAgentIds());
      setUnitIds(getWatchedUnitIds());
      setUserHandles(getWatchedUserHandles());
    };
    window.addEventListener(WATCH_CHANGED_EVENT, onChange);
    return () => window.removeEventListener(WATCH_CHANGED_EVENT, onChange);
  }, []);
  return { agentIds, unitIds, userHandles };
}

/** One pinned-kind section's collapsed flag, read/written under its own
 * `key` (one of the three `*_SECTION_COLLAPSED_KEY` constants above) - the
 * same read-a-string/write-a-string-to-localStorage shape the whole-dock
 * `collapsed`/`toggleCollapsed` pair in `WatchDock` itself uses, just
 * parametrized so the three sections do not need three near-identical copies
 * of this same little state machine inlined in the component. */
function useSectionCollapsed(key: string): [boolean, () => void] {
  const [collapsed, setCollapsed] = useState<boolean>(() => {
    try {
      return localStorage.getItem(key) === "true";
    } catch {
      return false;
    }
  });
  const toggle = useCallback(() => {
    setCollapsed((prev) => {
      const next = !prev;
      try {
        localStorage.setItem(key, next ? "true" : "false");
      } catch {
        // best-effort only, see the module doc comment above.
      }
      return next;
    });
  }, [key]);
  return [collapsed, toggle];
}

/**
 * The "Watch" / "Watching" pin toggle shared by `Agent360.tsx`'s card header
 * and `UnitCard.tsx`'s body - the only two places besides this dock's own
 * unpin "x" that ever write to the pinned sets. Kept minimal on purpose (a
 * text label, no icon) per the brief.
 */
export function WatchToggleButton({
  kind,
  id,
  label,
}: {
  kind: "agent" | "unit" | "user";
  id: string;
  label: string;
}) {
  const { agentIds, unitIds, userHandles } = useWatchedIds();
  const watched = kind === "agent" ? agentIds.includes(id) : kind === "unit" ? unitIds.includes(id) : userHandles.includes(id);
  const toggle = kind === "agent" ? toggleWatchedAgent : kind === "unit" ? toggleWatchedUnit : toggleWatchedUser;
  return (
    <button
      type="button"
      className="icon-btn"
      style={{ width: "auto", padding: "0 10px", fontSize: 11, color: watched ? "var(--iris)" : undefined }}
      aria-pressed={watched}
      aria-label={watched ? `Unpin ${label} from the watch dock` : `Pin ${label} to the watch dock`}
      title={watched ? "Unpin from Watch dock" : "Pin to Watch dock"}
      onClick={() => toggle(id)}
    >
      {watched ? "Watching" : "Watch"}
    </button>
  );
}

function ChevronIcon({ direction }: { direction: "left" | "right" }) {
  return (
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" aria-hidden="true">
      <path
        d={direction === "left" ? "M15 5l-7 7 7 7" : "M9 5l7 7-7 7"}
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** The dock's own drag handle - a thin, invisible-until-hover strip on the
 * dock's inner (left) edge, the mirror image of `AppHeader.tsx`'s
 * `RailResizeHandle` on the opposite edge of the screen. Only rendered while
 * expanded (see the collapsed-branch render below, which never mounts this),
 * so a collapsed dock has no handle to grab, matching the rail's identical
 * rule. */
function DockResizeHandle({
  onPointerDown,
  onPointerMove,
  onPointerUp,
}: {
  onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => void;
  onPointerMove: (e: React.PointerEvent<HTMLDivElement>) => void;
  onPointerUp: (e: React.PointerEvent<HTMLDivElement>) => void;
}) {
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize watch dock"
      title="Drag to resize"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      style={{
        position: "absolute",
        top: 0,
        bottom: 0,
        left: 0,
        width: 6,
        cursor: "col-resize",
        zIndex: 1,
        touchAction: "none",
      }}
      onMouseEnter={(e) => (e.currentTarget.style.background = "color-mix(in srgb, var(--iris) 35%, transparent)")}
      onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
    />
  );
}

/** Same live / near-cap / over-cap / killed classification `RunsBoard.tsx`
 * gives a single run (0.8 and 1.0 fraction thresholds, same four class
 * names), aggregated across every run a pinned agent has in the currently
 * fetched window rather than one run at a time: "dead" only when every run
 * for this agent is killed, otherwise the worst live-run fraction decides.
 * `null` (rendered as a muted dash) means no runs at all for this id in the
 * current fetch - honest "not yet in the data", not a fabricated status. */
type AgentDockStatus = "live" | "near" | "over" | "dead";

const STATUS_LABEL: Record<AgentDockStatus, string> = {
  live: "live",
  near: "near cap",
  over: "over cap",
  dead: "killed",
};

function agentDockStatus(agentRuns: Run[]): AgentDockStatus | null {
  if (agentRuns.length === 0) return null;
  const live = agentRuns.filter((r) => !r.killed);
  if (live.length === 0) return "dead";
  let maxFraction = 0;
  for (const r of live) {
    if (r.budget_usd && r.budget_usd > 0) maxFraction = Math.max(maxFraction, r.spent_usd / r.budget_usd);
  }
  if (maxFraction >= 1) return "over";
  if (maxFraction >= 0.8) return "near";
  return "live";
}

/** What the dock's own "Kill" button targets: the agent's highest-utilisation
 * LIVE run (spent/budget fraction, same measure [`agentDockStatus`] above
 * already ranks by), so one tap goes after the run most likely to be the
 * runaway one rather than an arbitrary one. A run with no budget set counts
 * last, not first, so an uncapped run never outranks a genuinely near- or
 * over-cap one. `null` when the agent has no live run in the currently
 * fetched window (already-killed runs are excluded on purpose - there is
 * nothing left to kill) - the row's Kill button disables itself rather than
 * targeting a stale or already-dead run. */
function topRunForAgent(agentRuns: Run[]): Run | null {
  const live = agentRuns.filter((r) => !r.killed);
  if (live.length === 0) return null;
  const fractionOf = (r: Run) => (r.budget_usd && r.budget_usd > 0 ? r.spent_usd / r.budget_usd : -1);
  let best = live[0];
  let bestFraction = fractionOf(best);
  for (const r of live.slice(1)) {
    const fraction = fractionOf(r);
    if (fraction > bestFraction) {
      best = r;
      bestFraction = fraction;
    }
  }
  return best;
}

/** One unit's real, runs-derived aggregate - see the module doc comment
 * above for why this (not `UnitRecord`/`fetchUnitRecord`) is the number this
 * dock leads with. */
interface UnitSpendAgg {
  /** This window's total spend across every agent whose id parses to this
   * team, straight off `Run.spent_usd` - the same number `spendByAgent`
   * already totals per agent, just grouped one level up. */
  spentUsd: number;
  /** Count of DISTINCT agents (not runs) seen for this team in the current
   * fetch - what the dock shows as "N agents". */
  agentCount: number;
  /** Sum of every LIVE run's own `budget_usd` for this team's agents, when
   * at least one such run carries a budget - the nearest honest, already-real
   * proxy this data model has for "this unit's cap" (there is no literal
   * unit-level budget field or command, mock or real). `null` when not a
   * single live run in the unit has a budget set, so the dock never draws a
   * used/cap bar against a fabricated ceiling. Deliberately summed off the
   * runs directly (not off `spendByAgent`'s per-agent totals): a run's
   * `budget_usd` is a per-run ceiling, not a per-agent one, so an agent with
   * two runs must contribute both runs' budgets, not just one. */
  budgetUsd: number | null;
}

/** Groups `fetchRuns()`'s result by business unit (`agentTeam()` of each
 * run's `agent_id`) - the one unit-spend source that also answers on a real
 * box, since it rides `money_runs` rather than the mock-only `unit_record`.
 * Keyed by the SAME raw team string `agentTeam()`/`UnitRecord.team` use
 * everywhere else in this app (no prettifying here - that is a display-only
 * concern, done once at render time via `prettyUnit`). A team absent from
 * `runs` entirely (no agent in the current fetch belongs to it) is simply
 * absent from the returned map, not a zero entry - callers fall back to
 * `fetchUnitRecord`'s mock enrichment or a muted dash exactly as they would
 * for any other not-yet-resolved pin. */
function unitSpendFromRuns(runs: Run[]): Map<string, UnitSpendAgg> {
  // Keyed by business UNIT (via unitForTeam), not raw team, so a pinned unit
  // id like "financial-crime" matches and multi-team units (fraud + kyc-aml)
  // aggregate into one row instead of never resolving.
  const byTeam = new Map<string, { spentUsd: number; agents: Set<string>; budgetUsd: number; hasBudget: boolean }>();
  for (const agent of spendByAgent(runs)) {
    const unit = unitForTeam(agent.team);
    const entry = byTeam.get(unit) ?? { spentUsd: 0, agents: new Set<string>(), budgetUsd: 0, hasBudget: false };
    entry.spentUsd += agent.spent;
    entry.agents.add(agent.agent);
    byTeam.set(unit, entry);
  }
  for (const r of runs) {
    if (r.killed || r.budget_usd === null || r.budget_usd <= 0) continue;
    const entry = byTeam.get(unitForTeam(agentTeam(r.agent_id)));
    if (!entry) continue; // agentTeam() disagreeing with spendByAgent() above never happens (same parse), but never trust that silently.
    entry.budgetUsd += r.budget_usd;
    entry.hasBudget = true;
  }
  const out = new Map<string, UnitSpendAgg>();
  for (const [team, entry] of byTeam) {
    // budgetUsd stays null on purpose: summing per-run ceilings is not a unit
    // monthly cap (it produced absurd "33000%" bars), and no real unit-cap
    // field/command exists client-side. Show the spend number + agent count,
    // not a fabricated used/cap bar.
    void entry.hasBudget;
    out.set(team, { spentUsd: entry.spentUsd, agentCount: entry.agents.size, budgetUsd: null });
  }
  return out;
}

/** One user's real, runs-derived aggregate - the identity-joined mirror of
 * [`UnitSpendAgg`] above. No budget field: same as units, there is no
 * per-user budget/ceiling anywhere in this data model, mock or real. */
interface UserSpendAgg {
  spentUsd: number;
  agentCount: number;
}

/** Groups `fetchRuns()`'s result by OWNER, joined through
 * `fetchIdentities()`'s own `owner` field rather than anything on `Run`
 * itself - `Run` (`moneyTypes.ts`) carries no owner/on_behalf_of field at
 * all, unlike `agent_id` (`agentTeam`) which [`unitSpendFromRuns`] above
 * groups by directly. `identity_list_identities`, like `money_runs`, has a
 * REAL `crates/web/src/dispatch.rs` handler (unlike the mock-only
 * `user_record`/`unit_record`), so this resolves genuine spend and distinct
 * agent count on a real box too, not just the preview. Mock owners come
 * back `user://<org>/<handle>` (`userId()`, `lib/mockPreview.ts`); a real
 * idryx owner may already be a bare handle - `userHandle()`
 * (`lib/agentRecord.ts`, the SAME parse `AgentDetailCard.tsx`'s own "owner"
 * field already uses) takes the last path segment either way, so both
 * shapes land on the exact key `UserCard`/`WatchToggleButton` use. An
 * identity with no owner at all (`""`) contributes no entry, never a fake
 * "" user; an agent whose id is simply absent from the current identities
 * fetch is likewise excluded, not zero-charged to a guessed owner. */
function userSpendFromRuns(runs: Run[], identities: IdryxIdentity[]): Map<string, UserSpendAgg> {
  const ownerByAgent = new Map<string, string>();
  for (const identity of identities) {
    if (identity.owner) ownerByAgent.set(identity.id, userHandle(identity.owner));
  }
  const byOwner = new Map<string, { spentUsd: number; agents: Set<string> }>();
  for (const agent of spendByAgent(runs)) {
    const owner = ownerByAgent.get(agent.agent);
    if (!owner) continue;
    const entry = byOwner.get(owner) ?? { spentUsd: 0, agents: new Set<string>() };
    entry.spentUsd += agent.spent;
    entry.agents.add(agent.agent);
    byOwner.set(owner, entry);
  }
  const out = new Map<string, UserSpendAgg>();
  for (const [owner, entry] of byOwner) {
    out.set(owner, { spentUsd: entry.spentUsd, agentCount: entry.agents.size });
  }
  return out;
}

/** Units carry no per-unit budget in this data model (`UnitRecord` has none -
 * see `lib/entityRecords.ts`), so "percent of cap" has no literal field to
 * read. The nearest honest, already-available proxy for "at a glance
 * operating health" is the share of the unit's own agents that are neither
 * blocked nor closed - derived from data `UnitCard.tsx` already fetches for
 * the same team, nothing new. `null` (muted dash) when the unit has no
 * agents to compute a share from. */
function unitActivePct(rec: UnitRecord): number | null {
  if (rec.agents.length === 0) return null;
  const active = rec.agents.filter((a) => !a.blocked && !a.closed).length;
  return Math.round((active / rec.agents.length) * 100);
}

function unitActiveTone(pct: number): string {
  if (pct >= 100) return "var(--mint)";
  if (pct <= 0) return "var(--sev-critical)";
  return "var(--amber)";
}

/** The unit row's used/cap bar - only ever rendered when [`UnitSpendAgg.budgetUsd`]
 * is non-null (see that field's own doc comment for what "cap" means here).
 * Same 80%/100% amber/critical thresholds `RunsBoard.tsx`'s per-run bars and
 * this file's own [`agentDockStatus`] already use, so a unit nearing or over
 * its constituent runs' combined budget reads the same as a single run doing
 * the same would. */
function UnitCapBar({ usedUsd, capUsd }: { usedUsd: number; capUsd: number }) {
  const pct = capUsd > 0 ? Math.round((usedUsd / capUsd) * 100) : 0;
  const tone = pct >= 100 ? "var(--sev-critical)" : pct >= 80 ? "var(--amber)" : "var(--mint)";
  return (
    <span
      className="flex items-center gap-1.5"
      style={{ width: "100%" }}
      title={`${usd0(usedUsd)} of ${usd0(capUsd)} budgeted across this unit's live runs`}
    >
      <span style={{ flex: 1, height: 4, borderRadius: 999, background: "var(--panel-3)", overflow: "hidden" }}>
        <span
          aria-hidden="true"
          style={{ display: "block", height: "100%", width: `${Math.min(100, pct)}%`, background: tone, borderRadius: 999 }}
        />
      </span>
      <span className="mono" style={{ fontSize: 9.5, color: "var(--faint)" }}>
        {pct}%
      </span>
    </span>
  );
}

function MutedDash() {
  return (
    <span className="mono" style={{ fontSize: 10.5, color: "var(--faint)" }}>
      -
    </span>
  );
}

/** One pinned-kind group's own clickable header - the chevron + label + live
 * count row that toggles just THIS section's rows between shown and hidden
 * (Yurii, 2026-07-24: "each of the three groups collapsible on its own").
 * Reuses the file's existing [`ChevronIcon`] rather than a second icon,
 * rotated 90deg for "expanded" instead of swapping to a visually different
 * glyph - `direction="right"` already points the way a collapsed section's
 * disclosure triangle should. Typography matches the plain section labels
 * this replaces exactly (`mono`, 9px, uppercase, 0.12em tracking, `--faint`),
 * so a section's own look is unchanged apart from gaining the chevron and
 * the count. */
function WatchSectionHeader({
  label,
  count,
  collapsed,
  onToggle,
}: {
  label: string;
  count: number;
  collapsed: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      className="flex items-center gap-1.5"
      style={{
        width: "100%",
        background: "none",
        border: "none",
        padding: "10px 12px 4px",
        cursor: "pointer",
        textAlign: "left",
      }}
      aria-expanded={!collapsed}
      title={collapsed ? `Expand ${label}` : `Collapse ${label}`}
    >
      <span
        aria-hidden="true"
        style={{
          display: "inline-flex",
          color: "var(--faint)",
          transform: collapsed ? undefined : "rotate(90deg)",
          transition: "transform 0.12s ease",
        }}
      >
        <ChevronIcon direction="right" />
      </span>
      <span className="mono" style={{ fontSize: 9, letterSpacing: "0.12em", textTransform: "uppercase", color: "var(--faint)" }}>
        {label}
      </span>
      <span className="mono" style={{ fontSize: 9, color: "var(--faint)" }}>
        {count}
      </span>
    </button>
  );
}

function WatchRow({
  kind,
  name,
  spendText,
  hint,
  extra,
  action,
  onOpen,
  onUnpin,
}: {
  kind: "agent" | "unit" | "user";
  name: string;
  spendText: string;
  hint: React.ReactNode;
  /** An optional third line under spend/hint - today only a unit row's
   * [`UnitCapBar`] (Yurii, 2026-07-24), rendered only when a real cap number
   * is known. `undefined` for every agent row (and any unit with no known
   * budget), which renders nothing extra - agent rows are pixel-identical to
   * before this existed. */
  extra?: React.ReactNode;
  /** The row's own small destructive control (Kill for an agent, Freeze for
   * a unit) - optional only so `WatchRow` itself stays generically reusable;
   * both callers below always pass one. Rendered beside the unpin "x" as a
   * sibling of the open-button, never nested inside it, so tapping it never
   * also opens the full card. */
  action?: React.ReactNode;
  onOpen: () => void;
  onUnpin: () => void;
}) {
  return (
    <div className="flex items-start gap-1.5" style={{ padding: "7px 6px 7px 12px", borderBottom: "1px solid var(--line)" }}>
      <button
        type="button"
        onClick={onOpen}
        className="flex-1 min-w-0 flex flex-col gap-1"
        style={{ background: "none", border: "none", padding: 0, textAlign: "left", cursor: "pointer" }}
        title={`Open ${name}`}
      >
        <span className="flex items-center gap-1.5 min-w-0">
          <span
            className="badge"
            style={cssVar("tone", kind === "agent" ? "var(--iris)" : kind === "unit" ? "var(--src-qryx)" : "var(--src-engram)")}
          >
            {kind}
          </span>
          <span className="truncate" style={{ fontSize: 11.5, color: "var(--fg)" }}>
            {name}
          </span>
        </span>
        <span className="flex items-center gap-2">
          <span className="mono tabular" style={{ fontSize: 11, color: "var(--dim)" }}>
            {spendText}
          </span>
          {hint}
        </span>
        {extra}
      </button>
      <div className="flex items-center gap-1" style={{ flexShrink: 0 }}>
        {action}
        <button
          type="button"
          className="icon-btn"
          style={{ width: 20, height: 20, fontSize: 11, flexShrink: 0, padding: 0 }}
          onClick={onUnpin}
          aria-label={`Unpin ${name}`}
          title="Unpin"
        >
          &times;
        </button>
      </div>
    </div>
  );
}

// The dock's per-row lifecycle controls now come from the shared
// `lib/lifecycle.tsx` (Yurii, 2026-07-24: "consistent everywhere"): agent rows
// get `KillRunButton` (break-glass) + `FreezeToggleButton` (plain confirm),
// unit rows get `StopStartButton`, and user rows get `StopStartButton` too -
// the exact same components the Agent/Unit/User cards use, so the model reads
// and behaves identically in the dock and in every card. A unit/user Stop
// still has no `crates/web` handler on a real box today, so it stays a visible
// no-op there until one ships (the dock reflects the command's response, never
// a faked local mutation); the agent Kill already works end to end on a real
// box via `money_kill_run`.

/** How often the agents read (`fetchRuns()`) and the pinned units read
 * (`fetchUnitRecord()` per id) are re-polled - same cadence
 * `MoneyView.tsx`/`OverviewView.tsx` already use for the identical runs
 * fetch. Units share the same cadence too, so a Freeze (or any other
 * out-of-band change to a pinned unit) is reflected here on its own, without
 * the operator having to unpin/re-pin it to force a fresh look. */
const REFRESH_INTERVAL_MS = 20_000;

export function WatchDock({
  onOpenAgent,
  onOpenUnit,
}: {
  /** Opens the full Agent 360 overlay - the exact same callback
   * `AppShell.tsx` already threads to every other "open agent" entry point. */
  onOpenAgent: (agentId: string) => void;
  /** Opens the unit's detail card - `AppShell.tsx` wires this to a centered
   * `UnitCard` popover, the same component every other unit link opens. */
  onOpenUnit: (unitId: string) => void;
}) {
  const [collapsed, setCollapsed] = useState<boolean>(() => {
    try {
      return localStorage.getItem(DOCK_COLLAPSED_KEY) === "true";
    } catch {
      return false;
    }
  });
  const toggleCollapsed = useCallback(() => {
    setCollapsed((prev) => {
      const next = !prev;
      try {
        localStorage.setItem(DOCK_COLLAPSED_KEY, next ? "true" : "false");
      } catch {
        // best-effort only, see the module doc comment above.
      }
      return next;
    });
  }, []);

  // Per-section collapse (Yurii, 2026-07-24: "each of the three groups
  // collapsible on its own"), independent of `collapsed`/`toggleCollapsed`
  // above - see [`useSectionCollapsed`]'s own doc comment for why this is
  // three calls to one small hook rather than three inlined copies of the
  // same state machine.
  const [agentsSectionCollapsed, toggleAgentsSection] = useSectionCollapsed(AGENTS_SECTION_COLLAPSED_KEY);
  const [unitsSectionCollapsed, toggleUnitsSection] = useSectionCollapsed(UNITS_SECTION_COLLAPSED_KEY);
  const [usersSectionCollapsed, toggleUsersSection] = useSectionCollapsed(USERS_SECTION_COLLAPSED_KEY);

  // Drag-resizable width (Yurii, 2026-07-24: "resizable in addition to
  // collapsible"), the mirror image of `AppHeader.tsx`'s `railWidth` on the
  // opposite edge of the screen - same clamp-and-persist shape, own
  // localStorage key, only read/written here since nothing outside this
  // component needs the number. `dragging` suppresses the width transition
  // below while a drag is live so the dock tracks the pointer 1:1 instead of
  // lagging through a 0.16s ease; the drag start point/width live in a plain
  // ref (written on every pointermove, never read by render) rather than
  // state, so a drag does not re-run this whole component's render once per
  // pixel of movement.
  const [dockWidth, setDockWidth] = useState<number>(() =>
    readStoredWidth(DOCK_WIDTH_KEY, DOCK_DEFAULT_WIDTH, DOCK_MIN_WIDTH, DOCK_MAX_WIDTH),
  );
  const [dockDragging, setDockDragging] = useState(false);
  const dockDragRef = useRef<{ startX: number; startWidth: number } | null>(null);
  const onDockHandlePointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      e.preventDefault();
      e.currentTarget.setPointerCapture(e.pointerId);
      dockDragRef.current = { startX: e.clientX, startWidth: dockWidth };
      setDockDragging(true);
    },
    [dockWidth],
  );
  const onDockHandlePointerMove = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    const drag = dockDragRef.current;
    if (!drag) return;
    // The dock sits on the RIGHT edge of the screen and its handle is on its
    // LEFT inner edge, so dragging LEFT (negative clientX delta) is what
    // widens it - the opposite sign from the rail's own handle on the other
    // side of the screen.
    setDockWidth(clampWidth(drag.startWidth + (drag.startX - e.clientX), DOCK_MIN_WIDTH, DOCK_MAX_WIDTH));
  }, []);
  const onDockHandlePointerUp = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    if (!dockDragRef.current) return;
    dockDragRef.current = null;
    setDockDragging(false);
    setDockWidth((w) => {
      writeStoredWidth(DOCK_WIDTH_KEY, w);
      return w;
    });
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {
      // Already released (e.g. the pointer left the window mid-drag).
    }
  }, []);

  const { agentIds, unitIds, userHandles } = useWatchedIds();
  const totalCount = agentIds.length + unitIds.length + userHandles.length;
  // User rows open `UserCard` straight through this dock's own popover
  // rather than an `onOpen*` callback prop - see the module doc comment
  // above for why (this file importing `UserCard` directly, unlike the
  // agent/unit callbacks `AppShell.tsx` supplies).
  const { open } = usePopover();

  // Agents: the SAME `fetchRuns()` + `spendByAgent()` pair MoneyView and
  // OverviewView already use for their own "spend by agent" reads.
  // `refreshRuns` is pulled out of the effect (rather than a fetch closure
  // the effect owns alone) so the post-Kill handler further down can trigger
  // the exact same re-fetch on demand, instead of waiting for the next
  // [`REFRESH_INTERVAL_MS`] tick to notice the run it just killed.
  const moneyStatus = useMoneyStatus();
  const moneyReady = moneyStatus?.state === "ready";
  const [runs, setRuns] = useState<Run[]>([]);
  const refreshRuns = useCallback(async () => {
    try {
      setRuns(await fetchRuns());
    } catch {
      // Fail-quiet, same contract as `Agent360.tsx`'s own `fetchRuns()`
      // call: pinned rows simply fall back to their "not yet in the data"
      // dash below rather than surfacing a dock-wide error banner.
    }
  }, []);
  // Bumps whenever any lifecycle action lands anywhere (a Stop/Freeze/Kill from
  // a card or from this dock), so the dock re-reads runs/records within a beat
  // rather than only on the 20s poll - added to each read effect's deps below.
  const consoleVersion = useConsoleStateVersion();
  useEffect(() => {
    if (!moneyReady) return;
    void refreshRuns();
    const id = window.setInterval(() => void refreshRuns(), REFRESH_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [moneyReady, refreshRuns, consoleVersion]);

  const bySpend = useMemo(() => new Map(spendByAgent(runs).map((a) => [a.agent, a] as const)), [runs]);
  // Per-agent blocked lifecycle (STOPPED/FROZEN/KILLED) from the same `runs`,
  // so an agent row shows its state badge and its Freeze button reads the
  // opposite of the current state.
  // What a REAL box says is blocked (`lifecycle_blocks`). Merged with the
  // runs-derived map below because the two cover different gaps: runs carry an
  // agent's state on a box but say nothing about a unit or user, and an agent
  // with no runs in the window has no row to stamp at all.
  const serverBlocks = useLifecycleBlocks();
  const blockedByAgent = useMemo(() => {
    const derived = agentBlockedStateFromRuns(runs);
    for (const id of serverBlocks.agents) if (!derived.has(id)) derived.set(id, "frozen");
    return derived;
  }, [runs, serverBlocks]);
  const statusById = useMemo(
    () => new Map(agentIds.map((id) => [id, agentDockStatus(runs.filter((r) => r.agent_id === id))] as const)),
    [agentIds, runs],
  );
  // What each pinned agent's own "Kill" button targets - see
  // `topRunForAgent`'s own doc comment.
  const topRunById = useMemo(
    () => new Map(agentIds.map((id) => [id, topRunForAgent(runs.filter((r) => r.agent_id === id))] as const)),
    [agentIds, runs],
  );
  // Units' real, real-box-safe spend/count/budget - grouped from the SAME
  // `runs` fetch the agents section above already holds (see
  // `unitSpendFromRuns`'s own doc comment). Every pinned unit looks itself up
  // by team here FIRST; `unitRecords` below is the mock-only enrichment layered
  // on top, not the primary source.
  const unitAggByTeam = useMemo(() => unitSpendFromRuns(runs), [runs]);
  // The dock's own "Kill": the exact `killRun`/`money_kill_run` break-glass
  // call `RunsBoard.tsx` already uses, then an immediate `refreshRuns()`
  // (rather than waiting for the next poll) so the row's status/spend
  // reflects the kill right away. Left to throw on failure: `ConfirmButton`'s
  // own break-glass ceremony already handles a rejected `onConfirm` (see its
  // doc comment) exactly the same way every other caller in this app relies
  // on, so this needs no bespoke error handling of its own.
  const handleKillTopRun = useCallback(
    async (runId: string, reason: string) => {
      await killRun(runId, reason);
      await refreshRuns();
    },
    [refreshRuns],
  );
  // The agent row's Freeze <-> Unfreeze toggle: `blockAgent`/`agent_block`,
  // then an immediate `refreshRuns()` so the row's badge/status catches up
  // without waiting for the poll (and `blockAgent` also broadcasts app-wide).
  const handleToggleFreeze = useCallback(
    async (agentId: string, frozen: boolean) => {
      await blockAgent(agentId, !frozen);
      await refreshRuns();
    },
    [refreshRuns],
  );

  // Units' MOCK-ONLY enrichment: no bulk "list units" command exists
  // anywhere in this app, so each pinned id is ALSO looked up individually
  // with the exact `fetchUnitRecord` call `UnitCard.tsx` already makes for
  // one team at a time - this resolves to `null` on a real box (see the
  // module doc comment above), where `unitAggByTeam` is the ONLY thing that
  // resolves. Re-polled on the SAME [`REFRESH_INTERVAL_MS`] cadence the
  // agents read above uses, so on the mock backend a unit's owners and its
  // Freeze button's "Frozen" state both catch up on their own rather than
  // only on the next pin/unpin.
  const [unitRecords, setUnitRecords] = useState<Map<string, UnitRecord | null>>(new Map());
  const refreshUnitRecords = useCallback(async (ids: string[]) => {
    if (ids.length === 0) {
      setUnitRecords(new Map());
      return;
    }
    const pairs = await Promise.all(ids.map(async (id) => [id, await fetchUnitRecord(id)] as const));
    setUnitRecords(new Map(pairs));
  }, []);
  useEffect(() => {
    void refreshUnitRecords(unitIds);
    if (unitIds.length === 0) return;
    const id = window.setInterval(() => void refreshUnitRecords(unitIds), REFRESH_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [unitIds, refreshUnitRecords, consoleVersion]);
  // The dock's own unit Stop <-> Start: `blockUnit`/`unit_block`, patching just
  // the affected unit's entry from the returned record so the row's state
  // updates immediately rather than waiting for the poll (and `blockUnit` also
  // broadcasts app-wide). Mirrors `UnitCard.tsx`'s own toggle: a `null` result
  // (no backend store for this - today EVERY real box) leaves state untouched
  // rather than pretending the change landed.
  const handleToggleUnitStop = useCallback(async (team: string, stopped: boolean) => {
    const updated = await blockUnit(team, !stopped);
    if (updated) setUnitRecords((prev) => new Map(prev).set(team, updated));
  }, []);

  // Users' real, real-box-safe spend/count - joined from the SAME `runs`
  // fetch above through `fetchIdentities()` (see `userSpendFromRuns`'s own
  // doc comment for why the identity plane, not `Run` itself, is the join).
  // Gated on its own `identity_status` the same way the runs fetch above is
  // gated on `money_status` - a separate backend plane, so a separate
  // readiness check.
  const identityStatus = useIdentityStatus();
  const identityReady = identityStatus?.state === "ready";
  const [identities, setIdentities] = useState<IdryxIdentity[]>([]);
  const refreshIdentities = useCallback(async () => {
    try {
      setIdentities(await fetchIdentities());
    } catch {
      // Fail-quiet, same contract as `refreshRuns` above: pinned user rows
      // simply fall back to `fetchUserRecord`'s mock enrichment or a muted
      // dash rather than surfacing a dock-wide error banner.
    }
  }, []);
  useEffect(() => {
    if (!identityReady) return;
    void refreshIdentities();
    const id = window.setInterval(() => void refreshIdentities(), REFRESH_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [identityReady, refreshIdentities]);
  const userAggByHandle = useMemo(() => userSpendFromRuns(runs, identities), [runs, identities]);

  // Users' MOCK-ONLY enrichment - the exact same role `unitRecords` above
  // plays for units: `fetchUserRecord`/`user_record` resolves to `null` on
  // a real box (see the module doc comment above), where `userAggByHandle`
  // is the ONLY thing that resolves there. Re-polled on the SAME
  // [`REFRESH_INTERVAL_MS`] cadence, ungated on any status, mirroring
  // `refreshUnitRecords` precisely.
  const [userRecords, setUserRecords] = useState<Map<string, UserRecord | null>>(new Map());
  const refreshUserRecords = useCallback(async (handles: string[]) => {
    if (handles.length === 0) {
      setUserRecords(new Map());
      return;
    }
    const pairs = await Promise.all(handles.map(async (h) => [h, await fetchUserRecord(h)] as const));
    setUserRecords(new Map(pairs));
  }, []);
  useEffect(() => {
    void refreshUserRecords(userHandles);
    if (userHandles.length === 0) return;
    const id = window.setInterval(() => void refreshUserRecords(userHandles), REFRESH_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [userHandles, refreshUserRecords, consoleVersion]);
  // The dock's own user Stop <-> Start: `blockUser`/`user_block`, mirroring
  // `handleToggleUnitStop` exactly (patch the affected handle's entry, honest
  // no-op on a null result). Users had no destructive action here before
  // (Yurii, 2026-07-24: "consistent everywhere").
  const handleToggleUserStop = useCallback(async (handle: string, stopped: boolean) => {
    const updated = await blockUser(handle, !stopped);
    if (updated) setUserRecords((prev) => new Map(prev).set(handle, updated));
  }, []);

  if (collapsed) {
    return (
      <aside
        className="flex flex-col items-center shrink-0"
        aria-label="Watch dock (collapsed)"
        style={{
          width: 44,
          height: "100%",
          borderLeft: "1px solid var(--line)",
          background: "color-mix(in srgb, var(--panel) 55%, transparent)",
          backdropFilter: "blur(12px) saturate(1.2)",
          WebkitBackdropFilter: "blur(12px) saturate(1.2)",
          transition: "width 0.16s ease",
          paddingTop: 14,
          gap: 10,
        }}
      >
        <button
          type="button"
          className="icon-btn"
          style={{ width: 26, height: 26 }}
          onClick={toggleCollapsed}
          aria-label="Expand watch dock"
          title="Expand watch dock"
        >
          <ChevronIcon direction="left" />
        </button>
        {totalCount > 0 && (
          <span
            className="mono"
            style={{
              fontSize: 10,
              fontWeight: 700,
              lineHeight: 1,
              padding: "3px 6px",
              borderRadius: 999,
              background: "var(--panel-3)",
              border: "1px solid var(--line-2)",
              color: "var(--dim)",
            }}
            title={`${totalCount} pinned`}
          >
            {totalCount}
          </span>
        )}
      </aside>
    );
  }

  return (
    <aside
      className="flex flex-col shrink-0"
      aria-label="Watch dock"
      style={{
        position: "relative",
        width: dockWidth,
        height: "100%",
        borderLeft: "1px solid var(--line)",
        background: "color-mix(in srgb, var(--panel) 55%, transparent)",
        backdropFilter: "blur(12px) saturate(1.2)",
        WebkitBackdropFilter: "blur(12px) saturate(1.2)",
        transition: dockDragging ? undefined : "width 0.16s ease",
      }}
    >
      <DockResizeHandle onPointerDown={onDockHandlePointerDown} onPointerMove={onDockHandlePointerMove} onPointerUp={onDockHandlePointerUp} />
      <div className="flex items-center gap-2 px-3 shrink-0" style={{ height: 44, borderBottom: "1px solid var(--line)" }}>
        <button
          type="button"
          className="icon-btn"
          style={{ width: 24, height: 24 }}
          onClick={toggleCollapsed}
          aria-label="Collapse watch dock"
          title="Collapse watch dock"
        >
          <ChevronIcon direction="right" />
        </button>
        <span className="mono" style={{ fontSize: 11, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}>
          Watch
        </span>
        {totalCount > 0 && (
          <span className="mono" style={{ fontSize: 10, color: "var(--faint)" }}>
            {totalCount}
          </span>
        )}
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto thin-scroll">
        {totalCount === 0 ? (
          <div className="mono" style={{ fontSize: 11.5, color: "var(--faint)", padding: "20px 16px", textAlign: "center" }}>
            Pin an agent or unit to watch it here.
          </div>
        ) : (
          <>
            {agentIds.length > 0 && (
              <div className="flex flex-col">
                <WatchSectionHeader
                  label="Agents"
                  count={agentIds.length}
                  collapsed={agentsSectionCollapsed}
                  onToggle={toggleAgentsSection}
                />
                {!agentsSectionCollapsed &&
                  agentIds.map((id) => {
                    const spend = bySpend.get(id);
                    const status = statusById.get(id) ?? null;
                    const topRun = topRunById.get(id) ?? null;
                    // A blocked agent shows its lifecycle badge; a live one keeps
                    // the utilisation pill (live/near/over) that flags a runaway.
                    const blocked = blockedByAgent.get(id) ?? null;
                    const frozen = blocked === "frozen";
                    return (
                      <WatchRow
                        key={`agent:${id}`}
                        kind="agent"
                        name={shortAgentLabel(id)}
                        spendText={spend ? usd0(spend.spent) : "-"}
                        hint={
                          blocked ? (
                            <StateBadge state={blocked} />
                          ) : status === null ? (
                            <MutedDash />
                          ) : (
                            <span className={`d-pill ${status}`}>{STATUS_LABEL[status]}</span>
                          )
                        }
                        action={
                          <>
                            <KillRunButton
                              run={topRun}
                              detail={topRun ? `run ${topRun.run_id} · spent ${formatUsd(topRun.spent_usd)}` : undefined}
                              onKill={handleKillTopRun}
                            />
                            <FreezeToggleButton frozen={frozen} onToggle={() => handleToggleFreeze(id, frozen)} />
                          </>
                        }
                        onOpen={() => onOpenAgent(id)}
                        onUnpin={() => toggleWatchedAgent(id)}
                      />
                    );
                  })}
              </div>
            )}
            {unitIds.length > 0 && (
              <div className="flex flex-col">
                <WatchSectionHeader
                  label="Units"
                  count={unitIds.length}
                  collapsed={unitsSectionCollapsed}
                  onToggle={toggleUnitsSection}
                />
                {!unitsSectionCollapsed &&
                  unitIds.map((id) => {
                    // Real, real-box-safe numbers first (`unitAggByTeam`,
                    // grouped from `runs`); `rec` (`fetchUnitRecord`, mock
                    // only) fills in ONLY where the aggregate has nothing for
                    // this id - see the module doc comment for why the order
                    // is this way round, not the reverse.
                    const rec = unitRecords.get(id);
                    const agg = unitAggByTeam.get(id) ?? null;
                    const spentUsd = agg ? agg.spentUsd : (rec?.totalSpentUsd ?? null);
                    const agentCount = agg ? agg.agentCount : (rec ? rec.agents.length : null);
                    const capUsd = agg?.budgetUsd ?? null; // `rec` has no budget field at all - see `UnitSpendAgg.budgetUsd`'s doc comment.
                    const activePct = rec ? unitActivePct(rec) : null;
                    const agentCountLabel = agentCount === null ? null : `${agentCount} agent${agentCount === 1 ? "" : "s"}`;
                    const hintText =
                      agentCountLabel === null ? null : activePct === null ? agentCountLabel : `${agentCountLabel} · ${activePct}% active`;
                    const stopped = isUnitStopped(rec) || serverBlocks.units.includes(id);
                    return (
                      <WatchRow
                        key={`unit:${id}`}
                        kind="unit"
                        name={prettyUnit(rec?.team ?? id)}
                        spendText={spentUsd === null ? "-" : usd0(spentUsd)}
                        hint={
                          <span className="flex items-center gap-1.5">
                            {stopped && <StateBadge state="stopped" />}
                            {hintText === null ? (
                              <MutedDash />
                            ) : (
                              <span className="mono" style={{ fontSize: 10, color: activePct === null ? "var(--faint)" : unitActiveTone(activePct) }}>
                                {hintText}
                              </span>
                            )}
                          </span>
                        }
                        extra={capUsd !== null && spentUsd !== null ? <UnitCapBar usedUsd={spentUsd} capUsd={capUsd} /> : undefined}
                        action={<StopStartButton stopped={stopped} onToggle={() => handleToggleUnitStop(id, stopped)} />}
                        onOpen={() => onOpenUnit(id)}
                        onUnpin={() => toggleWatchedUnit(id)}
                      />
                    );
                  })}
              </div>
            )}
            {userHandles.length > 0 && (
              <div className="flex flex-col">
                <WatchSectionHeader
                  label="Users"
                  count={userHandles.length}
                  collapsed={usersSectionCollapsed}
                  onToggle={toggleUsersSection}
                />
                {!usersSectionCollapsed &&
                  userHandles.map((handle) => {
                    // Real, real-box-safe numbers first (`userAggByHandle`,
                    // joined from `runs` + `identities`); `rec`
                    // (`fetchUserRecord`, mock only) fills in ONLY where the
                    // aggregate has nothing for this handle - same order, and
                    // same reason, as the units section above.
                    const rec = userRecords.get(handle);
                    const agg = userAggByHandle.get(handle) ?? null;
                    const spentUsd = agg ? agg.spentUsd : (rec?.totalSpentUsd ?? null);
                    const agentCount = agg ? agg.agentCount : (rec ? rec.agents.length : null);
                    const agentCountLabel = agentCount === null ? null : `${agentCount} agent${agentCount === 1 ? "" : "s"}`;
                    const stopped = isUserStopped(rec) || serverBlocks.users.includes(handle);
                    return (
                      <WatchRow
                        key={`user:${handle}`}
                        kind="user"
                        name={rec?.handle ?? handle}
                        spendText={spentUsd === null ? "-" : usd0(spentUsd)}
                        hint={
                          <span className="flex items-center gap-1.5">
                            {stopped && <StateBadge state="stopped" />}
                            {agentCountLabel === null ? (
                              <MutedDash />
                            ) : (
                              <span className="mono" style={{ fontSize: 10, color: "var(--faint)" }}>
                                {agentCountLabel}
                              </span>
                            )}
                          </span>
                        }
                        action={<StopStartButton stopped={stopped} onToggle={() => handleToggleUserStop(handle, stopped)} />}
                        onOpen={() => open(<UserCard handle={handle} onOpenFullAgent={onOpenAgent} />)}
                        onUnpin={() => toggleWatchedUser(handle)}
                      />
                    );
                  })}
              </div>
            )}
          </>
        )}
      </div>
    </aside>
  );
}
