import { hasBackend, invokeBackend } from "./transport";
import { spendByAgent, agentTeam } from "./dashData";
import { ownerByAgent } from "./entityFolds";
import { unitForTeam } from "./views";
import type { AgentStats, StatsError, StatsPanel } from "../statsTypes";
import type { IdryxIdentity } from "../identityTypes";
import type { Owner, Run } from "../moneyTypes";

/**
 * The Statistics view's data layer: one fold, three keys.
 *
 * # WHY ONE FOLD AND NOT THREE
 *
 * Grouping by agent, by owner and by unit are the same arithmetic over the same
 * per-agent rows; only the key changes. Written as three functions they drift,
 * and they drift silently, because each keeps returning a plausible total. So
 * [`groupRows`] takes a key function and everything else is shared.
 *
 * # THE TWO WINDOWS, AND WHY THEY ARE NOT MERGED
 *
 * Money columns come from `money_runs` (the Cloud, its own retention). Count
 * columns come from `stats_counts` (this console's bus store, which is fresh
 * per launch). They are different windows over different stores, and a single
 * "total" spanning both would be a number with no defined period. They stay in
 * two labelled column groups, and this module never sums across them.
 *
 * # THE UNATTRIBUTED BUCKET IS A ROW, NOT A GAP
 *
 * Grouping by owner needs idryx to know who owns an agent, and grouping by unit
 * needs the agent id to parse. Neither always holds. Those agents land in an
 * explicit row (`unattributed: true`) carrying their real numbers, rather than
 * being dropped: a leaderboard whose total quietly disagrees with the Money tab
 * is worse than one with an honest "no owner in idryx" line at the bottom.
 */

const NO_ENVIRONMENT_ERROR: StatsError = { kind: "no_environment" };

/** How many FULL event rows the backend may open per refresh.
 *
 * This no longer bounds the counts, and the distinction is the whole point. The
 * blocked / odd / budget columns come from a SQL aggregate that reads no rows
 * and is exact for the entire window however large it is. What is bounded here
 * is the second, narrow read: the events whose own `data` has to be opened to
 * say which detector fired, how far over budget a run went, and which agents an
 * operator's block halted.
 *
 * It used to bound everything, at 20,000, and that was a silent truncation on
 * any busy estate. @measured `genaryx/crates/api/tests/stats_scale.rs`,
 * 2026-08-11, on 42 agents x 100 events/day x 90 days: 378,000 events in the
 * window, of which the panel read 20,000, so a question about that window was
 * answered from about five per cent of it under a sentence reading "counted
 * from 20,000 event(s)".
 *
 * 100,000 comes from the same bench. That shape produces 52,920 events needing
 * their own row (14% of the bus), read in about 200 ms, so the cap clears a
 * heavy quarter with headroom rather than sitting just above today's estate. If
 * it IS hit, `panel.detail_truncated` says so and the affected columns are
 * marked; the counts stay exact either way. */
export const STATS_SCAN = 100_000;

function toStatsError(err: unknown): StatsError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as StatsError;
  }
  return { kind: "backend", message: err instanceof Error ? err.message : String(err) };
}

/** Per-agent counts from the bus. Throws [`StatsError`]; never returns a
 * panel-shaped nothing (the same guard `lib/egress.ts` documents: the mock
 * transport answers `null` for a command it does not know, and a view that
 * cannot tell that from an empty result hangs on "loading" for ever). */
export async function fetchStats(scan = STATS_SCAN, windowDays = 0): Promise<StatsPanel> {
  if (!hasBackend()) throw NO_ENVIRONMENT_ERROR;
  let raw: unknown;
  try {
    raw = await invokeBackend<unknown>("stats_counts", { scan, window_days: windowDays });
  } catch (err) {
    throw toStatsError(err);
  }
  if (!raw || typeof raw !== "object" || !("measured" in raw)) {
    throw {
      kind: "backend",
      message:
        "This build asked for the event counts and got an answer it could not read. " +
        "That is not a report that your agents were never stopped.",
    } as StatsError;
  }
  return raw as StatsPanel;
}

/** The four cuts, and two of them are about people on purpose.
 *
 * `owner` joins through idryx: who OWNS the agent, a registry fact about
 * accountability. `launcher` reads the money plane's `/v1/owners`: the root of
 * the delegation chain, so who STARTED the run, a runtime fact about what
 * actually happened. An agent owned by one person and run on another's behalf
 * appears under different names in the two.
 *
 * They stay separate rather than being reconciled into one "owner" column. A
 * single merged figure would answer neither question and would do it without
 * saying so, which is the shape of defect this console has already paid for
 * once (a unit reading 12 agents on one screen and 16 on another). */
export type GroupBy = "agent" | "owner" | "launcher" | "unit";

/** One line of the table. Money fields and count fields are both here, but they
 * come from the two different windows named in this module's doc, and the view
 * labels them as two column groups. */
export interface StatsRow {
  /** The group key: an agent id, an owner handle, or a unit id. */
  key: string;
  /** What to show. For an agent, its short name; for a unit, the pretty label;
   * for an owner, the handle as idryx spells it. */
  label: string;
  /** How many distinct agents this row covers (always 1 when grouping by
   * agent). */
  agentCount: number;

  // Money window (the Cloud).
  spentUsd: number;
  calls: number;
  runs: number;

  // Bus window (this console, since it started).
  blocked: number;
  /** Of `blocked`, the ones a person caused. `blocked - blockedByOperator` is
   * what the services stopped on their own. */
  blockedByOperator: number;
  anomalies: number;
  budgetEvents: number;
  /** Which idryx detectors fired across this row, by name, so "odd behaviour"
   * can say WHAT was odd. Summed across the row's agents: two agents each
   * tripping `impossible_travel` is two. */
  detectors: Record<string, number>;
  /** The worst single breach in micro-USD, or `null` when no event carried the
   * amounts. Never coerced to 0: see `statsTypes.ts`. A group's value is the
   * worst of its members, not their total - summing breaches across agents
   * would report an overspend the estate never had. */
  worstOvershootMicrousd: number | null;

  /** True for the single row collecting agents this grouping could not place.
   * Rendered as itself, never hidden. */
  unattributed: boolean;

  /** Whether the bus-derived count columns mean anything for this row.
   *
   * False for the `launcher` grouping: the counts are per AGENT, and the money
   * plane's per-person rollup is an aggregate with no agent list to join them
   * to. Rendering 0 there would say "this person's agents were never stopped",
   * which nothing measured. The cells show a dash instead. */
  countsApply: boolean;
}

/** The key an owner grouping puts agents under when idryx has no owner for
 * them. Not a handle, and deliberately not a blank: idryx's own `OrphanedNHI`
 * detector treats an unowned identity as a finding, so this row is a real
 * answer to "who owns this", not a rendering gap. */
export const NO_OWNER_KEY = "(no owner in idryx)";

/** The unit key for an agent id that does not parse to a team. */
export const NO_UNIT_KEY = "(no unit)";

interface Acc {
  key: string;
  label: string;
  agents: Set<string>;
  spentUsd: number;
  calls: number;
  runs: number;
  blocked: number;
  blockedByOperator: number;
  anomalies: number;
  budgetEvents: number;
  overshoot: number | null;
  detectors: Record<string, number>;
  unattributed: boolean;
}

/** Fold the money rows and the count rows into one table, keyed by `by`.
 *
 * `counts` may be empty (an unmeasured panel, or simply a quiet bus): the money
 * columns still resolve, and the count columns are honestly zero for the window
 * the view labels. The reverse also holds - an agent that only appears on the
 * bus and has no run in the money window gets a row with zero spend, because
 * "this agent was blocked 40 times and spent nothing" is exactly the row an
 * operator needs to see. */
export function groupRows(
  runs: Run[],
  identities: IdryxIdentity[],
  counts: AgentStats[],
  by: GroupBy,
): StatsRow[] {
  const owners = ownerByAgent(identities);
  const spend = spendByAgent(runs);
  const byAgentSpend = new Map(spend.map((s) => [s.agent, s]));
  const byAgentCount = new Map(counts.map((c) => [c.agent_id, c]));

  // Every agent seen in EITHER window, so neither store can hide a row.
  const allAgents = new Set<string>([...byAgentSpend.keys(), ...byAgentCount.keys()]);

  const keyFor = (agentId: string): { key: string; label: string; unattributed: boolean } => {
    if (by === "agent") {
      const short = byAgentSpend.get(agentId)?.name ?? shortAgent(agentId);
      return { key: agentId, label: short, unattributed: false };
    }
    if (by === "owner") {
      const owner = owners.get(agentId);
      if (!owner) return { key: NO_OWNER_KEY, label: NO_OWNER_KEY, unattributed: true };
      return { key: owner, label: owner, unattributed: false };
    }
    const team = byAgentSpend.get(agentId)?.team || agentTeam(agentId);
    if (!team) return { key: NO_UNIT_KEY, label: NO_UNIT_KEY, unattributed: true };
    const unit = unitForTeam(team);
    return { key: unit, label: unit, unattributed: false };
  };

  const acc = new Map<string, Acc>();
  for (const agentId of allAgents) {
    const { key, label, unattributed } = keyFor(agentId);
    const row = acc.get(key) ?? {
      key,
      label,
      agents: new Set<string>(),
      spentUsd: 0,
      calls: 0,
      runs: 0,
      blocked: 0,
      blockedByOperator: 0,
      anomalies: 0,
      budgetEvents: 0,
      overshoot: null,
      detectors: {},
      unattributed,
    };
    row.agents.add(agentId);

    const s = byAgentSpend.get(agentId);
    if (s) {
      row.spentUsd += s.spent;
      row.calls += s.calls;
      row.runs += s.runs;
    }
    const c = byAgentCount.get(agentId);
    if (c) {
      row.blocked += c.blocked;
      row.blockedByOperator += c.blocked_by_operator;
      for (const [name, n] of Object.entries(c.by_detector ?? {})) {
        row.detectors[name] = (row.detectors[name] ?? 0) + n;
      }
      row.anomalies += c.anomalies;
      row.budgetEvents += c.budget_events;
      // The worst of the group, not the sum. A null stays null rather than
      // becoming a 0: "nobody wrote it down" is not "did not go over".
      if (c.worst_overshoot_microusd !== null) {
        row.overshoot = Math.max(row.overshoot ?? 0, c.worst_overshoot_microusd);
      }
    }
    acc.set(key, row);
  }

  return [...acc.values()].map((r) => ({
    key: r.key,
    label: r.label,
    agentCount: r.agents.size,
    spentUsd: r.spentUsd,
    calls: r.calls,
    runs: r.runs,
    blocked: r.blocked,
    blockedByOperator: r.blockedByOperator,
    anomalies: r.anomalies,
    budgetEvents: r.budgetEvents,
    worstOvershootMicrousd: r.overshoot,
    detectors: r.detectors,
    unattributed: r.unattributed,
    countsApply: true,
  }));
}

/** Rows straight from the money plane's own per-person rollup.
 *
 * No fold here at all: the plane already aggregated, and re-deriving it
 * console-side from runs would produce a second number for the same question.
 *
 * The count columns are marked inapplicable rather than zeroed. `/v1/owners`
 * returns totals per person with no agent list, so there is nothing to join the
 * per-agent bus counts to, and a 0 in the blocked column would be a claim
 * nobody measured. */
export function rowsFromOwners(owners: Owner[]): StatsRow[] {
  return owners.map((o) => ({
    key: o.owner,
    // "unassigned" is the plane's own literal for a run whose chain named
    // nobody. Rendered as the sentence it is, and pinned last like every other
    // unattributed row.
    label: o.owner === "unassigned" ? NO_CHAIN_KEY : o.owner,
    agentCount: o.agents,
    spentUsd: o.spent_usd,
    calls: o.calls,
    runs: o.runs,
    blocked: 0,
    blockedByOperator: 0,
    anomalies: 0,
    budgetEvents: 0,
    worstOvershootMicrousd: null,
    detectors: {},
    unattributed: o.owner === "unassigned",
    countsApply: false,
  }));
}

/** What the money plane calls a run whose delegation chain named no human,
 * spelled for a reader rather than as the wire literal `"unassigned"`. */
export const NO_CHAIN_KEY = "(no delegation chain)";

/** Last path segment of an agent id, for a row whose agent never appeared in
 * the money window (so `spendByAgent` never named it). */
function shortAgent(agentId: string): string {
  return agentId.split("/").filter(Boolean).pop() ?? agentId;
}

export type SortKey =
  | "label"
  | "spentUsd"
  | "calls"
  | "blocked"
  | "blockedByOperator"
  | "anomalies"
  | "budgetEvents"
  | "worstOvershootMicrousd"
  | "agentCount";

/** Sort a copy of `rows`, keeping the unattributed row pinned last whichever
 * way the table is sorted.
 *
 * Pinned because it is not a competitor in the ranking: "the agents nobody
 * owns" topping a spend leaderboard reads as a person called "(no owner in
 * idryx)" being the biggest spender in the company. A `null` overshoot sorts as
 * absent (always after any number), never as zero. */
export function sortRows(rows: StatsRow[], key: SortKey, desc: boolean): StatsRow[] {
  const out = [...rows];
  out.sort((a, b) => {
    if (a.unattributed !== b.unattributed) return a.unattributed ? 1 : -1;
    if (key === "label") {
      const cmp = a.label.localeCompare(b.label);
      return desc ? -cmp : cmp;
    }
    const av = a[key];
    const bv = b[key];
    if (av === null && bv === null) return 0;
    if (av === null) return 1;
    if (bv === null) return -1;
    return desc ? bv - av : av - bv;
  });
  return out;
}
