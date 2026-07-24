import { useCallback, useEffect, useMemo, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { spendByAgent, usd0 } from "../lib/dashData";
import { fetchUnitRecord, type UnitRecord } from "../lib/entityRecords";
import { shortAgentLabel } from "../lib/graph";
import { fetchRuns } from "../lib/money";
import { useMoneyStatus } from "../lib/useMoneyStatus";
import type { Run } from "../moneyTypes";

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
 * path. Units have no bulk "list units" command anywhere in this app (only
 * a per-team lookup), so this looks each pinned unit up individually via
 * `fetchUnitRecord`, the exact call `UnitCard.tsx` already makes for one
 * team at a time.
 *
 * A pinned id absent from whatever the box currently returns (a stale pin,
 * or - for the seed below - a demo id this backend was never going to know
 * about) renders its bare id/name with a muted dash for spend and status,
 * never a crash and never a fabricated number.
 */

const AGENTS_KEY = "genaryx.watch.agents";
const UNITS_KEY = "genaryx.watch.units";
const DOCK_COLLAPSED_KEY = "genaryx.watchDock";
const WATCH_CHANGED_EVENT = "genaryx:watch-changed";

/** Demo seed (Yurii, 2026-07-24): only applied the very first time this app
 * runs on a given browser profile, so a fresh screenshot never shows an
 * empty dock. These ids do not need to resolve against any particular
 * backend's fleet - an unresolved pin is an explicitly supported, honest
 * render state (see the module doc comment above), not a bug. */
const SEED_AGENT_IDS = [
  "agent://meridian.example/kyc-aml/aml-case-copilot",
  "agent://meridian.example/treasury/cashflow-forecaster",
  "agent://meridian.example/treasury/reconciliation-batch",
];
const SEED_UNIT_IDS = ["treasury", "financial-crime"];

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
/** Seeds both sets together, and only the first time this module ever runs
 * in a session, and only when BOTH keys are simultaneously absent. An
 * operator who has unpinned every agent but kept a watched unit (or vice
 * versa) has a real, present empty array for the other kind, not an absent
 * one, so the seed can never quietly reappear once dismissed. */
function ensureSeeded(): void {
  if (seedChecked) return;
  seedChecked = true;
  try {
    const bothAbsent = localStorage.getItem(AGENTS_KEY) === null && localStorage.getItem(UNITS_KEY) === null;
    if (bothAbsent) {
      localStorage.setItem(AGENTS_KEY, JSON.stringify(SEED_AGENT_IDS));
      localStorage.setItem(UNITS_KEY, JSON.stringify(SEED_UNIT_IDS));
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
function toggleWatchedAgent(agentId: string): void {
  const cur = getWatchedAgentIds();
  writeIdArray(AGENTS_KEY, cur.includes(agentId) ? cur.filter((x) => x !== agentId) : [...cur, agentId]);
}
function toggleWatchedUnit(unitId: string): void {
  const cur = getWatchedUnitIds();
  writeIdArray(UNITS_KEY, cur.includes(unitId) ? cur.filter((x) => x !== unitId) : [...cur, unitId]);
}

/** Re-renders whenever either pinned set changes, from any source (this
 * dock's own "x", a "Watch" button elsewhere, or another mounted instance of
 * this same hook). */
function useWatchedIds(): { agentIds: string[]; unitIds: string[] } {
  const [agentIds, setAgentIds] = useState<string[]>(() => getWatchedAgentIds());
  const [unitIds, setUnitIds] = useState<string[]>(() => getWatchedUnitIds());
  useEffect(() => {
    const onChange = () => {
      setAgentIds(getWatchedAgentIds());
      setUnitIds(getWatchedUnitIds());
    };
    window.addEventListener(WATCH_CHANGED_EVENT, onChange);
    return () => window.removeEventListener(WATCH_CHANGED_EVENT, onChange);
  }, []);
  return { agentIds, unitIds };
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
  kind: "agent" | "unit";
  id: string;
  label: string;
}) {
  const { agentIds, unitIds } = useWatchedIds();
  const watched = kind === "agent" ? agentIds.includes(id) : unitIds.includes(id);
  const toggle = kind === "agent" ? toggleWatchedAgent : toggleWatchedUnit;
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

function MutedDash() {
  return (
    <span className="mono" style={{ fontSize: 10.5, color: "var(--faint)" }}>
      -
    </span>
  );
}

function WatchRow({
  kind,
  name,
  spendText,
  hint,
  onOpen,
  onUnpin,
}: {
  kind: "agent" | "unit";
  name: string;
  spendText: string;
  hint: React.ReactNode;
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
          <span className="badge" style={cssVar("tone", kind === "agent" ? "var(--iris)" : "var(--src-qryx)")}>
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
      </button>
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
  );
}

/** How often the agents read (`fetchRuns()`) is re-polled - same cadence
 * `MoneyView.tsx`/`OverviewView.tsx` already use for the identical fetch. */
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

  const { agentIds, unitIds } = useWatchedIds();
  const totalCount = agentIds.length + unitIds.length;

  // Agents: the SAME `fetchRuns()` + `spendByAgent()` pair MoneyView and
  // OverviewView already use for their own "spend by agent" reads.
  const moneyStatus = useMoneyStatus();
  const moneyReady = moneyStatus?.state === "ready";
  const [runs, setRuns] = useState<Run[]>([]);
  useEffect(() => {
    if (!moneyReady) return;
    let cancelled = false;
    const refresh = async () => {
      try {
        const r = await fetchRuns();
        if (!cancelled) setRuns(r);
      } catch {
        // Fail-quiet, same contract as `Agent360.tsx`'s own `fetchRuns()`
        // call: pinned rows simply fall back to their "not yet in the data"
        // dash below rather than surfacing a dock-wide error banner.
      }
    };
    void refresh();
    const id = window.setInterval(() => void refresh(), REFRESH_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [moneyReady]);

  const bySpend = useMemo(() => new Map(spendByAgent(runs).map((a) => [a.agent, a] as const)), [runs]);
  const statusById = useMemo(
    () => new Map(agentIds.map((id) => [id, agentDockStatus(runs.filter((r) => r.agent_id === id))] as const)),
    [agentIds, runs],
  );

  // Units: no bulk "list units" command exists anywhere in this app, so each
  // pinned id is looked up individually with the exact `fetchUnitRecord`
  // call `UnitCard.tsx` already makes for one team at a time.
  const [unitRecords, setUnitRecords] = useState<Map<string, UnitRecord | null>>(new Map());
  useEffect(() => {
    let cancelled = false;
    if (unitIds.length === 0) {
      setUnitRecords(new Map());
      return;
    }
    void Promise.all(unitIds.map(async (id) => [id, await fetchUnitRecord(id)] as const)).then((pairs) => {
      if (!cancelled) setUnitRecords(new Map(pairs));
    });
    return () => {
      cancelled = true;
    };
  }, [unitIds]);

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
        width: 260,
        height: "100%",
        borderLeft: "1px solid var(--line)",
        background: "color-mix(in srgb, var(--panel) 55%, transparent)",
        backdropFilter: "blur(12px) saturate(1.2)",
        WebkitBackdropFilter: "blur(12px) saturate(1.2)",
        transition: "width 0.16s ease",
      }}
    >
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
                <span
                  className="mono"
                  style={{
                    fontSize: 9,
                    letterSpacing: "0.12em",
                    textTransform: "uppercase",
                    color: "var(--faint)",
                    padding: "10px 12px 4px",
                  }}
                >
                  Agents
                </span>
                {agentIds.map((id) => {
                  const spend = bySpend.get(id);
                  const status = statusById.get(id) ?? null;
                  return (
                    <WatchRow
                      key={`agent:${id}`}
                      kind="agent"
                      name={shortAgentLabel(id)}
                      spendText={spend ? usd0(spend.spent) : "-"}
                      hint={status === null ? <MutedDash /> : <span className={`d-pill ${status}`}>{STATUS_LABEL[status]}</span>}
                      onOpen={() => onOpenAgent(id)}
                      onUnpin={() => toggleWatchedAgent(id)}
                    />
                  );
                })}
              </div>
            )}
            {unitIds.length > 0 && (
              <div className="flex flex-col">
                <span
                  className="mono"
                  style={{
                    fontSize: 9,
                    letterSpacing: "0.12em",
                    textTransform: "uppercase",
                    color: "var(--faint)",
                    padding: "10px 12px 4px",
                  }}
                >
                  Units
                </span>
                {unitIds.map((id) => {
                  const rec = unitRecords.get(id);
                  const pct = rec ? unitActivePct(rec) : null;
                  return (
                    <WatchRow
                      key={`unit:${id}`}
                      kind="unit"
                      name={rec?.team ?? id}
                      spendText={rec ? usd0(rec.totalSpentUsd) : "-"}
                      hint={
                        pct === null ? (
                          <MutedDash />
                        ) : (
                          <span className="mono" style={{ fontSize: 10, color: unitActiveTone(pct) }}>
                            {pct}% active
                          </span>
                        )
                      }
                      onOpen={() => onOpenUnit(id)}
                      onUnpin={() => toggleWatchedUnit(id)}
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
