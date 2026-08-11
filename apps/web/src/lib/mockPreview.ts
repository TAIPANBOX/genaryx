/**
 * A no-backend preview of the console.
 *
 * On only when the build sets `VITE_GENARYX_MOCK` (`.env.mock`, `pnpm
 * dev:mock`). In that mode `transport.ts` routes every command here and every
 * bus subscription to the synthetic stream below, and because no
 * `VITE_GENARYX_API` is set the login gate never renders: open a localhost URL
 * and see a populated, MOVING dashboard, no passwords.
 *
 * Everything here is invented and says so (`bus_status` returns a `demo` mode,
 * so the Bus Explorer shows its own "this is generated" banner). It is a design
 * surface and a showroom, never a screenshot to pass off as real traffic.
 *
 * WHAT IT MODELS
 *
 * One engineering org, `meridian.io`, governing its AI agent fleet. Four
 * business units (SRE, Platform, FinOps, Data Platform), each with several
 * engineers, each engineer owning a few agents that do concrete work for their
 * unit. One agent, SRE's `rca-copilot`, is the caught runaway. Every wire DTO
 * is derived from that one generated fleet, so the numbers agree across tabs.
 *
 * HOW IT EVOLVES
 *
 * The fleet above is the frozen seed; the "Live world model" section below
 * makes it move, in two ways gated by `src/demo/scenario.ts`'s calm/incident
 * toggle: a gentle, unconditional background drift on every agent's totals
 * (calm's "still alive" feel), and a directed ~30s spend-runaway arc on the
 * protagonist (`rca-copilot`) that only plays in "incident" - climbing past
 * its budget, tripping bus events and an incident + approval, then resolving
 * by operator action or on its own and looping. Both are computed lazily from
 * elapsed wall-clock time, not a background timer, so every read command
 * (`money_*`, `policy_*`, the live bus, ...) agrees on "where the story is
 * right now" no matter which tab is open or how long it has been.
 */

import type { UiEvent } from "../types";
import { getScenario, onScenarioChange, type DemoScenario } from "../demo/scenario";
import type { EntityLifecycleState } from "./lifecycleTypes";
import type { EgressRow } from "../egressTypes";

export const MOCK = import.meta.env.VITE_GENARYX_MOCK === "1";

const ORG = "meridian.io";
const DAY = 86_400_000;
const now = Date.now();
const ago = (ms: number) => new Date(now - ms).toISOString();

/** How long this estate has been running, which every dated fixture below is
 * measured against.
 *
 * The fleet used to be 8 to 20 days old. Every history in the console was
 * therefore a fortnight long: ownership never changed hands twice, a quality
 * baseline from "end of month" was three weeks old, and the Statistics windows
 * had nothing to distinguish. It read as a product someone installed last
 * Tuesday, which is the opposite of what this console is for.
 *
 * Now: two months at the youngest, fourteen at the oldest. That spread matters
 * more than the absolute number. A fleet where every agent is the same age has
 * no transfers worth showing, no agents that predate a policy, and nothing for
 * a year window to select over. */
const AGENT_AGE_FLOOR_MS = 60 * DAY;
const AGENT_AGE_SPREAD_MS = 370 * DAY;

// ---------------------------------------------------------------------------
// Fleet shape.
// ---------------------------------------------------------------------------

export interface LifecycleEntry {
  ts: string;
  kind: "launched" | "owned" | "transferred" | "budget_set" | "closed";
  detail: string;
  actor: string;
}

/** One stretch of an agent's life under a given owner and unit, with what it
 * spent during it. Transferring the agent freezes the current segment and
 * opens a new one, so an owner or unit is only ever charged for the spend that
 * happened while the agent was theirs, and the agent's own total is the sum of
 * all segments. */
export interface AttributionSegment {
  owner: string;
  team: string;
  spentUsd: number;
  from: string;
  to: string | null;
}

export interface FleetAgent {
  /** The emitted agent id, fixed for the life of the agent. Governance
   * attributes (unit, owner) can change without it moving, the same way a real
   * agent keeps emitting the same id even when its ownership is reassigned. */
  id: string;
  team: string;
  name: string;
  model: string;
  owner: string;
  budgetUsd: number;
  allowed: string[];
  spentUsd: number;
  calls: number;
  /** Spend split by ownership period; the last segment (to === null) is the
   * current one and its owner/team match this agent's owner/team. */
  segments: AttributionSegment[];
  /** Operator-disabled (blocked), distinct from `closed` (the runaway killed
   * for cause). DERIVED on read from the manual lifecycle store below (frozen
   * agents / stopped units / stopped users / killed runs), never mutated on the
   * fixture itself, so one agent can be blocked via several paths at once and
   * un-blocking one leaves the others in force. */
  blocked?: boolean;
  /** Effective operator-lifecycle state, set on every read of this record from
   * the manual store. See {@link agentLifecycleState}. */
  lifecycle?: EntityLifecycleState;
  history: LifecycleEntry[];
  closed?: { by: string; reason: string; wrongdoing: string; ts: string };
}

// ---------------------------------------------------------------------------
// The org: units, their engineers, and the agent roles each unit runs. The
// fleet is generated from this so it is a realistic size (dozens of agents,
// twenty engineers) without hand-listing every row.
// ---------------------------------------------------------------------------

interface Kind {
  suffix: string;
  model: string;
  allowed: string[];
}

interface UnitDef {
  team: string;
  label: string;
  users: string[];
  kinds: Kind[];
}

const UNITS: UnitDef[] = [
  {
    team: "sre",
    label: "SRE",
    users: ["j.carter", "m.bennett", "s.dawson", "r.walsh", "p.novak"],
    kinds: [
      { suffix: "incident-triage-copilot", model: "claude-sonnet-4-5", allowed: ["pagerduty_read", "logs_read", "summarize only", "no writes"] },
      { suffix: "alert-correlator", model: "claude-haiku-4-5", allowed: ["metrics_read", "dedupe alerts"] },
      { suffix: "runbook-executor", model: "claude-sonnet-4-5", allowed: ["run approved runbooks", "human approval above sev2", "no prod delete"] },
      { suffix: "log-analyzer", model: "gpt-4o-mini", allowed: ["logs_read", "grep + summarize"] },
      { suffix: "postmortem-writer", model: "claude-sonnet-4-5", allowed: ["incident_read", "draft docs, no publish"] },
      { suffix: "oncall-summarizer", model: "claude-haiku-4-5", allowed: ["shift_read", "summarize handoff"] },
      { suffix: "slo-watchdog", model: "gpt-4o-mini", allowed: ["slo_read", "alert on burn"] },
      { suffix: "capacity-forecaster", model: "gpt-4o", allowed: ["metrics_read", "forecast only"] },
      { suffix: "rca-copilot", model: "claude-opus-4-5", allowed: ["traces_read", "correlate", "max 12 steps"] },
      { suffix: "deploy-guard", model: "claude-sonnet-4-5", allowed: ["deploy_read", "block risky deploys", "human approve override"] },
      { suffix: "toil-automator", model: "claude-haiku-4-5", allowed: ["ticket_read", "propose automation"] },
      { suffix: "synthetic-monitor", model: "gpt-4o-mini", allowed: ["probe endpoints", "no writes"] },
    ],
  },
  {
    team: "platform",
    label: "Platform",
    users: ["a.klein", "t.osei", "l.moreau", "d.singh", "e.rossi", "k.abbott"],
    kinds: [
      { suffix: "ci-optimizer", model: "claude-haiku-4-5", allowed: ["pipeline_read", "suggest caching"] },
      { suffix: "iac-reviewer", model: "claude-sonnet-4-5", allowed: ["terraform_read", "comment only, no apply"] },
      { suffix: "dependency-upgrader", model: "gpt-4o-mini", allowed: ["deps_read", "open PRs", "human merge"] },
      { suffix: "k8s-config-linter", model: "claude-haiku-4-5", allowed: ["manifests_read", "lint"] },
      { suffix: "service-scaffolder", model: "claude-sonnet-4-5", allowed: ["template_read", "scaffold in sandbox"] },
      { suffix: "api-gateway-tuner", model: "gpt-4o", allowed: ["gateway_read", "propose limits", "human apply"] },
      { suffix: "build-cost-analyzer", model: "gpt-4o-mini", allowed: ["ci_metrics_read"] },
      { suffix: "flaky-test-hunter", model: "claude-haiku-4-5", allowed: ["test_history_read", "quarantine propose"] },
      { suffix: "terraform-drift-detector", model: "claude-sonnet-4-5", allowed: ["state_read", "report drift"] },
      { suffix: "image-slimmer", model: "gpt-4o-mini", allowed: ["dockerfile_read", "suggest layers"] },
      { suffix: "secret-scanner", model: "claude-sonnet-4-5", allowed: ["repo_read", "dlp on", "no external send"] },
      { suffix: "release-notes-writer", model: "claude-haiku-4-5", allowed: ["pr_read", "draft notes"] },
    ],
  },
  {
    team: "finops",
    label: "FinOps",
    users: ["n.foster", "c.ibarra", "w.zhang", "b.oconnor", "h.mueller"],
    kinds: [
      { suffix: "cost-anomaly-detector", model: "claude-sonnet-4-5", allowed: ["billing_read", "alert on spike"] },
      { suffix: "rightsizing-advisor", model: "gpt-4o", allowed: ["usage_read", "recommend only"] },
      { suffix: "budget-forecaster", model: "gpt-4o", allowed: ["spend_read", "forecast"] },
      { suffix: "commitment-planner", model: "claude-sonnet-4-5", allowed: ["usage_read", "propose commitments", "human approve"] },
      { suffix: "chargeback-reporter", model: "claude-haiku-4-5", allowed: ["tags_read", "generate reports"] },
      { suffix: "invoice-reconciler", model: "gpt-4o-mini", allowed: ["invoice_read", "match line items"] },
      { suffix: "idle-resource-sweeper", model: "claude-haiku-4-5", allowed: ["inventory_read", "propose stop", "human approve"] },
      { suffix: "unit-economics-analyst", model: "claude-opus-4-5", allowed: ["metrics_read", "model unit cost"] },
      { suffix: "savings-plan-optimizer", model: "gpt-4o", allowed: ["usage_read", "recommend plans"] },
      { suffix: "tag-compliance-auditor", model: "gpt-4o-mini", allowed: ["tags_read", "flag untagged"] },
    ],
  },
  {
    team: "data",
    label: "Data Platform",
    users: ["g.harper", "s.malik", "o.lindqvist", "f.romano"],
    kinds: [
      { suffix: "pipeline-monitor", model: "claude-haiku-4-5", allowed: ["airflow_read", "alert on failure"] },
      { suffix: "schema-drift-detector", model: "claude-sonnet-4-5", allowed: ["catalog_read", "report drift"] },
      { suffix: "query-cost-optimizer", model: "gpt-4o", allowed: ["warehouse_read", "suggest rewrites"] },
      { suffix: "data-quality-checker", model: "claude-haiku-4-5", allowed: ["table_read", "run checks"] },
      { suffix: "dbt-model-reviewer", model: "claude-sonnet-4-5", allowed: ["dbt_read", "comment on PRs"] },
      { suffix: "backfill-planner", model: "gpt-4o-mini", allowed: ["job_read", "plan backfills"] },
      { suffix: "pii-scanner", model: "claude-sonnet-4-5", allowed: ["column_read", "dlp on", "attestation required"] },
      { suffix: "lineage-mapper", model: "gpt-4o-mini", allowed: ["lineage_read", "map only"] },
    ],
  },
];

/** Deterministic 0..1 from a string, so spend/calls are stable across reloads
 * (screenshots must not jitter). */
function pseudo(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return (h >>> 0) / 4294967296;
}

const PER_CALL_USD: Record<string, number> = {
  "claude-haiku-4-5": 0.0032,
  "claude-sonnet-4-5": 0.011,
  "claude-opus-4-5": 0.024,
  "gpt-4o": 0.0085,
  "gpt-4o-mini": 0.0007,
};

const RUNAWAY_TEAM = "sre";
const RUNAWAY_NAME = "rca-copilot";

/** I11 "agent drift card" fixture: the one mock agent that gets a
 * quality-drift history, a short spend-run history, and a behavior-anomaly
 * alert, so Agent 360's Drift section (`components/Agent360.tsx`) has
 * something real to render for at least one agent under `dev:mock`. An
 * ordinary, unprivileged data-team agent - deliberately NOT the runaway
 * (already its own distinct story above) and not one of the I5 access-matrix
 * fixture agents below, so each fixture agent's "reason to look
 * interesting" stays legible on its own. */
const DRIFT_DEMO_AGENT_ID = `agent://${ORG}/data/data-quality-checker`;

function genHistory(a: { team: string; name: string; owner: string }): LifecycleEntry[] {
  const born = AGENT_AGE_FLOOR_MS + Math.floor(pseudo(a.name + "born") * AGENT_AGE_SPREAD_MS);
  const h: LifecycleEntry[] = [
    { ts: ago(born), kind: "launched", detail: `launched for ${a.team} work`, actor: a.owner },
    { ts: ago(born), kind: "owned", detail: `owned by ${a.team} / ${a.owner}`, actor: "system" },
  ];
  if (pseudo(a.name + "xfer") > 0.72) {
    h.push({ ts: ago(born / 2), kind: "transferred", detail: `reassigned within ${a.team}`, actor: a.owner });
  }
  return h;
}

/** Which of a unit's people owns its i-th agent.
 *
 * This was `u.users[i % u.users.length]`, a strict round-robin, and the result
 * was that every person in the company ran exactly the same number of agents.
 * On the Owner grouping that produced a column of identical 2s, which reads as
 * a placeholder rather than as an estate (Yurii, 2026-08-10: "щоб не було всіх
 * по два").
 *
 * Real teams are not flat. Someone owns the four agents that matter and
 * somebody else owns one they inherited, because the work is not the same size.
 *
 * Two passes, so the spread never costs anyone their place in the list:
 *
 *  1. The first `users.length` agents go round-robin, so every person owns at
 *     least one and nobody vanishes from the Owner grouping.
 *  2. Everything after that is picked by a deterministic WEIGHT per person, so
 *     the surplus piles up unevenly. Weights come from `pseudo`, so the fleet
 *     is identical on every reload and a screenshot never shifts under you.
 */
function ownerFor(u: { team: string; users: string[] }, i: number): string {
  if (i < u.users.length) return u.users[i];

  // DEALT from a weighted bag, not drawn from one.
  //
  // The first cut drew independently against the weights on every surplus
  // agent, and the heaviest person in each unit took almost all of it: sixteen
  // people ended up with exactly one agent and four with five to eight. That is
  // a different wrong shape from the flat 2s it replaced, not a fix.
  //
  // A bag holding each person `weight` times, dealt in order, spreads the
  // surplus in PROPORTION to the weights instead of concentrating it. Someone
  // ends up with four and someone with one, which is the ask.
  const bag: string[] = [];
  for (const h of u.users) {
    const weight = 1 + Math.floor(pseudo(`${u.team}:${h}:load`) * 3);
    for (let n = 0; n < weight; n++) bag.push(h);
  }
  return bag[(i - u.users.length) % bag.length];
}

function buildFleet(): FleetAgent[] {
  const out: FleetAgent[] = [];
  for (const u of UNITS) {
    u.kinds.forEach((k, i) => {
      const owner = ownerFor(u, i);
      const isRunaway = u.team === RUNAWAY_TEAM && k.suffix === RUNAWAY_NAME;
      const calls = isRunaway ? 1240 : 150 + Math.floor(pseudo(k.suffix + "c") * 2300);
      const per = PER_CALL_USD[k.model] ?? 0.01;
      const spentUsd = isRunaway
        ? 41.6
        : Number((calls * per * (0.55 + pseudo(k.suffix + "s") * 1.05)).toFixed(2));
      const budgetUsd = isRunaway ? 1.25 : Number((0.5 + pseudo(k.suffix + "b") * 3).toFixed(2));

      // Attribution: a few agents already carry a prior owner (or a prior unit),
      // so the spend split by ownership period is visible out of the box.
      const bornMs = AGENT_AGE_FLOOR_MS + Math.floor(pseudo(k.suffix + "born") * AGENT_AGE_SPREAD_MS);
      const launchTs = ago(bornMs);
      const history = genHistory({ team: u.team, name: k.suffix, owner });
      let segments: AttributionSegment[];
      if (!isRunaway && pseudo(k.suffix + "seg") > 0.74) {
        const midTs = ago(Math.floor(bornMs / 2));
        const frac = 0.35 + pseudo(k.suffix + "frac") * 0.35;
        const past = Number((spentUsd * frac).toFixed(2));
        const cur = Number((spentUsd - past).toFixed(2));
        const unitMove = pseudo(k.suffix + "segtype") > 0.68;
        if (unitMove) {
          const pastTeam = UNITS.find((x) => x.team !== u.team)?.team ?? u.team;
          segments = [
            { owner, team: pastTeam, spentUsd: past, from: launchTs, to: midTs },
            { owner, team: u.team, spentUsd: cur, from: midTs, to: null },
          ];
          history.push({ ts: midTs, kind: "transferred", detail: `business unit ${pastTeam} -> ${u.team}`, actor: "console-op" });
        } else {
          const pastOwner = u.users.find((x) => x !== owner) ?? owner;
          segments = [
            { owner: pastOwner, team: u.team, spentUsd: past, from: launchTs, to: midTs },
            { owner, team: u.team, spentUsd: cur, from: midTs, to: null },
          ];
          history.push({ ts: midTs, kind: "transferred", detail: `owner ${pastOwner} -> ${owner}`, actor: "console-op" });
        }
      } else {
        segments = [{ owner, team: u.team, spentUsd, from: launchTs, to: null }];
      }

      const agent: FleetAgent = {
        id: `agent://${ORG}/${u.team}/${k.suffix}`,
        team: u.team,
        name: k.suffix,
        model: k.model,
        owner,
        budgetUsd,
        allowed: k.allowed,
        spentUsd,
        calls,
        segments,
        history,
      };
      if (isRunaway) {
        const ts = ago(18 * 60_000);
        agent.history.push({ ts: ago(4 * DAY), kind: "budget_set", detail: "per-run ceiling set to $1.25", actor: owner });
        agent.history.push({ ts, kind: "closed", detail: "killed after runaway retries on an oversized incident context", actor: "sre-oncall" });
        agent.closed = {
          by: "sre-oncall",
          reason: "break-glass: runaway retries burning the per-run ceiling",
          wrongdoing: "looped on a huge incident trace, retried past its budget 26 times across shards, tripping budget_exhausted and fanout_explosion",
          ts,
        };
      }
      out.push(agent);
    });
  }
  return out;
}

export const FLEET: FleetAgent[] = buildFleet();

export const agentId = (a: FleetAgent) => a.id;
export const userId = (u: string) => `user://${ORG}/${u}`;

// ---------------------------------------------------------------------------
// Live world model.
//
// Two ingredients, deliberately kept separate:
//
// 1. A gentle, unconditional background drift (`liveDrift`) applied to every
//    agent's historical totals (spend/calls), a pure function of how long the
//    page has been open. This is what makes "calm" feel alive: small ticks,
//    nothing ever crosses a budget.
//
// 2. A directed incident arc for ONE protagonist (SRE's `rca-copilot`, the
//    fleet's own documented runaway - see `FleetAgent.closed` above), active
//    only in the "incident" scenario: its LIVE run's spend/budget fraction
//    climbs 0.6 -> 1.0 -> 1.28 over ~30s, trips `budget_exceeded`/
//    `breaker_tripped` bus events and an incident + approval once it is
//    airborne, then resolves - by the operator killing the run or denying the
//    approval, or on its own once the arc's time is up - sits in a
//    resolved/idle beat, and loops into a fresh climb.
//
// Computed LAZILY from elapsed wall-clock time (`reconcileProtagonist`)
// rather than mutated by a background timer, so the story advances
// identically no matter how many tabs/panels are reading it, and a tab left
// unread for a while still lands on an honest "where would this be right
// now" state instead of replaying every missed beat.
//
// Operator actions (`money_kill_run`, `money_set_budget`, `money_ack_incident`,
// `policy_decide_approval` - all four unhandled before this, falling through
// to `mockInvoke`'s `default: return r(null)`) are layered on top as small,
// idempotent, keyed overrides (`manualRunKills`, `manualRunBudgets`,
// `ackedIncidentIds`, `decidedApprovals`) that persist across a scenario flip
// - an operator's own past decision does not un-happen just because the demo
// storyline switched - and, for the protagonist specifically, feed straight
// into `reconcileProtagonist`'s own resolution.
// ---------------------------------------------------------------------------

const PROTAGONIST: FleetAgent = FLEET.find((a) => a.team === RUNAWAY_TEAM && a.name === RUNAWAY_NAME) ?? FLEET[0];
const PROTAGONIST_ID = agentId(PROTAGONIST);
const PROTAGONIST_RUN_ID = `${PROTAGONIST.name}-live`;
/** The live run's own per-run ceiling - distinct from `PROTAGONIST.budgetUsd`
 * (the frozen $1.25 fixture from the DIFFERENT, already-closed incident on
 * `FleetAgent.closed`), so this arc reads as a fresh run, not a replay. */
const PROTAGONIST_RUN_BUDGET_USD = 18;

const CLIMB_START_FRACTION = 0.6;
const CLIMB_MS = 16_000; // fraction 0.6 -> 1.0
const OVER_MS = 14_000; // fraction 1.0 -> 1.28
const TRIP_MS = CLIMB_MS + OVER_MS; // 30s airborne, inside the brief's 20-40s
const RESOLVED_BEAT_MS = 15_000; // idle-green tail before the arc loops

let currentScenario: DemoScenario = getScenario();
let armedAt = now; // when the CURRENT arc started climbing
let manualKillAt: number | null = null; // set once this arc has been resolved
let killFraction = 0; // fraction frozen at the moment it resolved
let killedByOperator = false;

interface ArcApproval {
  id: string;
  requestedAt: number;
  decided: boolean;
  decision: "grant" | "deny" | null;
  decidedAt: number | null;
}
let arcApproval: ArcApproval | null = null;

// Cross-scenario, cross-arc operator-action overrides - never cleared by a
// scenario flip or an arc loop, only by time (the generic kill recovery) or
// by staleness (a stale kill timestamp from a past arc simply never matches
// `opKillAt >= armedAt` again once a fresh arc has armed).
const manualRunKills = new Map<string, number>();
const manualRunBudgets = new Map<string, number>();
const ackedIncidentIds = new Set<string>();
const decidedApprovals = new Map<string, { decision: "grant" | "deny"; decidedAt: number }>();

// ---------------------------------------------------------------------------
// Manual lifecycle store (Yurii, 2026-07-24): the ONE source of truth for the
// console's operator-driven lifecycle actions, so a Stop/Freeze/Kill reads
// app-wide (Overview spend-by-agent, Money runs, the Graph, the Agent/Unit/User
// cards, the watch dock), not only where it was clicked. Every read DTO below
// reflects it; the four mutation commands (`agent_block`, `unit_block`,
// `user_block`, `money_kill_run`) write it.
//
//  - `frozenAgents`  : agent ids an operator froze (Freeze <-> Unfreeze).
//  - `stoppedUnits`  : teams an operator stopped, so EVERY agent in the unit
//                      reads STOPPED and stops accruing (Stop <-> Start).
//  - `stoppedUsers`  : owner handles an operator stopped, so EVERY agent that
//                      user owns reads STOPPED and stops accruing.
//  - `killedRuns`    : run ids an operator killed. STICKY on purpose: unlike
//                      the old 20s generic self-heal (which made an operator
//                      kill look like it did nothing), an operator kill persists
//                      visibly until the page reloads. Kill is a one-way action,
//                      not a toggle, so nothing removes an id from this set.
//
// All four survive a scenario flip (an operator's decision does not un-happen
// because the storyline changed) and are idempotent (Set add/delete).
const frozenAgents = new Set<string>();
const stoppedUnits = new Set<string>();
const stoppedUsers = new Set<string>();
const killedRuns = new Set<string>();

/** When an agent was first observed blocked, so its live drift freezes at that
 * instant ("stop accruing") and resumes from real time once it is started
 * again. Managed lazily by {@link effectiveActivity}, so every block path
 * (freeze / unit stop / user stop / kill) freezes without each mutation
 * stamping a time of its own. */
const blockedSince = new Map<string, number>();

const liveRunIdFor = (a: FleetAgent): string => `${a.name}-live`;

/** An agent's blocked state from the manual store ALONE - ignores `a.closed`
 * (a fixture fact, not an operator action) and the protagonist incident arc,
 * so freezing/stopping the protagonist overlays cleanly on top of its arc.
 * `null` when the store holds nothing for it. Precedence killed > frozen >
 * stopped. */
function manualAgentState(a: FleetAgent): EntityLifecycleState | null {
  if (killedRuns.has(liveRunIdFor(a))) return "killed";
  if (frozenAgents.has(a.id)) return "frozen";
  if (stoppedUnits.has(a.team) || stoppedUsers.has(a.owner)) return "stopped";
  return null;
}

/** An agent's full effective lifecycle state. `ignoreClosed` is for the "calm"
 * scenario, where the protagonist's PAST closed incident is not held against
 * its current live run (mirrors `baseRunFor`'s own `ignoreClosed`). */
function agentLifecycleState(a: FleetAgent, ignoreClosed = false): EntityLifecycleState {
  const manual = manualAgentState(a);
  if (manual) return manual;
  if (!ignoreClosed && a.closed) return "killed";
  return "live";
}

function isAgentBlocked(a: FleetAgent, ignoreClosed = false): boolean {
  return agentLifecycleState(a, ignoreClosed) !== "live";
}

// Savings ratchet: only a genuine incident resolution (see
// `registerIncidentResolution`) advances these, so "calm" stays close to the
// original static seed instead of drifting off it.
let budgetBreaksFromIncidents = 0;
let blockedSpendFromIncidents = 0;

function armProtagonistArc(scenario: DemoScenario): void {
  currentScenario = scenario;
  armedAt = Date.now();
  manualKillAt = null;
  killFraction = 0;
  killedByOperator = false;
  arcApproval = null;
}
onScenarioChange(() => armProtagonistArc(getScenario()));

/** The protagonist's spend/budget fraction at `elapsedMs` into an arc, had it
 * not been killed: a straight climb to 1.0 by `CLIMB_MS`, then on to 1.28 by
 * `TRIP_MS`, then pinned - `reconcileProtagonist` never actually evaluates
 * this past `TRIP_MS` (it resolves the arc right at that boundary); the pin
 * just keeps this a total function. */
function fractionAtElapsed(elapsedMs: number): number {
  if (elapsedMs <= 0) return CLIMB_START_FRACTION;
  if (elapsedMs < CLIMB_MS) return CLIMB_START_FRACTION + (elapsedMs / CLIMB_MS) * (1 - CLIMB_START_FRACTION);
  if (elapsedMs < TRIP_MS) return 1 + ((elapsedMs - CLIMB_MS) / OVER_MS) * 0.28;
  return 1.28;
}

/** One arc's worth of "the breaker did its job": bumps the savings ratchet
 * once (every caller only reaches this from a state transition, never a
 * plain re-read, so a resolution is never credited twice) and settles a
 * still-pending incident approval as a side effect of the run itself
 * ending. */
function registerIncidentResolution(fractionAtKill: number): void {
  budgetBreaksFromIncidents += 1;
  const overspend = Math.max(0, fractionAtKill - 1) * PROTAGONIST_RUN_BUDGET_USD;
  blockedSpendFromIncidents = Number((blockedSpendFromIncidents + Math.max(2.5, overspend)).toFixed(2));
  if (arcApproval && !arcApproval.decided) {
    arcApproval = { ...arcApproval, decided: true, decision: arcApproval.decision ?? "deny", decidedAt: Date.now() };
  }
}

interface ProtagonistState {
  phase: "climbing" | "tripped" | "resolved";
  fraction: number;
  killed: boolean;
  killedByOperator: boolean;
  /** How far into the arc the current (possibly frozen, if resolved) state
   * is - used to place `first_seen`/`last_seen`-style timestamps honestly
   * relative to when the arc actually started. */
  arcElapsedMs: number;
}

/**
 * Advances the protagonist's live incident arc to "where it should be right
 * now" and returns that state. Called by every reader (a run/incident/alert
 * answer, a bus tick) - never by a background timer - so the story advances
 * in lock-step with wall-clock time no matter how many tabs/polls are reading
 * it. Only meaningful while `currentScenario === "incident"`; every caller
 * already guards that before calling in.
 */
function reconcileProtagonist(): ProtagonistState {
  const opKillAt = manualRunKills.get(PROTAGONIST_RUN_ID) ?? null;
  if (opKillAt !== null && manualKillAt === null && opKillAt >= armedAt) {
    manualKillAt = opKillAt;
    killFraction = fractionAtElapsed(opKillAt - armedAt);
    killedByOperator = true;
    registerIncidentResolution(killFraction);
  }

  // Fast-forward past any number of fully-elapsed cycles (a tab that sat
  // unread for a while) instead of replaying each one: credit an
  // auto-resolve for any cycle that was never manually killed, then move on.
  for (let guard = 0; guard < 10_000; guard++) {
    const resolvedAt = manualKillAt ?? armedAt + TRIP_MS;
    const cycleEnd = resolvedAt + RESOLVED_BEAT_MS;
    if (Date.now() < cycleEnd) break;
    if (manualKillAt === null) registerIncidentResolution(1.28);
    armedAt = cycleEnd;
    manualKillAt = null;
    killFraction = 0;
    killedByOperator = false;
    arcApproval = null;
  }

  if (manualKillAt !== null) {
    return { phase: "resolved", fraction: killFraction, killed: true, killedByOperator, arcElapsedMs: manualKillAt - armedAt };
  }

  const elapsed = Date.now() - armedAt;
  if (elapsed >= TRIP_MS) {
    manualKillAt = armedAt + TRIP_MS;
    killFraction = 1.28;
    killedByOperator = false;
    registerIncidentResolution(killFraction);
    return { phase: "resolved", fraction: killFraction, killed: true, killedByOperator: false, arcElapsedMs: TRIP_MS };
  }

  const fraction = fractionAtElapsed(elapsed);
  if (!arcApproval && fraction >= 0.8) {
    arcApproval = { id: `ap_incident_${armedAt.toString(36)}`, requestedAt: Date.now(), decided: false, decision: null, decidedAt: null };
  }
  return { phase: elapsed < CLIMB_MS ? "climbing" : "tripped", fraction, killed: false, killedByOperator: false, arcElapsedMs: elapsed };
}

/** Gentle, unconditional per-agent drift: a small monotonic function of how
 * long the page has been open (capped at 4 hours of "credit" so a tab left
 * open for days does not produce an absurd number), layered on top of the
 * frozen fixture's own spend/calls wherever a read wants "alive" numbers.
 * Deterministic per agent (same `pseudo` convention the fixture generator
 * above already uses), so it is stable within one render rather than
 * re-randomized on every call. */
function liveDriftAsOf(a: FleetAgent, asOfMs: number): { spentUsd: number; calls: number } {
  const elapsedMin = Math.min(240, Math.max(0, asOfMs - now) / 60_000);
  const usdPerMin = 0.04 + pseudo(a.name + "driftusd") * 0.18;
  const callsPerMin = 0.8 + pseudo(a.name + "driftcalls") * 2.6;
  return { spentUsd: Number((usdPerMin * elapsedMin).toFixed(4)), calls: Math.floor(callsPerMin * elapsedMin) };
}

function liveDrift(a: FleetAgent): { spentUsd: number; calls: number } {
  return liveDriftAsOf(a, Date.now());
}

/** The activity (drift over the fixture baseline) a read should show for an
 * agent, FROZEN at the instant it was blocked so a stopped/frozen/killed agent
 * genuinely stops accruing - its spend/calls hold where they were - and resumes
 * from real time once it is started/unfrozen again. Freezing keys off the
 * MANUAL store only ({@link manualAgentState}), not `a.closed`: the
 * protagonist's fixture closure is not an operator "stop accruing", and its own
 * live arc (`runFor`) drives its numbers, not this. */
function effectiveActivity(a: FleetAgent): { spentUsd: number; calls: number } {
  if (manualAgentState(a) === null) {
    blockedSince.delete(a.id);
    return liveDrift(a);
  }
  let since = blockedSince.get(a.id);
  if (since === undefined) {
    since = Date.now();
    blockedSince.set(a.id, since);
  }
  return liveDriftAsOf(a, since);
}

// ---- Operator-mutation bus feedback ----------------------------------------
// A privileged console mutation (kill / budget / ack / decide) is not just a
// local state change: the real backend journals it AND appends a conforming
// `console_command` bus event (`crates/core/src/command.rs`), which is
// exactly what `MoneyView.tsx`/`PolicyView.tsx`'s own success notice already
// promises ("signed console_command recorded, visible in the Bus tab").
// Mirrored here so that promise is true in the mock too.

const COMMAND_EVENTS_CAP = 12;
/** Newest-first, for batch reads (`recent_events`) - capped, never
 * destructively drained, so it survives being read by more than one
 * caller. */
const recentCommandEvents: UiEvent[] = [];
/** FIFO, drained one-per-tick by the live bus subscription (`mockSubscribe`),
 * so a just-issued command shows up on the Bus tab within one tick instead of
 * waiting for the next random synthetic event to happen to cover it. */
const pendingLiveDelivery: UiEvent[] = [];

function emitConsoleCommand(
  action: string,
  target: string,
  decision: "allow" | "break_glass",
  verifyResult: string,
  operator: string,
): void {
  eventSeq += 1;
  const evt: UiEvent = {
    id: eventSeq,
    env: "live",
    ts: new Date().toISOString(),
    source: "console",
    type: "console_command",
    agent_id: `agent://${ORG}/console/demo-operator`,
    run_id: null,
    severity: decision === "break_glass" ? "high" : "low",
    schema: "taipanbox.dev/agent-event/v0.2",
    on_behalf_of: [operator],
    data: {
      action,
      target,
      decision,
      sig_alg: "es256",
      sig_fpr: "software-signed",
      http_status: 200,
      verify_result: verifyResult,
    },
    prev_hash: null,
    raw: "",
    file: "/root/.stack-up/events/console.ndjson",
    off: eventSeq,
  };
  recentCommandEvents.unshift(evt);
  if (recentCommandEvents.length > COMMAND_EVENTS_CAP) recentCommandEvents.length = COMMAND_EVENTS_CAP;
  pendingLiveDelivery.push(evt);
}

export function mockAgentRecord(id: string): FleetAgent | null {
  const a = FLEET.find((x) => agentId(x) === id) ?? null;
  if (!a) return null;
  // Gentle drift on the historical totals only - the protagonist's own live
  // incident arc lives entirely in `money_runs` (see `runFor`), never here:
  // an agent's all-time spend should not jump just because its CURRENT run
  // is having a bad time, the same way a real system only reconciles a run's
  // spend into history once the run itself ends. `effectiveActivity` freezes
  // that drift while the agent is blocked, and `blocked`/`lifecycle` reflect
  // the manual store so the card's status badge is app-wide-consistent.
  const act = effectiveActivity(a);
  return {
    ...a,
    spentUsd: Number((a.spentUsd + act.spentUsd).toFixed(2)),
    calls: a.calls + act.calls,
    blocked: isAgentBlocked(a),
    lifecycle: agentLifecycleState(a),
  };
}

// ---------------------------------------------------------------------------
// Derived wire DTOs.
// ---------------------------------------------------------------------------

/** Every non-protagonist agent's live run, and the protagonist's own run when
 * "calm" is selected: the original pseudo-random utilisation snapshot, now
 * with `liveDrift` layered on top so the numbers still creep forward tick
 * over tick, plus the generic operator kill/budget overrides so a Kill/Set
 * budget click on ANY run - not just the protagonist's incident arc - sticks.
 * `ignoreClosed` is for the protagonist in "calm": its `closed` record
 * describes a PAST, already-resolved incident (`FleetAgent.closed`) - calm's
 * own story is "nothing is on fire right now", so that record is not held
 * against its current live run. */
function baseRunFor(a: FleetAgent, opts: { ignoreClosed?: boolean } = {}) {
  const runId = liveRunIdFor(a);
  // `effectiveActivity` freezes the drift while the agent is blocked, so a
  // stopped/frozen/killed agent stops accruing here (its spend/calls hold).
  const act = effectiveActivity(a);
  const spentUsd = Number((a.spentUsd + act.spentUsd).toFixed(2));
  const calls = a.calls + act.calls;
  const closed = opts.ignoreClosed ? false : Boolean(a.closed);
  const util = closed ? 1.18 : 0.4 + pseudo(a.name + "u") * 0.46;
  const budget = manualRunBudgets.get(runId) ?? Number((spentUsd / util).toFixed(2));
  // The whole lifecycle from the one store: `closed` (via `ignoreClosed`),
  // an operator freeze, a stopped unit/user, or a STICKY operator kill
  // (`killedRuns`, replacing the old 20s self-heal so a kill persists).
  const lifecycle = agentLifecycleState(a, opts.ignoreClosed);
  return {
    run_id: runId,
    model: a.model,
    agent_id: agentId(a),
    spent_usd: spentUsd,
    budget_usd: budget,
    calls,
    cache_hits: Math.round(calls * 0.12),
    steps: Math.min(calls, 40),
    last_seen: ago(Math.random() * 60_000),
    killed: lifecycle !== "live",
    lifecycle,
  };
}

/** The protagonist's live run, in the "incident" scenario, is fully
 * world-driven (see `reconcileProtagonist`) instead of the generic snapshot
 * formula: this is the one run whose spend/budget fraction actually climbs
 * past 1.0 and gets killed. Every other agent, and the protagonist itself in
 * "calm", uses `baseRunFor`. */
function runFor(a: FleetAgent) {
  if (a.id !== PROTAGONIST_ID) return baseRunFor(a);
  if (currentScenario !== "incident") return baseRunFor(a, { ignoreClosed: true });

  const state = reconcileProtagonist();
  // An operator freeze on the protagonist, or a Stop on SRE / on its owner,
  // overlays cleanly on top of its live incident arc: the manual state wins the
  // badge and halts the run, otherwise the arc's own live/killed state decides.
  const manual = manualAgentState(a);
  const spent = Number((state.fraction * PROTAGONIST_RUN_BUDGET_USD).toFixed(2));
  const budget = manualRunBudgets.get(PROTAGONIST_RUN_ID) ?? PROTAGONIST_RUN_BUDGET_USD;
  const calls = Math.round(70 + state.fraction * 340);
  const lifecycle: EntityLifecycleState = manual ?? (state.killed ? "killed" : "live");
  return {
    run_id: PROTAGONIST_RUN_ID,
    model: a.model,
    agent_id: agentId(a),
    spent_usd: spent,
    budget_usd: budget,
    calls,
    cache_hits: Math.round(calls * 0.04),
    steps: Math.min(calls, 40),
    last_seen: state.killed ? new Date(Math.min(Date.now(), armedAt + state.arcElapsedMs)).toISOString() : new Date().toISOString(),
    killed: state.killed || manual !== null,
    lifecycle,
  };
}

/** I11 fixture: `spendSeries` (lib/dashData.ts) needs at least two distinct
 * `last_seen` timestamps to draw anything - below that it returns `[]`
 * (`Sparkline`'s own "< 2 values" empty case). Every fixture agent otherwise
 * has exactly the one live run `runFor` produces above, so NO agent's own
 * spend trend would ever render in Agent 360's Drift section under
 * `dev:mock` without this: a short run history for the SAME agent the drift
 * events below are attached to ({@link DRIFT_DEMO_AGENT_ID}), so its
 * per-agent `runs` (Agent 360 already filters `money_runs` to
 * `run.agent_id === agentId`) has more than the one point. A modest,
 * gently-rising spend trend - nothing dramatic, this agent's story is the
 * quality regression below, not a cost incident. */
function mockDriftAgentRunHistory() {
  const model = "claude-haiku-4-5";
  const points = [
    { agoMs: 10 * DAY, spentUsd: 1.12, calls: 210 },
    { agoMs: 7 * DAY, spentUsd: 1.38, calls: 236 },
    { agoMs: 4 * DAY, spentUsd: 1.61, calls: 258 },
    { agoMs: 1 * DAY, spentUsd: 2.27, calls: 301 },
  ];
  return points.map((p, i) => ({
    run_id: `data-quality-checker-hist-${i + 1}`,
    model,
    agent_id: DRIFT_DEMO_AGENT_ID,
    spent_usd: p.spentUsd,
    budget_usd: 3,
    calls: p.calls,
    cache_hits: Math.round(p.calls * 0.1),
    steps: Math.min(p.calls, 40),
    last_seen: ago(p.agoMs),
    killed: false,
  }));
}

function mockRuns() {
  return [...FLEET.map(runFor), ...mockDriftAgentRunHistory()];
}

function mockOverview() {
  const runs = mockRuns();
  const spent = runs.reduce((s, r) => s + r.spent_usd, 0);
  const calls = runs.reduce((s, r) => s + r.calls, 0);
  const incidents = mockIncidents();
  return {
    total_spent_usd: Number(spent.toFixed(2)),
    total_calls: calls,
    total_runs: runs.length,
    active_runs: runs.filter((r) => !r.killed).length,
    killed_runs: runs.filter((r) => r.killed).length,
    open_incidents: incidents.filter((i) => !i.acknowledged).length,
    total_incidents: incidents.length,
    total_saved_usd: mockSavings().total_saved_usd,
  };
}

function mockSavings() {
  if (currentScenario === "incident") reconcileProtagonist();
  const elapsedMin = Math.min(240, Math.max(0, Date.now() - now) / 60_000);
  const cacheSaved = Number((0.15 + elapsedMin * 0.05).toFixed(2));
  const routerSaved = Number((0.1 + elapsedMin * 0.025).toFixed(2));
  const blocked = Number((38.9 + blockedSpendFromIncidents).toFixed(2));
  const budgetBreaks = 61 + budgetBreaksFromIncidents;
  return {
    blocked_spend_usd: blocked,
    cache_saved_usd: cacheSaved,
    router_saved_usd: routerSaved,
    budget_breaks: budgetBreaks,
    total_saved_usd: Number((blocked + cacheSaved + routerSaved).toFixed(2)),
  };
}

/** Calm: only the always-there, already-acknowledged filler row - "near
 * zero", not literally empty. Incident: the protagonist's own arc adds a
 * `spend_spike` while it climbs past 0.8, then swaps that for a
 * `budget_exceeded` + `fanout_explosion` pair once it trips, both cleared
 * (acknowledged) the moment the run is killed - auto or by the operator - and
 * gone again once a fresh arc loops in. Every arc-specific incident id is
 * suffixed with the arc's own `armedAt`, so an ack from a PAST arc can never
 * silently apply to a fresh one reusing the same kind. */
/** The money plane's own per-person rollup (`/v1/owners`, tokenfuse #192).
 *
 * Built from the SAME delegation chain the preview already puts on every bus
 * event (`makeEvent`'s `on_behalf_of: [userId(a.owner)]`), so the demo's two
 * owner answers agree with each other here. On a real estate they routinely
 * will not, because an agent can be owned by one person and run on another's
 * behalf, and the console shows the two groupings side by side rather than
 * reconciling them. The preview cannot manufacture that disagreement honestly,
 * so it does not try to.
 *
 * One row is deliberately `unassigned`: a run whose chain named nobody is the
 * ordinary case on a real box, and a demo where everything is attributed would
 * teach the wrong shape. */
function mockOwners() {
  const byOwner = new Map<string, { spent: number; calls: number; runs: number; agents: Set<string>; last: string }>();
  for (const a of FLEET) {
    const e = byOwner.get(a.owner) ?? { spent: 0, calls: 0, runs: 0, agents: new Set<string>(), last: ago(0) };
    e.spent += a.spentUsd;
    e.calls += a.calls;
    e.runs += 1;
    e.agents.add(a.id);
    byOwner.set(a.owner, e);
  }
  const out = [...byOwner.entries()].map(([owner, e]) => ({
    owner: userId(owner),
    spent_usd: Number(e.spent.toFixed(2)),
    calls: e.calls,
    runs: e.runs,
    agents: e.agents.size,
    last_seen: e.last,
    tool_calls: Math.round(e.calls * 0.4),
  }));
  out.push({
    owner: "unassigned",
    spent_usd: 12.4,
    calls: 806,
    runs: 3,
    agents: 2,
    last_seen: ago(45 * 60_000),
    tool_calls: 240,
  });
  return out.sort((a, b) => b.spent_usd - a.spent_usd);
}

/** Recent web egress for the Web Egress panel, mirroring
 * `crates/api/src/egress/mod.rs`'s shape.
 *
 * WP13 shipped that panel with no preview handler at all, so the published
 * demo showed its honest "nothing was read" card, which is the right card for
 * a box that could not look and the wrong one for a demo that has plenty to
 * show.
 *
 * The fidelity story is the point of the panel, so these rows carry it: a
 * majority served by a backend that enforces per request, a visible minority
 * served only at the navigation, and two refusals with different verdicts.
 * Deterministic (`pseudo`) so screenshots do not jitter. Hosts are RFC 2606
 * documentation domains, never anywhere real. */
function mockEgress() {
  const hosts = [
    "https://docs.example.com",
    "https://status.example.net",
    "https://api.example.org",
    "https://registry.example.com",
    "https://blog.example.net",
  ];
  const fleet = FLEET.slice(0, 8);
  const rows: EgressRow[] = fleet.flatMap((a, i): EgressRow[] => {
    const id = agentId(a);
    const p = pseudo(id + "egress");
    const origin = hosts[i % hosts.length];
    // Every third agent's backend governs the navigation only: the number the
    // panel exists to surface.
    const navigationOnly = i % 3 === 2;
    const out: EgressRow[] = [
      {
        ts: ago(Math.round(p * 90 * 60_000)),
        agent_id: id,
        run_id: `${a.name}-live`,
        outcome: "fetched",
        origin,
        url_sha384: null,
        backend: navigationOnly ? "browser-run" : "kitesurf",
        enforcement: navigationOnly ? "navigation_only" : "per_request",
        content_bytes: 1_200 + Math.round(p * 40_000),
        verdict: null,
        reason: null,
      },
    ];
    if (i === 1) {
      out.push({
        ts: ago(Math.round(p * 40 * 60_000)),
        agent_id: id,
        run_id: `${a.name}-live`,
        outcome: "blocked",
        origin: "https://paste.example.org",
        url_sha384: null,
        backend: null,
        enforcement: null,
        content_bytes: null,
        verdict: "deny_policy",
        reason: "destination outside the allowed set for this run",
      });
    }
    if (i === 4) {
      out.push({
        ts: ago(Math.round(p * 20 * 60_000)),
        agent_id: id,
        run_id: `${a.name}-live`,
        outcome: "blocked",
        origin: "https://internal.example.com",
        url_sha384: null,
        backend: null,
        enforcement: null,
        content_bytes: null,
        verdict: "deny_address_range",
        reason: "resolves inside a private address range",
      });
    }
    return out;
  });

  const fetched = rows.filter((r) => r.outcome === "fetched");
  const blocked = rows.filter((r) => r.outcome === "blocked");
  const by_verdict: Record<string, number> = {};
  for (const b of blocked) by_verdict[b.verdict ?? "unrecorded"] = (by_verdict[b.verdict ?? "unrecorded"] ?? 0) + 1;

  return {
    measured: true,
    note: `Read from the ${rows.length * 12} most recent events on the bus. An older fetch than that is in the Bus Explorer, not here.`,
    totals: {
      fetched: fetched.length,
      blocked: blocked.length,
      by_verdict,
      navigation_only: fetched.filter((r) => r.enforcement === "navigation_only").length,
      // The passthrough backend reports what a page asked for; the
      // navigation-only one cannot, and that gap is a count of its own.
      subresources_unknown: fetched.filter((r) => r.enforcement === "navigation_only").length,
    },
    rows: rows.sort((x, y) => (x.ts < y.ts ? 1 : -1)),
  };
}

/** Per-agent event counts for the Statistics view, mirroring
 * `crates/api/src/stats/mod.rs`'s shape.
 *
 * Deterministic per agent (`pseudo`), for the same reason spend is: a
 * screenshot must not jitter between reloads. The protagonist's row is layered
 * on top from the live incident arc, so the agent the demo is about is the one
 * that stands out here too, exactly as it does on Overview and Money.
 *
 * Most of the fleet counts zero, and that is deliberate rather than thin: a
 * governed estate where every agent is being blocked daily is not the story
 * this console tells. */
/** A year of events per agent, deterministic and dated, so the preview can
 * answer a window the way a box with durable history does.
 *
 * This used to be a single static count per agent with a note explaining that
 * the preview only held one session. That note was true and it was also the
 * reason the window buttons did nothing here. Seeding real dated events instead
 * removes the need for the explanation rather than hiding it: 24h, 7d, 30d and
 * 1y now select genuinely different slices, because there is genuinely a year
 * of events behind them.
 *
 * Weighted toward the recent end (`p ** 1.7`), because an estate that has been
 * running a year has grown, and a flat spread over twelve months reads as
 * synthetic at a glance. */
const YEAR_MS = 365 * DAY;

interface SeededEvent {
  atMs: number;
  kind: "blocked" | "anomaly" | "budget";
  byOperator: boolean;
  /** Set when this odd-behaviour event is an idryx `identity_finding`; the
   * detector name is what makes it describable. Null for the money plane's own
   * runaway shapes, which are already named by their type. */
  detector: string | null;
}

/** Real detector names from idryx's own catalogue, so the preview teaches the
 * vocabulary an operator will actually meet rather than a plausible-looking
 * invention. */
const IDRYX_DETECTORS = [
  "over_privileged_nhi",
  "impossible_travel",
  "behavior_anomaly",
  "attestation_missing",
  "new_device",
  "shadow_mcp",
  "excessive_agency",
  "beaconing",
];

/** One agent's year, built once and reused across renders and windows. */
const seededYear = new Map<string, SeededEvent[]>();

function yearFor(a: FleetAgent): SeededEvent[] {
  const cached = seededYear.get(a.id);
  if (cached) return cached;

  const id = agentId(a);
  const base = pseudo(id);
  // Between 0 and ~48 events over the year. A third of the fleet is quiet all
  // year, which is what a governed estate actually looks like.
  const total = base < 0.34 ? 0 : Math.round(base * 48);
  const out: SeededEvent[] = [];
  for (let i = 0; i < total; i++) {
    const p = pseudo(`${id}:${i}`);
    // Recent-weighted: p**1.7 clusters toward 0, which is "days ago = few".
    const daysAgo = Math.pow(p, 1.7) * 365;
    const roll = pseudo(`${id}:kind:${i}`);
    const kind: SeededEvent["kind"] = roll < 0.62 ? "blocked" : roll < 0.88 ? "anomaly" : "budget";
    // Roughly half the odd behaviour is an idryx finding, which is the shape a
    // real estate has: the money plane sees runaways, the identity plane sees
    // permissions and devices, and they are not the same events. Names are
    // idryx's own detectors, not invented ones.
    const detector =
      kind === "anomaly" && pseudo(`${id}:det:${i}`) < 0.5
        ? IDRYX_DETECTORS[Math.floor(pseudo(`${id}:detn:${i}`) * IDRYX_DETECTORS.length)]
        : null;
    out.push({
      atMs: now - daysAgo * DAY,
      kind,
      // A small slice of stops were a person's call, the same shape the real
      // counter reports from `console.block_*` and an actor-named kill.
      byOperator: kind === "blocked" && pseudo(`${id}:op:${i}`) < 0.08,
      detector,
    });
  }
  seededYear.set(a.id, out);
  return out;
}

/** Per-agent event counts for the Statistics view, mirroring
 * `crates/api/src/stats/mod.rs`'s shape, over a real window.
 *
 * `windowDays` of 0 means every event held, which here is the full seeded year.
 * Anything else selects by each event's own timestamp, exactly as the real
 * counter does against a durable store. */
function mockStatsCounts(windowDays: number) {
  const cutoff = windowDays > 0 ? now - windowDays * DAY : now - YEAR_MS - DAY;
  let scanned = 0;

  const agents = FLEET.map((a) => {
    const id = agentId(a);
    const inWindow = yearFor(a).filter((e) => e.atMs >= cutoff);
    scanned += inWindow.length;

    const blocked = inWindow.filter((e) => e.kind === "blocked").length;
    const anomalies = inWindow.filter((e) => e.kind === "anomaly").length;
    const budget = inWindow.filter((e) => e.kind === "budget").length;
    const byOperator = inWindow.filter((e) => e.byOperator).length;

    // An agent an operator has ACTUALLY halted in this session counts now,
    // exactly as it does on a real box, where the freeze journals a
    // `console.block_agent` line naming the agents it stopped.
    const haltedNow =
      frozenAgents.has(a.id) || stoppedUnits.has(a.team) || stoppedUsers.has(a.owner) ? 1 : 0;
    if (haltedNow) scanned += 1;

    const by_detector: Record<string, number> = {};
    for (const e of inWindow) {
      if (e.detector) by_detector[e.detector] = (by_detector[e.detector] ?? 0) + 1;
    }
    const by_type: Record<string, number> = {};
    if (blocked) by_type["policy_deny"] = blocked;
    const findings = Object.values(by_detector).reduce((a, b) => a + b, 0);
    if (anomalies - findings > 0) by_type["sustained_loop"] = anomalies - findings;
    if (findings) by_type["identity_finding"] = findings;
    if (budget) by_type["budget_threshold"] = budget;

    const last = inWindow.reduce((m, e) => Math.max(m, e.atMs), 0);
    return {
      agent_id: id,
      blocked: blocked + haltedNow,
      blocked_by_operator: byOperator + haltedNow,
      anomalies,
      budget_events: budget,
      // Only where a budget event carried both amounts, which is the honest
      // common case on a real box too.
      worst_overshoot_microusd: budget ? Math.round(pseudo(`${id}:over`) * 400_000) : null,
      by_type,
      by_detector,
      last_seen: new Date(last || now).toISOString(),
    };
  });

  // The runaway, from the same incident arc every other panel reads.
  const protagonist = agents.find((a) => a.agent_id === PROTAGONIST_ID);
  if (protagonist) {
    const state = currentScenario === "incident" ? reconcileProtagonist() : null;
    const over = state ? Math.max(0, state.fraction - 1) : 0;
    protagonist.blocked += 26;
    protagonist.anomalies += 3;
    protagonist.budget_events += 2;
    protagonist.blocked_by_operator += 1;
    protagonist.by_type["breaker_tripped"] = 26;
    protagonist.by_type["fanout_explosion"] = 3;
    protagonist.worst_overshoot_microusd = Math.round(
      Math.max(over, 0.28) * PROTAGONIST_RUN_BUDGET_USD * 1_000_000,
    );
    protagonist.last_seen = new Date().toISOString();
    scanned += 31;
  }

  return {
    measured: true,
    note:
      windowDays === 0
        ? `Counted from all ${scanned} event(s), every age this box holds.`
        : `Counted from all ${scanned} event(s) in the last ${windowDays} day(s), by each event's own timestamp.`,
    scanned,
    // The preview computes every figure from the seeded year in one pass, so
    // there is no second capped read to truncate. Reported as zero and false
    // rather than omitted: a view that has to treat `undefined` as "probably
    // fine" is a view that will read a real truncation the same way.
    detail_scanned: 0,
    detail_truncated: false,
    window_days: windowDays,
    history_from: new Date(now - YEAR_MS).toISOString(),
    undated: 0,
    agents,
  };
}

function mockIncidents() {
  const ra = PROTAGONIST;
  const rid = PROTAGONIST_ID;
  const out: {
    id: string;
    run_id: string | null;
    agent_id: string | null;
    kind: string;
    severity: string;
    first_seen: string;
    last_seen: string;
    occurrences: number;
    acknowledged: boolean;
  }[] = [
    {
      id: "sustained_loop:query-cost-optimizer",
      run_id: "query-cost-optimizer-live",
      agent_id: `agent://${ORG}/data/query-cost-optimizer`,
      kind: "sustained_loop",
      severity: "medium",
      first_seen: ago(30 * 60_000),
      last_seen: ago(12 * 60_000),
      occurrences: 4,
      acknowledged: true,
    },
  ];

  if (currentScenario === "incident") {
    const state = reconcileProtagonist();
    const arcSuffix = armedAt.toString(36);
    const lastSeen = new Date(Math.min(Date.now(), armedAt + state.arcElapsedMs)).toISOString();

    if (state.phase === "climbing" && state.fraction >= 0.8) {
      const id = `spend_spike:${ra.name}-${arcSuffix}`;
      out.push({
        id,
        run_id: PROTAGONIST_RUN_ID,
        agent_id: rid,
        kind: "spend_spike",
        severity: "medium",
        first_seen: new Date(armedAt + CLIMB_MS * 0.5).toISOString(),
        last_seen: lastSeen,
        occurrences: Math.max(1, Math.round((state.fraction - 0.6) * 30)),
        acknowledged: ackedIncidentIds.has(id),
      });
    } else if (state.phase === "tripped" || state.killed) {
      const beId = `budget_exceeded:${ra.name}-${arcSuffix}`;
      const feId = `fanout_explosion:${ra.name}-${arcSuffix}`;
      const acked = state.killed;
      const cappedFraction = Math.min(state.fraction, 1.28);
      out.push({
        id: beId,
        run_id: PROTAGONIST_RUN_ID,
        agent_id: rid,
        kind: "budget_exceeded",
        severity: "high",
        first_seen: new Date(armedAt + CLIMB_MS).toISOString(),
        last_seen: lastSeen,
        occurrences: Math.max(1, Math.round((cappedFraction - 1) * 40) + 1),
        acknowledged: acked || ackedIncidentIds.has(beId),
      });
      out.push({
        id: feId,
        run_id: PROTAGONIST_RUN_ID,
        agent_id: rid,
        kind: "fanout_explosion",
        severity: "critical",
        first_seen: new Date(armedAt + CLIMB_MS).toISOString(),
        last_seen: lastSeen,
        occurrences: Math.max(1, Math.round((cappedFraction - 1) * 60) + 1),
        acknowledged: acked || ackedIncidentIds.has(feId),
      });
    }
  }
  return out;
}

// ---------------------------------------------------------------------------
// Delegation topology (Agent 360's Delegation section + the mini focus graph).
//
// The human owner already delegates to every agent (the `user -> agent` edges
// `mockGraph` emits below, so every agent's slice already has its owner as a
// parent). This adds the agent -> agent layer on top: a lead/orchestrator
// agent delegating to worker agents, exactly the shape
// `crates/core/src/graph.rs` builds from an `on_behalf_of` chain (an edge
// points from the delegator to the delegatee; a node's `parents` are its
// delegators, its `children` its delegatees). Kept as one explicit,
// hand-picked list so the graph reads as a believable fleet rather than a
// star, and so the two Agent 360 acceptance agents (`sre/rca-copilot`,
// `finops/unit-economics-analyst`) are genuine hubs - each with both a parent
// orchestrator and several children. Every id here is a real fleet suffix
// (see UNITS above); a typo would simply produce an edge to a node that never
// renders, never an error.
// ---------------------------------------------------------------------------

const DELEGATION_EDGES: readonly (readonly [string, string])[] = (() => {
  const out: [string, string][] = [];
  const link = (team: string, parent: string, children: string[]) => {
    for (const child of children) {
      out.push([`agent://${ORG}/${team}/${parent}`, `agent://${ORG}/${team}/${child}`]);
    }
  };
  // SRE: the triage copilot fans an incident out; rca-copilot is its own sub-hub.
  link("sre", "incident-triage-copilot", ["rca-copilot", "runbook-executor", "oncall-summarizer"]);
  link("sre", "rca-copilot", ["alert-correlator", "log-analyzer", "postmortem-writer"]);
  // FinOps: the anomaly detector escalates; unit-economics-analyst is its own sub-hub.
  link("finops", "cost-anomaly-detector", ["unit-economics-analyst", "rightsizing-advisor"]);
  link("finops", "unit-economics-analyst", ["chargeback-reporter", "savings-plan-optimizer", "budget-forecaster"]);
  // Platform + Data: enough delegation that the graph reads as a fleet, not a star.
  link("platform", "iac-reviewer", ["terraform-drift-detector", "k8s-config-linter"]);
  link("platform", "ci-optimizer", ["build-cost-analyzer", "flaky-test-hunter", "dependency-upgrader"]);
  link("data", "pipeline-monitor", ["schema-drift-detector", "data-quality-checker"]);
  link("data", "dbt-model-reviewer", ["lineage-mapper"]);
  return out;
})();

function delegationParentIds(id: string): string[] {
  return DELEGATION_EDGES.filter(([, to]) => to === id).map(([from]) => from);
}
function delegationChildIds(id: string): string[] {
  return DELEGATION_EDGES.filter(([from]) => from === id).map(([, to]) => to);
}

/** A delegation-graph node for one fleet agent id, carrying the same live
 * event count (`calls + liveDrift`) `mockGraph`/`mockIdentities` use, so a
 * chip's node and the same agent's own graph node agree. */
function delegationNode(id: string): { id: string; kind: "agent"; event_count: number; last_ts: string } {
  const rec = FLEET.find((x) => x.id === id);
  return {
    id,
    kind: "agent" as const,
    event_count: rec ? rec.calls + effectiveActivity(rec).calls : 0,
    last_ts: rec ? ago(60_000) : "",
  };
}

function mockGraph() {
  const nodes: { id: string; kind: "user" | "agent" | "other"; event_count: number; x: number; y: number; lifecycle?: EntityLifecycleState }[] = [];
  const users = [...new Set(FLEET.map((a) => a.owner))];
  users.forEach((u, i) => {
    nodes.push({ id: userId(u), kind: "user", event_count: 0, x: 120, y: 70 + i * 66 });
  });
  // Agents in a tidy grid so labels never overlap, six per row. `lifecycle`
  // marks a stopped/frozen/killed node so the graph reflects the store too;
  // `effectiveActivity` freezes a blocked node's event_count.
  FLEET.forEach((a, k) => {
    const col = k % 6;
    const row = Math.floor(k / 6);
    nodes.push({ id: agentId(a), kind: "agent", event_count: a.calls + effectiveActivity(a).calls, x: 440 + col * 200, y: 80 + row * 96, lifecycle: agentLifecycleState(a) });
  });
  // Every agent's human owner delegates to it, plus the agent -> agent
  // delegation layer (see DELEGATION_EDGES) so a focused agent's 1-hop
  // neighborhood - the mini graph in Agent 360, computed by
  // `DelegationGraphView` from these edges - shows its parent orchestrator and
  // its children, not just its owner.
  const edges = [
    ...FLEET.map((a) => ({ from: userId(a.owner), to: agentId(a) })),
    ...DELEGATION_EDGES.map(([from, to]) => ({ from, to })),
  ];
  return { nodes, edges, width: 1700, height: Math.max(1500, users.length * 66 + 120) };
}

function mockSlice(id: string) {
  const a = mockAgentRecord(id);
  if (!a) return { node: null, parents: [], children: [] };
  // Parents = this agent's delegators: its human owner (always) plus any
  // orchestrator agent that delegates to it. Children = the agents it
  // delegates to. Both mirror what `crates/core/src/graph.rs`'s
  // `parents`/`children` would return for the same edges `mockGraph` emits.
  return {
    node: { id, kind: "agent" as const, event_count: a.calls, last_ts: ago(30_000) },
    parents: [
      { id: userId(a.owner), kind: "user" as const, event_count: 0, last_ts: "" },
      ...delegationParentIds(id).map(delegationNode),
    ],
    children: delegationChildIds(id).map(delegationNode),
  };
}

// Agents whose envelope requires a human sign-off produce the pending
// approvals, decided-state overlaid from `decidedApprovals` so a
// `policy_decide_approval` call actually sticks. During an active "incident"
// arc, the protagonist's own climbing run additionally surfaces its own
// synthetic "may I keep spending" approval (see `reconcileProtagonist`),
// pinned to the front so it reads as the operator's most pressing item.
function mockApprovals() {
  const needHuman = FLEET.filter((a) => a.allowed.some((x) => x.includes("human")));
  const pick = needHuman.slice(0, 6);
  const rows = pick.map((a, i) => {
    const approvalId = `ap_${(pseudo(a.name + "ap") * 1e12).toString(16).slice(0, 10)}`;
    const decided = decidedApprovals.get(approvalId) ?? null;
    return {
      approval_id: approvalId,
      agent_id: agentId(a),
      run_id: `${a.name}-live`,
      requested_at: ago((1 + i) * 90_000),
      decided_at: decided ? new Date(decided.decidedAt).toISOString() : (null as string | null),
      decided_by: decided ? "console-op" : (null as string | null),
      decision: decided ? decided.decision : (null as string | null),
      pending: !decided,
      tool_names: a.allowed.filter((x) => x.includes("_read") || x.includes("run") || x.includes("apply")).slice(0, 2),
      est_cost_usd: Number((8 + pseudo(a.name + "c") * 40).toFixed(1)),
      reason: `estimated cost exceeds the ${a.team} human-approval threshold; approval required`,
      on_behalf_of: [userId(a.owner)],
      policy_version: "356f49daa246",
      org: ORG,
      model: a.model,
    };
  });

  if (currentScenario === "incident") {
    reconcileProtagonist();
    if (arcApproval) {
      rows.unshift({
        approval_id: arcApproval.id,
        agent_id: PROTAGONIST_ID,
        run_id: PROTAGONIST_RUN_ID,
        requested_at: new Date(arcApproval.requestedAt).toISOString(),
        decided_at: arcApproval.decidedAt ? new Date(arcApproval.decidedAt).toISOString() : null,
        decided_by: arcApproval.decided ? "console-op" : null,
        decision: arcApproval.decision,
        pending: !arcApproval.decided,
        tool_names: ["correlate", "traces_read"],
        est_cost_usd: PROTAGONIST_RUN_BUDGET_USD,
        reason: `${PROTAGONIST.name} is climbing past its per-run ceiling; approve to keep it running or deny to stop it`,
        on_behalf_of: [userId(PROTAGONIST.owner)],
        policy_version: "356f49daa246",
        org: ORG,
        model: PROTAGONIST.model,
      });
    }
  }
  return rows;
}

/** `policy_decide_approval`'s mock: settles the protagonist's own synthetic
 * incident approval when `id` matches it (denying it is equivalent to the
 * operator choosing to stop the run - the same resolution path a manual
 * kill takes; granting it just settles the request and lets the arc
 * continue on its own climb/auto-resolve timeline), otherwise settles a
 * generic fixed approval from `mockApprovals`'s `needHuman` list. Idempotent:
 * deciding an already-decided approval again just re-reports its settled
 * state rather than re-triggering any side effect. */
function decideApprovalMock(id: string, decision: "grant" | "deny") {
  if (currentScenario === "incident") {
    reconcileProtagonist();
    if (arcApproval && arcApproval.id === id) {
      if (!arcApproval.decided) {
        arcApproval = { ...arcApproval, decided: true, decision, decidedAt: Date.now() };
        if (decision === "deny" && manualKillAt === null) {
          manualRunKills.set(PROTAGONIST_RUN_ID, Date.now());
          reconcileProtagonist();
        }
      }
      const granted = arcApproval.decision === "grant";
      return {
        summary: `${decision === "grant" ? "Granted" : "Denied"} continued spend on ${PROTAGONIST.name}`,
        http_status: 200,
        verify_result: `decision:${decision}`,
        sig_alg: "es256",
        sig_fpr: "software-signed",
        token: granted
          ? {
              agent_id: PROTAGONIST_ID,
              run_id: PROTAGONIST_RUN_ID,
              tools: PROTAGONIST.allowed.slice(0, 3),
              cost_ceiling_usd: manualRunBudgets.get(PROTAGONIST_RUN_ID) ?? PROTAGONIST_RUN_BUDGET_USD,
              exp_unix: Math.floor(Date.now() / 1000) + 900,
            }
          : null,
        bus_recorded: true,
        bus_error: null,
      };
    }
  }

  const decidedAt = Date.now();
  decidedApprovals.set(id, { decision, decidedAt });
  const src = mockApprovals().find((a) => a.approval_id === id) ?? null;
  return {
    summary: `${decision === "grant" ? "Granted" : "Denied"} approval ${id || "(unknown)"}`,
    http_status: 200,
    verify_result: `decision:${decision}`,
    sig_alg: "es256",
    sig_fpr: "software-signed",
    token:
      decision === "grant" && src
        ? { agent_id: src.agent_id, run_id: src.run_id, tools: src.tool_names, cost_ceiling_usd: src.est_cost_usd ?? 25, exp_unix: Math.floor(Date.now() / 1000) + 900 }
        : null,
    bus_recorded: true,
    bus_error: null,
  };
}

function mockPolicies() {
  const pol = (o: Record<string, unknown>) => ({
    deny_tool: [] as string[],
    allow_domains: [] as string[],
    require_human_above_usd: 0,
    deny_above_usd: 0,
    max_steps: 0,
    deny_if_unattested: false,
    updated_at: ago(3 * DAY),
    ...o,
  });
  return [
    pol({ id: "sre-runbook-approval", name: "sre-runbook-approval", target: `agent://${ORG}/sre/runbook-executor`, require_human_above_usd: 25 }),
    pol({ id: "sre-deploy-approval", name: "sre-deploy-approval", target: `agent://${ORG}/sre/deploy-guard`, require_human_above_usd: 10 }),
    pol({ id: "deny-shell-exec", name: "deny-shell-exec", target: `agent://${ORG}/*`, deny_tool: ["shell_exec", "prod_delete"] }),
    pol({ id: "finops-spend-cap", name: "finops-spend-cap", target: `agent://${ORG}/finops/*`, require_human_above_usd: 12, deny_above_usd: 20 }),
    pol({ id: "data-pii-attestation", name: "data-pii-attestation", target: `agent://${ORG}/data/*`, deny_if_unattested: true }),
    pol({ id: "platform-secret-dlp", name: "platform-secret-dlp", target: `agent://${ORG}/platform/secret-scanner`, deny_tool: ["external_send"] }),
    pol({ id: "rca-max-steps", name: "rca-max-steps", target: `agent://${ORG}/sre/rca-copilot`, max_steps: 12 }),
    // I5 "Access matrix" fixtures: two `allow_domains` policies whose targets
    // glob-match the same fixture agent (a platform-wide one plus a narrower
    // exact-target one), so `lib/access.ts`'s effective-intersection logic
    // has a genuinely non-trivial case to render under `dev:mock` (the
    // platform-wide list's `api.github.com` is NOT in the narrower list, so
    // the effective allowed set is the one domain both share).
    pol({
      id: "platform-egress-allowlist",
      name: "platform-egress-allowlist",
      target: `agent://${ORG}/platform/*`,
      allow_domains: ["api.github.com", "registry.npmjs.org"],
    }),
    pol({
      id: "dependency-upgrader-domains",
      name: "dependency-upgrader-domains",
      target: `agent://${ORG}/platform/dependency-upgrader`,
      allow_domains: ["registry.npmjs.org"],
    }),
  ];
}

function utcStamp(msAgo: number) {
  return new Date(now - msAgo).toISOString().replace("T", " ").slice(0, 19) + " UTC";
}

/** `"YYYY-MM-DD"`, the exact shape `crates/api/src/onboard/commands.rs::today`
 * stamps on a real identity-map fragment - used only for `GatewayKeyEntry.created`
 * below, distinct from `utcStamp`'s full-timestamp format. */
function dateStamp(msAgo: number) {
  return new Date(now - msAgo).toISOString().slice(0, 10);
}

/**
 * I5 "Access matrix" fixtures: per-agent permission overrides, keyed by
 * agent id, so `dev:mock` has real (non-empty) `IdryxPermission[]` to build
 * a meaningful matrix from - every other fixture agent stays at `[]`
 * (granted 0, a legitimately empty row) rather than inventing permissions
 * for the whole 42-agent fleet. Three cases the spec calls for by name:
 *
 * - `incident-triage-copilot`: a used/unused mix INCLUDING an unused admin
 *   permission (`pagerduty_admin`) - the escalated "Unused" state, and its
 *   `pagerduty_read`/`pagerduty_admin` overlap the sanctioned MCP server
 *   below (sanctioned MCP reach).
 * - `alert-correlator`: every permission has `used: false` and NONE has
 *   `used: true` - the honesty-gate "no usage signal" state (not "2
 *   unused"), mirroring idryx's own `least_privilege` detector staying
 *   silent here.
 * - `dependency-upgrader`: `scratch_notes_write` overlaps the SHADOW MCP
 *   server below (shadow MCP reach), paired with the `agent_shadow_tool`
 *   alert further down so the two signals agree, per the task spec ("do not
 *   special-case it, but include the alert count in the row model").
 *
 * The two Agent 360 acceptance agents also get real permissions here so their
 * Access section and MCP reach are non-empty (both overlap the sanctioned
 * observability MCP server below):
 *
 * - `sre/rca-copilot`: its real tools (`traces_read`/`correlate`) plus
 *   `pagerduty_read` (reaches BOTH the observability and pagerduty sanctioned
 *   servers) and one unused write permission (`incident_write`).
 * - `finops/unit-economics-analyst`: `metrics_read` (reaches the
 *   observability server) plus an UNUSED ADMIN permission (`cost_admin`) - the
 *   escalated least-privilege finding for the fleet's top spender.
 */
const FIXTURE_PERMISSIONS: Record<string, { name: string; admin: boolean; used: boolean }[]> = {
  [`agent://${ORG}/sre/incident-triage-copilot`]: [
    { name: "pagerduty_read", admin: false, used: true },
    { name: "logs_read", admin: false, used: true },
    { name: "pagerduty_admin", admin: true, used: false },
    { name: "incident_write", admin: false, used: false },
  ],
  [`agent://${ORG}/sre/alert-correlator`]: [
    { name: "metrics_read", admin: false, used: false },
    { name: "alerts_dedupe", admin: false, used: false },
  ],
  [`agent://${ORG}/platform/dependency-upgrader`]: [
    { name: "deps_read", admin: false, used: true },
    { name: "scratch_notes_write", admin: false, used: true },
  ],
  [`agent://${ORG}/sre/rca-copilot`]: [
    { name: "traces_read", admin: false, used: true },
    { name: "correlate", admin: false, used: true },
    { name: "pagerduty_read", admin: false, used: true },
    { name: "incident_write", admin: false, used: false },
  ],
  [`agent://${ORG}/finops/unit-economics-analyst`]: [
    { name: "metrics_read", admin: false, used: true },
    { name: "billing_read", admin: false, used: true },
    { name: "unit_cost_model", admin: false, used: true },
    { name: "cost_admin", admin: true, used: false },
  ],
};

/** I5 fixtures: three `mcp_server` identities - two sanctioned, one shadow
 * (flagged by the `shadow_mcp` alert in `mockAlerts` below, since idryx
 * exposes no shadow flag over REST). Their own `permissions` are what
 * `lib/access.ts`'s name-intersection join checks fixture agents' own
 * permissions against - see {@link FIXTURE_PERMISSIONS}'s doc comment for
 * which agent overlaps which server. The observability connector is the one
 * both Agent 360 acceptance agents reach (SRE's rca-copilot via `traces_read`,
 * FinOps' unit-economics-analyst via `metrics_read`), so their MCP reach is
 * non-zero. */
function mockMcpServerIdentities() {
  return [
    {
      id: `mcp://${ORG}/sanctioned/pagerduty-connector`,
      type: "mcp_server",
      privileged: false,
      source: "mcp",
      owner: userId("j.carter"),
      created: utcStamp(90 * DAY),
      last_used: utcStamp(2 * 60_000),
      runtime: "mcp-server",
      on_behalf_of: [] as string[],
      permissions: [
        { name: "pagerduty_read", admin: false, used: true },
        { name: "pagerduty_admin", admin: true, used: true },
      ],
      remediation: null,
      rotation: null,
      events: 812,
      alerts: 0,
    },
    {
      id: `mcp://${ORG}/sanctioned/observability-connector`,
      type: "mcp_server",
      privileged: false,
      source: "mcp",
      owner: userId("l.moreau"),
      created: utcStamp(120 * DAY),
      last_used: utcStamp(90_000),
      runtime: "mcp-server",
      on_behalf_of: [] as string[],
      permissions: [
        { name: "metrics_read", admin: false, used: true },
        { name: "traces_read", admin: false, used: true },
        { name: "logs_read", admin: false, used: true },
      ],
      remediation: null,
      rotation: null,
      events: 1_506,
      alerts: 0,
    },
    {
      id: `mcp://${ORG}/shadow/scratch-notes`,
      type: "mcp_server",
      privileged: false,
      source: "mcp",
      owner: userId("t.osei"),
      created: utcStamp(6 * DAY),
      last_used: utcStamp(11 * 60_000),
      runtime: "mcp-server",
      on_behalf_of: [] as string[],
      permissions: [
        { name: "scratch_notes_write", admin: false, used: true },
        { name: "scratch_notes_admin", admin: true, used: false },
      ],
      remediation: null,
      rotation: null,
      events: 44,
      alerts: 1,
    },
  ];
}

function mockIdentities() {
  const agents = FLEET.map((a) => ({
    id: agentId(a),
    type: "agent",
    privileged: a.team === "sre" || a.name.includes("secret") || a.name.includes("pii"),
    source: "tokenfuse",
    owner: userId(a.owner),
    created: utcStamp(15 * DAY),
    last_used: utcStamp(Math.random() * 60_000),
    runtime: a.model,
    on_behalf_of: [userId(a.owner)],
    permissions: FIXTURE_PERMISSIONS[agentId(a)] ?? [],
    remediation: null,
    rotation: null,
    // Frozen while blocked (`effectiveActivity`), so a stopped/frozen/killed
    // agent stops accruing activity in the identity list too.
    events: a.calls + effectiveActivity(a).calls,
    alerts: a.closed ? 2 : 0,
    team: a.team,
  }));
  return [...agents, ...mockMcpServerIdentities()];
}

/** The two protagonist-specific detections are live: they only appear while
 * the current "incident" arc is tripped or has just resolved, with
 * timestamps anchored to the arc itself, rather than the permanent fixed
 * entries this file used to seed unconditionally - "calm" (and an idle
 * incident-arc lull) genuinely has no runaway to detect. Every other row
 * here is an UNRELATED I5/I11 fixture (shadow MCP, access-matrix, drift
 * baseline) and stays exactly as before regardless of scenario. */
function mockAlerts() {
  const ra = PROTAGONIST;
  const out: { detector: string; identity: string; severity: string; time: string; summary: string }[] = [];
  if (currentScenario === "incident") {
    const state = reconcileProtagonist();
    if (state.phase === "tripped" || state.killed) {
      const t = new Date(Math.min(Date.now(), armedAt + state.arcElapsedMs)).toISOString();
      out.push({ detector: "runaway_agent", identity: agentId(ra), severity: "high", time: t, summary: `runaway_agent: budget_exceeded blocks across shards on ${ra.name}` });
      out.push({ detector: "excessive_agency", identity: agentId(ra), severity: "medium", time: new Date(armedAt + CLIMB_MS).toISOString(), summary: "excessive_agency: agent opened many sub-runs in one window" });
    }
  }
  out.push(
    { detector: "over_privileged_nhi", identity: `agent://${ORG}/platform/secret-scanner`, severity: "low", time: ago(3 * DAY), summary: "over_privileged_nhi: secret-scanner holds unused repo_write" },
    { detector: "attestation_missing", identity: `agent://${ORG}/data/pii-scanner`, severity: "medium", time: ago(2 * DAY), summary: "attestation_missing: attestation=none on a data agent under data-pii-attestation" },
    // I5 "Access matrix" fixtures: the shadow_mcp alert is what MAKES the
    // scratch-notes server shadow (idryx exposes no shadow flag over REST -
    // see `lib/access.ts`'s `shadowServerIds` doc comment), and the paired
    // agent_shadow_tool alert is idryx's own detector output for the same
    // join `lib/access.ts` derives independently - both must agree here.
    { detector: "shadow_mcp", identity: `mcp://${ORG}/shadow/scratch-notes`, severity: "high", time: ago(11 * 60_000), summary: "unsanctioned MCP server in use (shadow MCP)" },
    { detector: "agent_shadow_tool", identity: `agent://${ORG}/platform/dependency-upgrader`, severity: "high", time: ago(9 * 60_000), summary: "agent uses tool(s) from an unsanctioned MCP server: scratch_notes_write" },
    // I11 "agent drift card" fixture: idryx's own login-behavior baseline
    // detector (`behavior_anomaly` - see `identityTypes.ts`'s `DETECTOR_IDS`
    // doc), for the SAME agent the quality-drift events below are attached
    // to. Agent 360's Drift section surfaces this alert AS-IS (this card's
    // "idryx baselines" are exactly its existing behavior_anomaly alerts,
    // nothing new fetched from idryx).
    { detector: "behavior_anomaly", identity: DRIFT_DEMO_AGENT_ID, severity: "medium", time: ago(30 * 60_000), summary: "behavior_anomaly: data-quality-checker's call cadence shifted outside its 30-day login-behavior baseline" },
  );
  return out;
}

// Owner and unit aggregates, so a card can navigate agent -> owner -> unit.
function entityAgentFor(a: FleetAgent, portion: number, current: boolean) {
  // Live drift lands on the CURRENT owner/team's own portion only - it
  // represents recent activity, which by definition belongs to whoever holds
  // the agent right now, not a past ownership segment. Frozen while blocked
  // (`effectiveActivity`); `blocked`/`lifecycle` come from the manual store so
  // each row's badge matches the agent card and the dock.
  const act = effectiveActivity(a);
  return {
    agentId: a.id,
    name: a.name,
    team: a.team,
    owner: a.owner,
    model: a.model,
    spentUsd: Number((portion + (current ? act.spentUsd : 0)).toFixed(2)),
    calls: a.calls + act.calls,
    closed: Boolean(a.closed),
    blocked: isAgentBlocked(a),
    lifecycle: agentLifecycleState(a),
    current,
  };
}

function mockUserRecord(userArg: string) {
  const handle = userArg.replace(/^user:\/\/[^/]+\//, "");
  // An agent belongs to a user's record if the user owned it during ANY
  // segment; the spend shown is only that user's segments, so a transferred
  // agent's history splits cleanly between the two owners.
  const agents = FLEET.filter((a) => a.segments.some((s) => s.owner === handle)).map((a) => {
    const portion = a.segments.filter((s) => s.owner === handle).reduce((sum, s) => sum + s.spentUsd, 0);
    return entityAgentFor(a, portion, a.owner === handle);
  });
  return {
    handle,
    agents,
    totalSpentUsd: Number(agents.reduce((s, a) => s + a.spentUsd, 0).toFixed(2)),
    totalCalls: agents.filter((a) => a.current).reduce((s, a) => s + a.calls, 0),
    teams: [...new Set(agents.map((a) => a.team))],
    stopped: stoppedUsers.has(handle),
  };
}
function mockUnitRecord(team: string) {
  const agents = FLEET.filter((a) => a.segments.some((s) => s.team === team)).map((a) => {
    const portion = a.segments.filter((s) => s.team === team).reduce((sum, s) => sum + s.spentUsd, 0);
    return entityAgentFor(a, portion, a.team === team);
  });
  return {
    team,
    agents,
    owners: [...new Set(agents.filter((a) => a.current).map((a) => a.owner))],
    totalSpentUsd: Number(agents.reduce((s, a) => s + a.spentUsd, 0).toFixed(2)),
    totalCalls: agents.filter((a) => a.current).reduce((s, a) => s + a.calls, 0),
    stopped: stoppedUnits.has(team),
  };
}

// ---------------------------------------------------------------------------
// Agent governance actions. These mutate the in-memory fleet and log the
// change to the agent's lifecycle, so the preview can demonstrate reassigning
// a unit, transferring an owner, or editing a budget or behaviour envelope.
// A real box keeps no such editable record yet (flagged), so on a real backend
// these commands simply are not answered.
// ---------------------------------------------------------------------------

function findById(id: string): FleetAgent | null {
  return FLEET.find((a) => a.id === id) ?? null;
}
function logChange(a: FleetAgent, kind: LifecycleEntry["kind"], detail: string) {
  a.history.push({ ts: new Date().toISOString(), kind, detail, actor: "console-op" });
}

function orgDirectory() {
  return {
    teams: UNITS.map((u) => ({ team: u.team, label: u.label })),
    users: UNITS.flatMap((u) => u.users.map((h) => ({ handle: h, team: u.team }))),
  };
}

// ---------------------------------------------------------------------------
// Felyx (copilot) connection, mutable so the Connect form actually connects.
// ---------------------------------------------------------------------------

// Connected by default in the preview (local Ollama), so the Copilot tab shows
// Felyx's seeded conversation straight away. Connect/Change still re-points it.
let copilotConfig: { provider: string; model: string; local: boolean } | null = { provider: "ollama", model: "qwen2.5:7b-instruct", local: true };

function copilotStatus() {
  if (!copilotConfig) {
    return {
      state: "ready",
      enabled: false,
      disabled_reason: "No provider is connected. Open Connect Felyx to choose Anthropic, OpenAI, OpenRouter, Ollama or LM Studio.",
      descriptor: null,
    };
  }
  return {
    state: "ready",
    enabled: true,
    local: copilotConfig.local,
    provider: copilotConfig.provider,
    model: copilotConfig.model,
    descriptor: { provider: copilotConfig.provider, model: copilotConfig.model },
  };
}

// ---------------------------------------------------------------------------
// On-demand plane content: quality evals, crypto scans, fire drills, evidence,
// and Felyx's answers. Coherent with the fleet and honestly framed (unsigned
// bundles, no fabricated migration progress, Felyx reads and proposes only).
// ---------------------------------------------------------------------------

function fakeSha(seed: string): string {
  let s = "";
  for (let i = 0; i < 8; i++) s += Math.floor(pseudo(seed + i) * 4294967296).toString(16).padStart(8, "0");
  return "sha256:" + s.slice(0, 64);
}

// --- Quality (Verdryx) ---
function mockQualityRuns() {
  const defs = [
    { model: "claude-opus-4-5", cases: 30, mean: 0.95 },
    { model: "claude-sonnet-4-5", cases: 48, mean: 0.93 },
    { model: "claude-sonnet-4-5", cases: 44, mean: 0.91 },
    { model: "gpt-4o", cases: 42, mean: 0.9 },
    { model: "claude-haiku-4-5", cases: 60, mean: 0.87 },
    { model: "gpt-4o-mini", cases: 55, mean: 0.82 },
  ];
  return defs.map((d, i) => {
    const startMs = (i + 1) * 6 * 3600 * 1000;
    const tokens = d.cases * (2000 + i * 350);
    return {
      run: { id: `eval-${1000 + i}`, model: d.model, started_at: ago(startMs), finished_at: ago(startMs - 240_000) },
      case_count: d.cases,
      mean_score: d.mean,
      total_tokens: tokens,
      total_cost_usd: Number(((tokens / 1e6) * 6).toFixed(2)),
    };
  });
}
function mockQualityScores(runId: string) {
  const out = [];
  for (let i = 0; i < 12; i++) {
    out.push({
      id: i + 1,
      run_id: runId,
      case_id: `case-${String(i + 1).padStart(2, "0")}`,
      value: Number((0.68 + pseudo(runId + "v" + i) * 0.31).toFixed(2)),
      tokens: 1400 + Math.floor(pseudo(runId + "t" + i) * 4200),
      cost_usd: Number((pseudo(runId + "c" + i) * 0.05).toFixed(4)),
    });
  }
  return out;
}
function mockQualityBaselines() {
  return [
    { id: "base-eom", eval_run_id: "eval-1000", mean_score: 0.92, created_at: ago(28 * DAY), label: "end-of-month gate" },
    { id: "base-release", eval_run_id: "eval-1002", mean_score: 0.88, created_at: ago(96 * DAY), label: "pre-release" },
    { id: "base-haiku", eval_run_id: "eval-1004", mean_score: 0.85, created_at: ago(284 * DAY), label: "haiku cost-tier check" },
  ];
}

// I2/I3 addition: one seeded `quality_drift` bus event (source "verdryx",
// schema v0.2 per docs/PHASE4.md's own grounded contract: "tokenfuse/qryx
// emit v0.1, wardryx/verdryx/mockryx v0.2") so `QualityDriftStream.tsx`'s
// Drift Alerts section AND the new Incident Center card both have a real
// "via verdryx" row to render in the mock preview - before this, the
// synthetic live event generator (`makeEvent`) never produced one at all,
// so this source was always empty under `pnpm dev:mock`. Mean/delta are
// consistent with `mockQualityBaselines`'s own "base-release" (mean 0.88):
// a drift down to 0.85 is exactly the -0.03 delta below. Appended (not
// prepended) wherever it is spliced into `recent_events` below, so it never
// outranks a genuinely fresher live-generated event as the bus's "newest"
// (which would wrongly read as a stale bus to the `bus_stale` zond).
function mockQualityDriftEvent(): UiEvent {
  return {
    id: 900_001,
    env: "live",
    ts: ago(6 * 60_000),
    source: "verdryx",
    type: "quality_drift",
    agent_id: DRIFT_DEMO_AGENT_ID,
    run_id: null,
    severity: "high",
    schema: "taipanbox.dev/agent-event/v0.2",
    on_behalf_of: [],
    data: {
      baseline_id: "base-release",
      window: "eval-1005",
      mean_score: 0.85,
      delta: -0.03,
      verdict: "regressed",
      baseline_n: 44,
      t_statistic: -2.31,
      ci_low: -0.081,
      ci_high: -0.009,
    },
    prev_hash: null,
    raw: "",
    file: "/root/.stack-up/events/verdryx.ndjson",
    off: 900_001,
  };
}

/** I11 "agent drift card" fixture: a second `quality_drift` event for the
 * SAME agent, verdict "on-track", so Agent 360's Drift section (assembled
 * client-side from these bus events - `components/Agent360.tsx`) has a real
 * example of both tones under `dev:mock`, not just the regression above.
 * Older than the regression (`ago(2 * DAY)` vs. the regression's own
 * `ago(6 * 60_000)`), so the honest reading is "was fine, later regressed" -
 * both check the SAME baseline (`base-release`). NOTE: `lib/incidents.ts`'s
 * own doc comment records that the REAL verdryx bus only ever fires
 * `quality_drift` on a regression (an "all clear" never gets an event at
 * all) - this on-track row exists ONLY in the mock preview, to exercise the
 * Drift section's other tone, using a value (`verdict: "on-track"`) the
 * wire contract already allows for even though production never emits it. */
function mockQualityOnTrackEvent(): UiEvent {
  return {
    id: 900_002,
    env: "live",
    ts: ago(2 * DAY),
    source: "verdryx",
    type: "quality_drift",
    agent_id: DRIFT_DEMO_AGENT_ID,
    run_id: null,
    severity: "low",
    schema: "taipanbox.dev/agent-event/v0.2",
    on_behalf_of: [],
    data: {
      baseline_id: "base-release",
      window: "eval-1003",
      mean_score: 0.875,
      delta: -0.005,
      verdict: "on-track",
      baseline_n: 44,
      t_statistic: -0.34,
      ci_low: -0.021,
      ci_high: 0.011,
    },
    prev_hash: null,
    raw: "",
    file: "/root/.stack-up/events/verdryx.ndjson",
    off: 900_002,
  };
}

/** A couple of deterministic Wardryx decisions for `id`, so Agent 360's Policy
 * section (which filters this agent's `agent_events` to `source === "wardryx"`)
 * always shows real decisions for a governed agent: every meridian.io agent is
 * under at least the org-wide `deny-shell-exec` policy, so a `policy_allow` is
 * honest for any of them, and an agent whose envelope needs a human sign-off
 * (the same `needHuman` set `mockApprovals` uses) additionally shows a
 * `policy_hold`. Timestamps a few minutes back so they read as the run's recent
 * history, not "now"; ids in a fixed 90_00x band, distinct from `makeEvent`'s
 * 100_000+ live sequence and the 900_00x quality fixtures. */
function mockAgentPolicyEvents(id: string): UiEvent[] {
  const a = FLEET.find((x) => x.id === id);
  if (!a) return [];
  const wardryxEvent = (seq: number, type: string, severity: string, agoMs: number, data: Record<string, unknown>): UiEvent => ({
    id: 90_000 + seq,
    env: "live",
    ts: ago(agoMs),
    source: "wardryx",
    type,
    agent_id: id,
    run_id: `${a.name}-live`,
    severity,
    schema: "taipanbox.dev/agent-event/v0.2",
    on_behalf_of: [userId(a.owner)],
    data,
    prev_hash: null,
    raw: "",
    file: "/root/.stack-up/events/wardryx.ndjson",
    off: 90_000 + seq,
  });
  const out: UiEvent[] = [
    wardryxEvent(1, "policy_allow", "low", 5 * 60_000, { decision: "allow", policy_id: "deny-shell-exec", reason: "no denied tool requested" }),
  ];
  if (a.allowed.some((x) => x.includes("human") || x.includes("approval"))) {
    out.unshift(wardryxEvent(2, "policy_hold", "medium", 3 * 60_000, { decision: "hold", reason: "estimated cost exceeds the human-approval threshold" }));
  }
  return out;
}

// --- Crypto (Qryx) ---
const F_ECDSA = { algorithm: "ECDSA-256", type: "public-key", severity: "high", occurrences: 14, locations: ["ingress/tls", "relay/acme-account.key", "cert-broker"], externallyFacing: true, longLivedData: false, planned: false };
const F_RSA = { algorithm: "RSA-2048", type: "certificate", severity: "high", occurrences: 6, locations: ["vault/pki", "legacy-lb/tls"], externallyFacing: true, longLivedData: true, planned: false };
const F_ED = { algorithm: "Ed25519", type: "public-key", severity: "medium", occurrences: 9, locations: ["ssh/authorized_keys", "ci/deploy-key"], externallyFacing: false, longLivedData: false, planned: false };
function mockNcsc(path: string) {
  return {
    standard: "NCSC PQC migration timeline",
    generatedAt: new Date().toISOString(),
    root: path || "/root",
    discovery2028: { verdict: "at-risk", coverageBySource: { source: 131, binary: 8, tls: 3 }, totalInventoried: 142, quantumVulnerableCount: 29, migrationPlanExists: false, migrationPlanNote: "no PQC migration plan on file for the externally-facing ECDSA/RSA assets", quantumVulnerableFindings: [F_ECDSA, F_RSA, F_ED] },
    highestPriority2031: { verdict: "not-started", criteria: "externally-facing with long-lived data", count: 2, migratedCount: 0, remainingCount: 2, note: "RSA-2048 protects long-lived data and is externally facing; migrate to ML-KEM/ML-DSA first", findings: [F_ECDSA, F_RSA] },
    fullMigration2035: { verdict: "not-started", count: 29, findings: [F_ECDSA, F_RSA, F_ED] },
  };
}
function cbomComp(name: string, primitive: string, param: string) {
  return { name, type: "crypto-asset", version: "", cryptoProperties: { assetType: "algorithm", algorithmProperties: { primitive, parameterSetIdentifier: param } } };
}
function mockCbom() {
  return {
    bomFormat: "CycloneDX",
    specVersion: "1.6",
    components: [
      cbomComp("ECDSA-256", "signature", "P-256"),
      cbomComp("RSA-2048", "signature", "2048"),
      cbomComp("Ed25519", "signature", "ed25519"),
      cbomComp("SHA-256", "hash", "sha-256"),
      cbomComp("AES-256-GCM", "ae", "256"),
      cbomComp("ML-DSA-65", "signature", "ML-DSA-65"),
      cbomComp("ML-KEM-768", "kem", "ML-KEM-768"),
    ],
  };
}
function mockCryptoEvidence(path: string) {
  return {
    tool: "qryx",
    version: "0.9.2",
    standard: "CNSA 2.0",
    generatedAt: new Date().toISOString(),
    root: path || "/root",
    summary: { compliant: 98, nonCompliant: 29, issues: 12, total: 127, scorePct: 77, bySeverity: { high: 8, medium: 14, low: 7 } },
    assets: [],
    digest: fakeSha("cnsa-evidence"),
    signature: null,
  };
}

// --- Drills (Mockryx) ---
function mockDrillReport() {
  const scenarios = ["runaway-budget", "dlp-secret-leak", "on-behalf-of-forged-chain", "wardryx-denied-tool", "approval-required"];
  return {
    run_id: `drill-${Math.floor(pseudo("drill" + eventSeq) * 1e6).toString(16)}`,
    gateway: "http://127.0.0.1:4100",
    generated_at: new Date().toISOString(),
    results: scenarios.map((s) => ({
      scenario: s,
      status: "passed",
      findings: [],
      skipped_findings: [],
      metrics: { calls: 1 + Math.floor(pseudo(s) * 3), budget_burned_usd: Number((pseudo(s + "b") * 0.006).toFixed(4)) },
    })),
  };
}

// --- Evidence ---
function mockEvidenceBuild() {
  return {
    zip_base64: "",
    filename: `evidence-${ORG}-${utcStamp(0).slice(0, 10)}.zip`,
    manifest: {
      pack_version: "1.0",
      generated_at: new Date().toISOString(),
      operator: `user://${ORG}/ops`,
      org: ORG,
      artifacts: [
        { name: "PQC crypto inventory", filename: "qryx-cbom.json", content_type: "application/json", source: "qryx", tool_version: "qryx 0.9.2", verify_status: "self-verified", sha256: fakeSha("cbom"), size_bytes: 48213 },
        { name: "CNSA 2.0 compliance", filename: "qryx-evidence.json", content_type: "application/json", source: "qryx", tool_version: "qryx 0.9.2", verify_status: "digest-ok", sha256: fakeSha("ev"), size_bytes: 12904 },
        { name: "Identity snapshot", filename: "idryx-identities.json", content_type: "application/json", source: "idryx", tool_version: "idryx 0.8.1", verify_status: null, sha256: fakeSha("idryx"), size_bytes: 20481 },
        { name: "Cloud audit chain", filename: "tokenfuse-audit.ndjson", content_type: "application/x-ndjson", source: "tokenfuse", tool_version: "cloud 0.4.0", verify_status: "chain-intact", sha256: fakeSha("audit"), size_bytes: 91002 },
      ],
      missing: [{ name: "Verdryx quality", reason: "no eval store wired into this environment's descriptor" }],
    },
    signed: false,
    cloud_included: true,
    journaled: true,
    journal_error: null,
  };
}

// --- Onboard (the "new agent" wizard, docs/ONBOARD.md), on-demand ---

/** `onboard_status`'s mock: a small, fixed set of already-provisioned
 * passports so the "Provisioned passports" table has something to show in
 * `dev:mock` (this command was left unmocked when `onboard_generate` was
 * added - without this arm the panel reads as permanently empty, since
 * `mockInvoke`'s `default:` case answers `null`). Ignores the request's own
 * `map_path`/`passports_dir` overrides and always answers the same handful
 * of rows - a fixture, not a simulation of the operator's own filesystem.
 * One passport declares zero filesystem scopes and two declare a few, so the
 * `filesystem_count` column reads as both "-" and "N folders" in the same
 * table; `models_count` varies independently of `filesystem_count` per row
 * (never the same number on the same row) so the two columns are visibly
 * distinct rather than looking like one value duplicated twice. */
function mockOnboardStatus() {
  return {
    map_path: "/root/.taipan/identity.json",
    map_loaded: true,
    map_error: null,
    units: [
      { id: "sre", name: "SRE", budget_usd_month: 4000 },
      { id: "platform", name: "Platform", budget_usd_month: 3000 },
    ],
    passports_dir: "/root/.taipan/passports",
    passports: [
      {
        agent_id: `agent://${ORG}/sre/rca-copilot`,
        owner: `user://${ORG}/j.carter`,
        file: "/root/.taipan/passports/sre-rca-copilot.json",
        filesystem_count: 2,
        models_count: 1,
        in_map: true,
      },
      {
        agent_id: `agent://${ORG}/platform/api-gateway-tuner`,
        owner: `user://${ORG}/t.osei`,
        file: "/root/.taipan/passports/platform-api-gateway-tuner.json",
        filesystem_count: 0,
        models_count: 0,
        in_map: true,
      },
      {
        agent_id: `agent://${ORG}/finops/idle-resource-sweeper`,
        owner: `user://${ORG}/n.foster`,
        file: "/root/.taipan/passports/finops-idle-resource-sweeper.json",
        filesystem_count: 1,
        models_count: 2,
        in_map: false,
      },
    ],
    skipped: [],
  };
}

/** A fixed, obviously-fake `gx_<32 hex>` - never a real secret, just a
 * plausible shape for the "shown once" client-key display. */
const MOCK_CLIENT_KEY_SECRET = "gx_0123456789abcdef0123456789abcdef";

/** One declared filesystem scope, mock-preview shape (mirrors `FsScope` in
 * `onboardTypes.ts`). */
interface MockFsScope {
  path: string;
  mode: string;
}

/** One declared model entry, mock-preview shape (mirrors `ModelDecl` in
 * `onboardTypes.ts`) - `model`/`endpoint` stay optional, same as the wire
 * shape, so an entry naming only a provider renders as bare `{ provider }`. */
interface MockModelDecl {
  provider: string;
  model?: string;
  endpoint?: string;
}

/** `onboard_generate`'s mock: builds the same four-artifact bundle shape the
 * real `crates/api/src/onboard/commands.rs::onboard_generate` returns, from
 * whatever the operator typed into the form. Echoes the request's own
 * `filesystem` rows and `models` entries when the operator declared any (so
 * add/remove/generate is faithful to test end to end), and otherwise falls
 * back to a couple of example entries for each - this is a showroom (see
 * this file's own header comment), and the whole point of extending this
 * mock was to make the feature visible without first wiring a live
 * backend. */
function mockOnboardGenerate(args?: Record<string, unknown>) {
  const req = (args?.request ?? {}) as Record<string, unknown>;
  const trustDomain = String(req.trust_domain || ORG);
  const path = String(req.path || "sre/demo-agent").replace(/^\/+|\/+$/g, "");
  const owner = String(req.owner || `user://${ORG}/j.carter`);
  const displayName = typeof req.display_name === "string" && req.display_name ? req.display_name : undefined;
  const runtime = typeof req.runtime === "string" && req.runtime ? req.runtime : undefined;
  const attestationMethod =
    typeof req.attestation_method === "string" && req.attestation_method && req.attestation_method !== "none"
      ? req.attestation_method
      : undefined;
  const keyId = typeof req.key_id === "string" && req.key_id ? req.key_id : path.replace(/\//g, "-");
  const agentId = `agent://${trustDomain}/${path}`;
  const bindPattern = typeof req.bind_pattern === "string" && req.bind_pattern ? req.bind_pattern : agentId;

  const requestedFilesystem = Array.isArray(req.filesystem) ? (req.filesystem as unknown[]) : [];
  const filesystem: MockFsScope[] =
    requestedFilesystem.length > 0
      ? requestedFilesystem.map((s) => {
          const scope = (s ?? {}) as Record<string, unknown>;
          return { path: String(scope.path ?? ""), mode: String(scope.mode ?? "read") };
        })
      : [
          { path: "/data/reports", mode: "read" },
          { path: "/data/out", mode: "write" },
        ];

  const requestedModels = Array.isArray(req.models) ? (req.models as unknown[]) : [];
  const models: MockModelDecl[] =
    requestedModels.length > 0
      ? requestedModels.map((m) => {
          const decl = (m ?? {}) as Record<string, unknown>;
          const model = typeof decl.model === "string" && decl.model ? decl.model : undefined;
          const endpoint = typeof decl.endpoint === "string" && decl.endpoint ? decl.endpoint : undefined;
          return { provider: String(decl.provider ?? ""), ...(model ? { model } : {}), ...(endpoint ? { endpoint } : {}) };
        })
      : [
          { provider: "anthropic", model: "claude-sonnet-4-5", endpoint: "api.anthropic.com" },
          { provider: "openai" },
        ];

  const passport: Record<string, unknown> = {
    schema: "taipanbox.dev/agent-passport/v0.1",
    id: agentId,
    owner,
    ...(displayName ? { display_name: displayName } : {}),
    ...(runtime ? { runtime } : {}),
    ...(attestationMethod ? { attestation: { method: attestationMethod } } : {}),
    ...(filesystem.length > 0 ? { filesystem } : {}),
    ...(models.length > 0 ? { models } : {}),
    created_at: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
  };

  const wardryxLines = [
    "# Wardryx policy stub generated by the Genaryx onboard wizard (docs/ONBOARD.md).",
    "# Review, adjust, and commit it next to your other policies.",
    `name: onboard-${keyId}`,
    `target: "${bindPattern}"`,
  ];
  if (filesystem.length > 0) {
    wardryxLines.push("# filesystem scopes declared on the passport (informational):");
    for (const s of filesystem) {
      wardryxLines.push(`#   ${`${s.mode}:`.padEnd(6, " ")} ${s.path}`);
    }
    wardryxLines.push(
      "# NOTE: wardryx does not enforce filesystem paths in v1 (its policy",
      "# surface is deny_tool / allow_domains / require_human_above_usd /",
      "# deny_above_usd / max_steps / deny_if_unattested). These lines are a",
      "# declaration carried on the passport, not an enforced control.",
    );
  }

  const tfName = keyId.replace(/[-.]/g, "_");
  const tfLines = [
    "# Generated by the Genaryx onboard wizard (docs/ONBOARD.md). Review and commit.",
    `resource "taipan_agent_passport" "${tfName}" {`,
    `  id    = "${agentId}"`,
    `  owner = "${owner}"`,
  ];
  if (displayName) tfLines.push(`  display_name = "${displayName}"`);
  if (runtime) tfLines.push(`  runtime = "${runtime}"`);
  if (attestationMethod) tfLines.push(`  attestation_method = "${attestationMethod}"`);
  for (const s of filesystem) {
    tfLines.push("  filesystem {", `    path = "${s.path}"`, `    mode = "${s.mode}"`, "  }");
  }
  for (const m of models) {
    tfLines.push("  models {", `    provider = "${m.provider}"`);
    if (m.model) tfLines.push(`    model = "${m.model}"`);
    if (m.endpoint) tfLines.push(`    endpoint = "${m.endpoint}"`);
    tfLines.push("  }");
  }
  tfLines.push("}", "", `resource "taipan_wardryx_policy" "${tfName}" {`);
  tfLines.push(`  id     = "onboard-${keyId}"`, `  target = "${bindPattern}"`, "}");

  return {
    agent_id: agentId,
    passport_json: `${JSON.stringify(passport, null, 2)}\n`,
    passport_path: `/root/.taipan/passports/${path.replace(/\//g, "-")}.json`,
    client_key_secret: MOCK_CLIENT_KEY_SECRET,
    client_keys_line: `${MOCK_CLIENT_KEY_SECRET}:${keyId}`,
    key_id: keyId,
    identity_map_fragment: `${JSON.stringify(
      { keys: [{ key_id: keyId, unit: String(req.unit || "sre"), agents: [bindPattern], created: new Date().toISOString().slice(0, 10) }] },
      null,
      2,
    )}\n`,
    unit_is_new: false,
    wardryx_policy_stub: `${wardryxLines.join("\n")}\n`,
    terraform_snippet: `${tfLines.join("\n")}\n`,
  };
}

// --- Felyx (copilot) canned answers, keyed loosely by the question ---
function mockCopilotAnswer(question: string) {
  const q = question.toLowerCase();
  // I10 "Felyx optimization recommendations" - checked BEFORE the generic
  // cost/spend branch below, since an optimization question ("how can I
  // reduce cost") would otherwise also match that looser regex. Mirrors the
  // real `savings_breakdown`/`cost_per_action` tools' shape
  // (`crates/copilot/src/tools/optimize.rs`) so this mock is a faithful
  // stand-in, not just a plausible-looking guess.
  if (/optimi[sz]e|optimization|savings breakdown|cost per (tool call|action)|reduce (my |the )?(cost|spend)/.test(q)) {
    return {
      text:
        "From the local TokenFuse trace: budget protection blocked $38.90 of runaway spend across 3 budget breaks, the semantic cache served $4.10 for free, and the model router saved $2.25 by downgrading eligible calls. By model, claude-opus-4-5 runs about $0.62 per tool call across 640 calls, versus $0.004 for claude-haiku - opus is spending a lot for comparatively little tool-calling work. By agent, finops/unit-economics-analyst accounts for most of that opus cost. I can't turn on more caching or re-route models myself (the console has no such control, and cache/router tuning is gateway config I cannot touch) - but I can propose tightening that agent's budget so the pattern is bounded.",
      tool_trace: [
        { name: "savings_breakdown", ok: true, result_preview: "blocked $38.90, cache $4.10, router $2.25, 3 budget breaks" },
        { name: "cost_per_action", ok: true, result_preview: "claude-opus-4-5 $0.62/tool-call (640 calls); claude-haiku $0.004/tool-call" },
      ],
      proposals: [
        { kind: "budget", target: "unit-economics-analyst-live", params: { usd_cap: 60 }, rationale: "Opus cost per tool call here is far above the fleet average and this agent has no cap today; $60/day bounds it without blocking its weekly unit-cost run.", confidence: 0.68, evidence_refs: ["cost_per_action:claude-opus-4-5"], policy_context: ["finops-spend-cap"] },
      ],
      usage: { prompt_tokens: 850 + question.length * 3, completion_tokens: 175 },
    };
  }
  if (/runaway|expensive|cost|most|spend/.test(q)) {
    return {
      text:
        "The caught runaway is sre/rca-copilot: it looped on an oversized incident trace, burned past its $1.25 per-run ceiling 26 times across shards, and tripped budget_exhausted and fanout_explosion. It was already killed break-glass by sre-oncall; its all-time spend is $41.60. The top LEGITIMATE spender is finops/unit-economics-analyst at $77.46 (Opus, modelling unit cost), inside its budget at 79% utilisation.",
      tool_trace: [
        { name: "money_incidents", ok: true, result_preview: "7 open; worst fanout_explosion x12 on rca-copilot" },
        { name: "list_runs", ok: true, result_preview: "42 runs, 1 killed, top spend $77.46" },
      ],
      proposals: [
        { kind: "budget", target: "unit-economics-analyst-live", params: { usd_cap: 60 }, rationale: "The top legitimate spender has no central cap; $60/day bounds it without blocking its weekly unit-cost run.", confidence: 0.72, evidence_refs: ["unit-economics-analyst-live"], policy_context: ["finops-spend-cap"] },
      ],
      usage: { prompt_tokens: 900 + question.length * 3, completion_tokens: 190 },
    };
  }
  if (/approval|pending|waiting|sign/.test(q)) {
    return {
      text:
        "Six approvals are awaiting a human decision, all from agents whose policy requires sign-off above a cost threshold: sre/runbook-executor, sre/deploy-guard, finops/commitment-planner, finops/idle-resource-sweeper and platform/api-gateway-tuner. The oldest has waited about nine minutes. None of them can act until a human grants them.",
      tool_trace: [{ name: "list_approvals", ok: true, result_preview: "6 pending, oldest ~9m" }],
      proposals: [],
      usage: { prompt_tokens: 700 + question.length * 3, completion_tokens: 120 },
    };
  }
  if (/who|what.*you|can you|help|do you/.test(q)) {
    return {
      text:
        "I read your fleet and recommend, I never act. I can name the most expensive runs, explain an incident, summarise pending approvals, or flag agents drifting toward their budget. Anything I suggest still needs a human to approve and sign it: I hold no signing key, so I cannot press a button myself.",
      tool_trace: [],
      proposals: [],
      usage: { prompt_tokens: 620 + question.length * 3, completion_tokens: 110 },
    };
  }
  return {
    text:
      "Across meridian.io I see 42 agents in four units (SRE, Platform, FinOps, Data Platform), $495 governed spend this window, $38.90 prevented by the budget breaker, and 7 open incidents. Ask me about the runaway, spend by unit, or the pending approvals.",
    tool_trace: [{ name: "money_overview", ok: true, result_preview: "spent $495, saved $38.90, 7 incidents" }],
    proposals: [],
    usage: { prompt_tokens: 640 + question.length * 3, completion_tokens: 130 },
  };
}

// I2 addition: `copilot_explain` (C1's "Explain with Felyx" - the Money
// panel's own existing Incidents feed already calls this, the new Incident
// Center card reuses the exact same wiring) had no mock case at all before
// this - `mockInvoke`'s default fell through to `return r(null)`, and
// `CopilotView.tsx`'s explain-request effect unconditionally reads
// `answer.text`, so every "Explain" click crashed the whole view under
// `pnpm dev:mock` (a real genaryx-web backend was never affected -
// `crates/api/src/copilot/commands.rs::copilot_explain` always returns a
// real `Answer`). Fixed here as a genuine mock-fidelity gap, not a new
// feature: same canned-answer shape `mockCopilotAnswer` above already uses,
// grounded in the SAME root-cause chain docs/PHASE6-C1.md's prompt asks
// Felyx to build (cause -> effect -> effect, citing the run/incident/policy
// ids it "used").
function mockCopilotExplainAnswer(incidentId: string) {
  const ra = PROTAGONIST;
  return {
    text:
      `Incident \`${incidentId}\` traces to sre/rca-copilot: an oversized incident trace caused retries past its $1.25 ` +
      "per-run ceiling 26 times across shards, tripping budget_exhausted then fanout_explosion. Root-cause chain: " +
      "oversized trace -> repeated over-budget retries -> fanout across shards. It was already killed break-glass by " +
      "sre-oncall; the only governing Wardryx policy on this agent (rca-max-steps) caps steps, not spend, which is why " +
      "nothing blocked it in advance. Recommended: add a Wardryx deny-above-usd policy for this agent so a future run " +
      "halts before 26 retries, not after.",
    tool_trace: [
      { name: "incidents", ok: true, result_preview: `resolved ${incidentId || "(no id)"}` },
      { name: "list_runs", ok: true, result_preview: `${ra.name}-live: 12x fanout_explosion, killed` },
      { name: "identity_alerts", ok: true, result_preview: "runaway_agent, excessive_agency on rca-copilot" },
      { name: "policies", ok: true, result_preview: "rca-max-steps (max_steps=12) - no spend cap on this agent" },
    ],
    proposals: [],
    usage: { prompt_tokens: 780, completion_tokens: 165 },
  };
}

// ---------------------------------------------------------------------------
// Live event stream.
// ---------------------------------------------------------------------------

let eventSeq = 100_000;

/**
 * One synthetic bus event, phase-aware instead of purely random: reads the
 * live world (via `reconcileProtagonist`, when the scenario is "incident")
 * to decide whether THIS tick is one of the protagonist's own crisis events,
 * and otherwise draws an ordinary background event exactly as before. Pure -
 * no mutation - so it is equally safe called once per live bus tick
 * (`mockSubscribe`) or in a tight loop for a historical batch (`seedEvents`).
 */
function makeEvent(): UiEvent {
  eventSeq += 1;
  const state = currentScenario === "incident" ? reconcileProtagonist() : null;
  const spotlight =
    !!state &&
    !state.killed &&
    (state.phase === "tripped" ? Math.random() < 0.6 : state.phase === "climbing" && Math.random() < 0.28);

  const a = spotlight ? PROTAGONIST : FLEET[Math.floor(Math.random() * FLEET.length)];
  const id = agentId(a);
  const base = {
    id: eventSeq,
    env: "live",
    ts: new Date().toISOString(),
    agent_id: id,
    on_behalf_of: [userId(a.owner)],
    schema: "taipanbox.dev/agent-event/v0.1",
    prev_hash: null,
    file: "/root/.stack-up/events/tokenfuse.ndjson",
    off: eventSeq,
  };

  if (a.id === PROTAGONIST_ID && state && state.phase === "tripped" && !state.killed) {
    const roll = Math.random();
    const budgetUsd = manualRunBudgets.get(PROTAGONIST_RUN_ID) ?? PROTAGONIST_RUN_BUDGET_USD;
    const spentUsd = Number((state.fraction * PROTAGONIST_RUN_BUDGET_USD).toFixed(4));
    if (roll < 0.45) {
      return { ...base, source: "tokenfuse", type: "budget_exceeded", severity: "critical", run_id: PROTAGONIST_RUN_ID, data: { reason: "budget_exceeded", budget_usd: budgetUsd, spent_usd: spentUsd, detail: "per-run budget exceeded", policy_id: "default" }, raw: "" };
    }
    if (roll < 0.8) {
      return { ...base, source: "tokenfuse", type: "breaker_tripped", severity: "critical", run_id: PROTAGONIST_RUN_ID, data: { reason: "budget_exceeded", budget_usd: budgetUsd, spent_usd: spentUsd, detail: "circuit breaker tripped after repeated retries", policy_id: "default" }, raw: "" };
    }
    return { ...base, source: "wardryx", type: "policy_hold", severity: "high", run_id: PROTAGONIST_RUN_ID, data: { decision: "hold", reason: "fanout beyond max sub-runs; holding for review" }, raw: "" };
  }
  if (a.id === PROTAGONIST_ID && state && state.phase === "climbing" && Math.random() < 0.35) {
    return { ...base, source: "wardryx", type: "policy_hold", severity: "medium", run_id: PROTAGONIST_RUN_ID, data: { decision: "hold", reason: "estimated cost approaching per-run ceiling" }, raw: "" };
  }

  const roll = Math.random();
  if (roll < 0.09) {
    return { ...base, source: "wardryx", type: "policy_hold", severity: "medium", run_id: `${a.name}-live`, data: { decision: "hold", reason: "estimated cost exceeds human-approval threshold" }, raw: "" };
  }
  if (roll < 0.19) {
    return { ...base, source: "tokenfuse", type: "cache_hit", severity: "info", run_id: `${a.name}-live`, data: { saved_usd: Number((Math.random() * 0.02).toFixed(4)) }, raw: "" };
  }
  return { ...base, source: "tokenfuse", type: "allow", severity: "low", run_id: `${a.name}-live`, data: { model: a.model, input_tokens: 3000 + Math.floor(Math.random() * 30000), output_tokens: 250 + Math.floor(Math.random() * 1800), cost_usd: Number((Math.random() * 0.06).toFixed(4)) }, raw: "" };
}

function seedEvents(n: number): UiEvent[] {
  const out: UiEvent[] = [];
  for (let i = 0; i < n; i++) out.push(makeEvent());
  return out.reverse();
}

// ---------------------------------------------------------------------------
// Command router.
// ---------------------------------------------------------------------------

const READY = (extra: Record<string, unknown>) => ({ state: "ready", source: { source: "taipan", name: "live" }, ...extra });

// ---- Remote (Distance) -----------------------------------------------------
// A seeded, PROVIDER-NEUTRAL environment so the Remote tab screenshots as a
// configured console without pinning any one cloud. Addresses are RFC 5737
// documentation ranges (TEST-NET), never a real host: this preview never opens
// an actual tunnel or SSH session.
const SEED_REMOTE_ENV = {
  name: "prod-stack",
  wireguard_go_bin: "/opt/homebrew/bin/wireguard-go",
  wg_peer_public_key_hex: "a3f19c7d42b8e05f6c1d9a837e04b2f61d8c3a950b6e7f24c9a1d3b85f2e08c7",
  wg_endpoint: "198.51.100.42:51820",
  wg_allowed_ips: ["10.9.0.1/32"],
  wg_persistent_keepalive: 25,
  wg_listen_port: null,
  wg_local_ip: "10.9.0.2",
  wg_peer_ip: "10.9.0.1",
  ssh_host: "198.51.100.42",
  ssh_port: 22,
  ssh_user: "genaryx",
  ssh_identity_file: "/Users/you/.ssh/genaryx",
  ssh_pinned_host_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIH8qHJ1Le5x3mKQv2pRfT0aVn7cB4dW9sYzX1oU6bLmE prod-stack",
};
let remoteEnv: Record<string, unknown> | null = { ...SEED_REMOTE_ENV };
let remoteTunnel: Record<string, unknown> = { state: "disconnected" };
function remoteStatus() {
  return {
    state: "ready",
    default_wireguard_go_bin: "/opt/homebrew/bin/wireguard-go",
    environment: remoteEnv,
    console_public_b64: remoteTunnel.state === "disconnected" ? null : "3Jx5o0Zr8Qk2mP1sVn7cB4dW9sYzX1oU6bLmE0aVn7c=",
    tunnel: remoteTunnel,
    tail: null,
  };
}
// The one built-in, read-only connector. Example rows (managed-by=taipan) so
// the Hetzner provider option demonstrates fully; every field is example data.
function mockHetznerList() {
  return [
    { id: 51234567, name: "prod-stack", status: "running", ipv4: "198.51.100.42", server_type: "cpx41", cores: 8, memory_gb: 16, location: "fsn1", price_hourly_eur: 0.0512, labels: { "managed-by": "taipan" }, created: "2026-07-18T08:12:00Z" },
    { id: 51234568, name: "runtime-eu-1", status: "running", ipv4: "198.51.100.87", server_type: "cpx31", cores: 4, memory_gb: 8, location: "nbg1", price_hourly_eur: 0.027, labels: { "managed-by": "taipan" }, created: "2026-07-18T08:15:00Z" },
    { id: 51234569, name: "relay-1", status: "running", ipv4: "203.0.113.9", server_type: "cx22", cores: 2, memory_gb: 4, location: "hel1", price_hourly_eur: 0.008, labels: { "managed-by": "taipan" }, created: "2026-07-19T19:02:00Z" },
  ];
}
// "Connect this machine" (`remote_operator_wg_config`): one fixed example
// config, the same fake "prod-stack" demo box `SEED_REMOTE_ENV` above already
// uses (RFC 5737 documentation endpoint, never a real host, and a fake
// keypair this preview never uses to open a real tunnel). `qr_svg` is
// deliberately empty, mirroring `mockEvidenceBuild`'s own `zip_base64: ""`:
// a QR that scans would hand whoever pointed a phone at it a config for a
// tunnel that does not exist, so the card renders its own honest "no QR
// available" placeholder instead.
function mockOperatorWgConfig() {
  const clientPriv = "eA3mK9pLwQ2vXsZ0tYbNcRf8dGjHu5MoPq1Wn6CiT2E=";
  // The lowest free address, the same rule the real allocator follows, so
  // issuing twice in the preview does not hand out one address twice.
  const taken = new Set(
    mockWgPeers.map((p) => Number((p.allowed_ips[0] ?? "").split("/")[0].split(".")[3])),
  );
  let host = 2;
  while (taken.has(host) && host < 255) host += 1;
  const clientIp = `10.9.0.${host}`;
  const serverPub = "k7QmZ4RfWp2NxTqYs81cVbA0dEoJnHu5MdMv9wXpUYo=";
  const endpoint = "198.51.100.42:51820";
  // A distinct key per issue, and the peer really joins the list: an "Issue"
  // that leaves the device list unchanged would be the preview telling a lie
  // about the one thing this panel exists to show.
  const peerPublicKey = `Qp7VnK2mX9dLc4RtYw0sZbF6gJhU3oNeM1iA8u${String(host).padStart(2, "0")}=`;
  mockWgPeers = [
    ...mockWgPeers,
    {
      public_key: peerPublicKey,
      allowed_ips: [`${clientIp}/32`],
      last_handshake_unix: null as number | null,
      endpoint: null as string | null,
      rx_bytes: 0,
      tx_bytes: 0,
    },
  ];
  return {
    conf: `[Interface]\nPrivateKey = ${clientPriv}\nAddress = ${clientIp}/32\n\n[Peer]\nPublicKey = ${serverPub}\nEndpoint = ${endpoint}\nAllowedIPs = 10.9.0.1/32\nPersistentKeepalive = 25\n`,
    qr_svg: "",
    client_ip: clientIp,
    endpoint,
    server_public_key: serverPub,
    peer_public_key: peerPublicKey,
    console_tunnel_url: "http://10.9.0.1:7420",
  };
}

// The devices this preview pretends are authorized. Mutable so Revoke visibly
// removes a row: a revoke button that leaves the list unchanged reads as
// broken, and the point of showing this panel at all is that revocation is a
// thing you can see happen.
let mockWgPeers = [
  {
    public_key: "h+tkRs4b2x3oHJU36eBBSJRNKXJBwMcTHBZcPdHNuXw=",
    allowed_ips: ["10.9.0.2/32"],
    // Two deliberately different states: one device that has connected and one
    // that never has. They look identical unless the UI distinguishes them,
    // and "issued but never used" is the row an operator should look at twice.
    //
    // Relative to now, not a fixed instant: a hardcoded timestamp reads as
    // "last seen 181d ago" in a demo meant to look live, which is
    // indistinguishable from stale data.
    last_handshake_unix: Math.floor(Date.now() / 1000) - 12 * 60,
    endpoint: "203.0.113.7:54321",
    rx_bytes: 148_992,
    tx_bytes: 96_400,
  },
  {
    public_key: "POkY1/qUIGYK9twxY1oJzR1CrLrl6f5cCX29dwKZKW8=",
    allowed_ips: ["10.9.0.3/32"],
    last_handshake_unix: null as number | null,
    endpoint: null as string | null,
    rx_bytes: 0,
    tx_bytes: 0,
  },
];

function mockOperatorWgPeers() {
  return {
    iface: "wg-op",
    server_public_key: "6Qi/70+2yBMhGBHlbPb2+R+czYHSbXBfFmhmCfnC92E=",
    listen_port: 51820,
    backend: "uapi",
    peers: mockWgPeers,
  };
}

function mockOperatorWgRevoke(args: Record<string, unknown> | undefined) {
  const key = typeof args?.public_key === "string" ? args.public_key : "";
  const wasPresent = mockWgPeers.some((p) => p.public_key === key);
  mockWgPeers = mockWgPeers.filter((p) => p.public_key !== key);
  return { public_key: key, was_present: wasPresent, remaining_peers: mockWgPeers.length };
}
// Example CloudServer rows for the AWS/GCP/Azure/IBM Cloud live-listing
// (preview only; the real connector shells out to the operator's own
// aws/gcloud/az/ibmcloud CLI). RFC 5737 documentation IPs, never a real host.
function mockCloudList(provider: string) {
  if (provider === "ibmcloud") {
    return [
      { provider, id: "0717_e4b2a1c8-9d3f-4a56-8b12-3c4d5e6f7089", name: "prod-vsi-1", status: "running", public_ip: "203.0.113.44", private_ip: "10.240.0.6", server_type: "bx2-4x16", region: "us-south-1" },
      { provider, id: "0717_a9c8b7d6-5e4f-4321-9a8b-7c6d5e4f3210", name: "runtime-vsi-2", status: "running", public_ip: null, private_ip: "10.240.0.11", server_type: "bx2-2x8", region: "us-south-1" },
      { provider, id: "0717_5f4e3d2c-1b0a-4988-8776-6554433221ff", name: "relay-vsi-3", status: "stopped", public_ip: null, private_ip: "10.240.0.19", server_type: "cx2-2x4", region: "us-south-1" },
    ];
  }
  const region = provider === "aws" ? "eu-central-1" : provider === "azure" ? "westeurope" : "europe-west3-a";
  const t = (aws: string, azure: string, gcp: string) => (provider === "aws" ? aws : provider === "azure" ? azure : gcp);
  const id = (aws: string, azure: string, gcp: string) => (provider === "aws" ? aws : provider === "azure" ? azure : gcp);
  return [
    { provider, id: id("i-0a1b2c3d4e5f60718", "3f2504e0-4f89-11d3-9a0c-0305e82c3301", "6421509874532180001"), name: "prod-stack", status: "running", public_ip: "198.51.100.42", private_ip: "10.0.1.10", server_type: t("m6i.2xlarge", "Standard_D8s_v5", "e2-standard-8"), region },
    { provider, id: id("i-0b2c3d4e5f6071829", "8ab3c9d1-2e45-4c67-89ab-1cdef2345678", "6421509874532180002"), name: "runtime-eu-1", status: "running", public_ip: "198.51.100.87", private_ip: "10.0.1.11", server_type: t("m6i.xlarge", "Standard_D4s_v5", "e2-standard-4"), region },
    { provider, id: id("i-0c3d4e5f60718293a", "9bc4dae2-3f56-4d78-9abc-2def34567890", "6421509874532180003"), name: "relay-1", status: provider === "azure" ? "stopped" : "running", public_ip: provider === "azure" ? null : "203.0.113.9", private_ip: "10.0.1.12", server_type: t("t3.medium", "Standard_B2s", "e2-small"), region },
  ];
}

// ---- Credentials (I15 "key lifecycle health") -------------------------------
// One `GatewayKeysReport` fixture, five keys, each landing on a different
// `deriveKeyStatus` outcome (`lib/credentials.ts`): active, stale, never-used,
// dangling, and removed - plus 3 unauthorized attempts since startup. Together
// these light up the Credentials card, its "Key issues" KpiTile, the
// `key_hygiene` posture zond, and the Incident Center, all under `dev:mock`
// with no real gateway involved. `unbound`/`mismatching` are exercised by the
// derivation logic's own unit-shaped coverage (`crates/connectors/src/gateway.rs`'s
// tests, `lib/credentials.ts`'s doc comment) rather than a sixth/seventh row
// here - five is enough to show the table's full worst-first sort and every
// severity tone `CredentialsKeysTable` renders.
function mockCredentialsKeys() {
  return {
    strict_mode: "enforce",
    identity_map_configured: true,
    history_available: true,
    unauthorized_since_startup: { attempts: 3, last_millis: now - 6 * 60_000 },
    keys: [
      // active: configured, bound, called minutes ago, no mismatches.
      {
        key_id: "billing-agent",
        configured: true,
        bound: true,
        unit: "finance",
        agents: [`agent://${ORG}/finance/billing-agent`],
        created: dateStamp(120 * DAY),
        since_startup: { calls: 214, identity_mismatches: 0, last_seen_millis: now - 4 * 60_000 },
        history: { calls: 48_112, identity_mismatches: 0, first_seen_millis: now - 120 * DAY, last_seen_millis: now - 4 * 60_000 },
      },
      // stale: configured, bound, real history, but nothing seen in 19 days.
      {
        key_id: "legacy-etl-sync",
        configured: true,
        bound: true,
        unit: "data-platform",
        agents: [`agent://${ORG}/data-platform/legacy-etl-sync`],
        created: dateStamp(200 * DAY),
        since_startup: { calls: 0, identity_mismatches: 0, last_seen_millis: null },
        history: { calls: 9_340, identity_mismatches: 0, first_seen_millis: now - 200 * DAY, last_seen_millis: now - 19 * DAY },
      },
      // never-used: configured, bound, onboarded 2 days ago, zero calls ever.
      {
        key_id: "fraud-detector-v2",
        configured: true,
        bound: true,
        unit: "platform",
        agents: [`agent://${ORG}/platform/fraud-detector-v2`],
        created: dateStamp(2 * DAY),
        since_startup: { calls: 0, identity_mismatches: 0, last_seen_millis: null },
        history: null as { calls: number; identity_mismatches: number; first_seen_millis: number | null; last_seen_millis: number | null } | null,
      },
      // dangling: still bound in the identity map, secret no longer configured.
      {
        key_id: "sre-oncall-summarizer",
        configured: false,
        bound: true,
        unit: "sre",
        agents: [`agent://${ORG}/sre/oncall-summarizer`],
        created: dateStamp(160 * DAY),
        since_startup: { calls: 0, identity_mismatches: 0, last_seen_millis: null },
        history: { calls: 3_802, identity_mismatches: 0, first_seen_millis: now - 160 * DAY, last_seen_millis: now - 41 * DAY },
      },
      // removed: neither configured nor bound anymore - a ghost, history only.
      {
        key_id: "decommissioned-recon-batch",
        configured: false,
        bound: false,
        unit: null as string | null,
        agents: [] as string[],
        created: null as string | null,
        since_startup: { calls: 0, identity_mismatches: 0, last_seen_millis: null },
        history: { calls: 12_004, identity_mismatches: 0, first_seen_millis: now - 300 * DAY, last_seen_millis: now - 95 * DAY },
      },
    ],
  };
}

// ---- Admission (I6 "admission gate") ---------------------------------------
// Reuses `mockCredentialsKeys`'s SAME fixture report rather than a second,
// parallel one - the gateway's key-lifecycle report is the exact same
// object both planes read (`genaryx_api::admission`'s module doc: "the SAME
// gateway `credentials` reads"), so the preview should agree with itself.
// Typing `billing-agent` (bound, matches a real `agents` pattern) is the
// happy path; typing anything else is the "key unknown to gateway" negative
// - both fall out of one lookup, no separate fixture needed for either.
function mockAdmissionCheck(keyId: string, agentId: string) {
  const report = mockCredentialsKeys();
  const key = report.keys.find((k) => k.key_id === keyId) ?? null;
  // Docs/20 grammar: a literal, or a single trailing `*` (prefix match) -
  // mirrors `agent_bound_in_report`/`valid_pattern`/`pattern_matches` in
  // `crates/api/src/admission/commands.rs`.
  const inMap = report.keys.some((k) =>
    k.agents.some((p) => (p.endsWith("*") ? agentId.startsWith(p.slice(0, -1)) : p === agentId)),
  );
  return {
    key_id: keyId,
    agent_id: agentId,
    strict_mode: report.strict_mode,
    identity_map_configured: report.identity_map_configured,
    key,
    in_map: inMap,
  };
}

function mockAdmissionBaseline(agentId: string) {
  return {
    run_id: `admission-${Math.floor(pseudo("admission-run" + agentId) * 1e12).toString(16).slice(0, 12)}`,
    case_count: 12,
    mean_score: 0.91,
    total_cost_usd: 0.34,
    baseline_id_or_label: `admission-${agentId}`,
  };
}

// ---- Routines (I7b "Routines tab") -----------------------------------------
// Five routines, each landing on a DISTINCT status so `dev:mock` shows the
// full worst-first sort with no real stack-up install involved: qryx-trend
// errors, mockryx-drill finds a gap (its own nature - a "findings" drill),
// verdryx-drift is skipped (no baseline configured), focus-export is a clean
// ok streak, and idryx-detect's timer is installed but has never fired yet -
// a genuine "installed, never run" state, deliberately DISTINCT from
// mockryx-drill's "ran at least once, but never installed as a timer"
// (mockryx-drill is opt-in only per routines.sh's own README, never
// installed by a plain `routines.sh install`). `installed` below matches
// routines.sh's own DEFAULT_ROUTINES split exactly: the four daily routines
// are installed, the weekly drill is not.
const ROUTINES_DIR = "/root/.stack-up/routines";

function routinesRecord(
  routine: string,
  startedAgoMs: number,
  durationMs: number,
  status: string,
  extra: { exit_code?: number; reason?: string | null; artifact?: string | null; summary?: string | null } = {},
) {
  return {
    schema: "stackup.routine-run/v1",
    routine,
    started_at: ago(startedAgoMs),
    finished_at: ago(startedAgoMs - durationMs),
    exit_code: extra.exit_code ?? 0,
    status,
    reason: extra.reason ?? null,
    artifact: extra.artifact ?? null,
    summary: extra.summary ?? null,
  };
}

function mockRoutinesStatus() {
  return {
    routines_dir: ROUTINES_DIR,
    routines_dir_exists: true,
    routines: [
      {
        name: "focus-export",
        installed: true,
        latest: routinesRecord("focus-export", 14 * 3_600_000, 6_000, "ok", {
          artifact: "out/focus-20260723.csv",
          summary: "142 data row(s) exported to focus-20260723.csv",
        }),
        latest_error: null,
      },
      {
        name: "qryx-trend",
        installed: true,
        latest: routinesRecord("qryx-trend", 14 * 3_600_000 - 10 * 60_000, 9_000, "error", {
          exit_code: 2,
          reason: "qryx: trend: could not open evidence file: no such file or directory",
          summary: "qryx trend exited 2",
        }),
        latest_error: null,
      },
      {
        name: "verdryx-drift",
        installed: true,
        latest: routinesRecord("verdryx-drift", 14 * 3_600_000 - 20 * 60_000, 2_000, "skipped", {
          reason: "ROUTINE_VERDRYX_BASELINE is not set; set it in /root/.stack-up/routines/config",
        }),
        latest_error: null,
      },
      // Never run: timer installed, but hasn't fired yet - no
      // status/idryx-detect.json on disk at all, so `latest`/`latest_error`
      // are both null (never conflated with "not installed", see this
      // section's own header comment).
      { name: "idryx-detect", installed: true, latest: null, latest_error: null },
      {
        name: "mockryx-drill",
        installed: false,
        latest: routinesRecord("mockryx-drill", 3 * DAY, 41_000, "findings", {
          exit_code: 1,
          artifact: "out/drill-20260720T064700Z.json",
          summary: "2 gap(s) found across 6 scenarios",
        }),
        latest_error: null,
      },
    ],
  };
}

// A dozen-ish records across four of the five routines (idryx-detect is
// deliberately absent - it has never run, see `mockRoutinesStatus` above,
// so its history is genuinely empty rather than faked), newest-last per
// routine (matching the real on-disk `history.ndjson`'s own append order -
// `mockRoutinesHistory` reverses, exactly like the real backend does).
const ROUTINES_HISTORY_POOL = [
  routinesRecord("focus-export", 72 * 3_600_000 + 14 * 3_600_000, 5_000, "ok", {
    artifact: "out/focus-20260720.csv",
    summary: "149 data row(s) exported to focus-20260720.csv",
  }),
  routinesRecord("focus-export", 48 * 3_600_000 + 14 * 3_600_000, 5_000, "ok", {
    artifact: "out/focus-20260721.csv",
    summary: "151 data row(s) exported to focus-20260721.csv",
  }),
  routinesRecord("focus-export", 24 * 3_600_000 + 14 * 3_600_000, 5_000, "ok", {
    artifact: "out/focus-20260722.csv",
    summary: "138 data row(s) exported to focus-20260722.csv",
  }),
  routinesRecord("focus-export", 14 * 3_600_000, 6_000, "ok", {
    artifact: "out/focus-20260723.csv",
    summary: "142 data row(s) exported to focus-20260723.csv",
  }),
  routinesRecord("qryx-trend", 48 * 3_600_000 + 14 * 3_600_000 - 10 * 60_000, 11_000, "ok", {
    artifact: "out/qryx-evidence.jsonl",
    summary: "compliance score 0.93 (+0.00 vs previous)",
  }),
  routinesRecord("qryx-trend", 24 * 3_600_000 + 14 * 3_600_000 - 10 * 60_000, 12_000, "ok", {
    artifact: "out/qryx-evidence.jsonl",
    summary: "compliance score 0.94 (+0.01 vs previous)",
  }),
  routinesRecord("qryx-trend", 14 * 3_600_000 - 10 * 60_000, 9_000, "error", {
    exit_code: 2,
    reason: "qryx: trend: could not open evidence file: no such file or directory",
    summary: "qryx trend exited 2",
  }),
  routinesRecord("verdryx-drift", 48 * 3_600_000 + 14 * 3_600_000 - 20 * 60_000, 2_000, "skipped", {
    reason: "ROUTINE_VERDRYX_BASELINE is not set; set it in /root/.stack-up/routines/config",
  }),
  routinesRecord("verdryx-drift", 24 * 3_600_000 + 14 * 3_600_000 - 20 * 60_000, 2_000, "skipped", {
    reason: "ROUTINE_VERDRYX_BASELINE is not set; set it in /root/.stack-up/routines/config",
  }),
  routinesRecord("verdryx-drift", 14 * 3_600_000 - 20 * 60_000, 2_000, "skipped", {
    reason: "ROUTINE_VERDRYX_BASELINE is not set; set it in /root/.stack-up/routines/config",
  }),
  routinesRecord("mockryx-drill", 10 * DAY, 38_000, "ok", {
    artifact: "out/drill-20260713T064700Z.json",
    summary: "0 gap(s) found across 6 scenarios",
  }),
  routinesRecord("mockryx-drill", 3 * DAY, 41_000, "findings", {
    exit_code: 1,
    artifact: "out/drill-20260720T064700Z.json",
    summary: "2 gap(s) found across 6 scenarios",
  }),
];

/** `routines_history` - filters the shared pool to `routine` (when given),
 * newest first (mirrors the real backend reversing `history.ndjson`'s own
 * append order), capped at `limit`. `skipped_lines` is a fixed 2 regardless
 * of the routine filter - the real backend counts malformed lines across
 * the WHOLE file before filtering to one routine
 * (`crates/api/src/routines/commands.rs::assemble_history`), so this mock
 * mirrors that "file-wide, not per-routine" fact rather than hiding it. */
function mockRoutinesHistory(routine?: string, limit?: number) {
  const filtered = routine ? ROUTINES_HISTORY_POOL.filter((r) => r.routine === routine) : [...ROUTINES_HISTORY_POOL];
  const newestFirst = [...filtered].sort((a, b) => Date.parse(b.started_at) - Date.parse(a.started_at));
  return {
    records: newestFirst.slice(0, limit ?? 200),
    skipped_lines: 2,
    routines_dir: ROUTINES_DIR,
    history_file_exists: true,
  };
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export async function mockInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const r = (v: unknown) => v as T;
  switch (command) {
    case "money_status": return r(READY({ cloud_url: "http://127.0.0.1:8080", org_domain: ORG }));
    case "policy_status": return r(READY({ wardryx_url: "http://127.0.0.1:8090", org_domain: "live" }));
    case "identity_status": return r(READY({ idryx_url: "http://127.0.0.1:8081", rescan_available: true }));
    case "credentials_status": return r(READY({ gateway_url: "http://127.0.0.1:4100" }));
    case "admission_status": return r({
      gateway: READY({ gateway_url: "http://127.0.0.1:4100" }),
      verdryx_bin: "/root/.taipan/bin/verdryx",
      verdryx_bin_present: true,
      verdryx_db: { source: { source: "well_known" }, path: "/root/.taipan/verdryx.db" },
      drills_scenario_dir: "/root/.stack-up/repos/mockryx/scenarios",
    });
    case "quality_status": return r(READY({ db_path: "/root/.taipan/verdryx.db" }));
    case "memory_status": return r(READY({ db_path: "/root/.taipan/engram.engram", engram_mcp_bin: "/root/.taipan/bin/engram-mcp" }));
    case "crypto_status": return r({ state: "ready", default_target: "/root", qryx_bin: "/root/.taipan/bin/qryx" });
    case "drills_status": return r(READY({ gateway_url: "http://127.0.0.1:4100", has_api_key: true, mockryx_bin: "/root/.taipan/bin/mockryx", scenario_dir: "/root/.stack-up/repos/mockryx/scenarios" }));
    case "evidence_status": return r({ state: "ready", qryx_available: true, qryx_bin: "/root/.taipan/bin/qryx", qryx_default_target: "/root", idryx_available: true, idryx_bin: "/root/.taipan/bin/idryx", idryx_load_sources: ["tokenfuse:/root/.stack-up/events/tokenfuse.ndjson"], tokenfuse_available: true, tokenfuse_bin: "/root/.taipan/bin/tokenfuse-cloud", tokenfuse_default_traces_dir: "/root/.stack-up/traces" });
    case "routines_status": return r(mockRoutinesStatus());
    case "routines_history": return r(mockRoutinesHistory(args?.routine as string | undefined, args?.limit as number | undefined));
    case "remote_status": return r(remoteStatus());
    case "remote_hetzner_list": return r(mockHetznerList());
    case "remote_cloud_list": return r(mockCloudList(String(args?.provider ?? "aws")));
    case "remote_operator_wg_config": return r(mockOperatorWgConfig());
    case "remote_operator_wg_peers": return r(mockOperatorWgPeers());
    case "remote_operator_wg_revoke": return r(mockOperatorWgRevoke(args));
    case "remote_set_environment": {
      const req = (args?.request ?? null) as Record<string, unknown> | null;
      if (req) remoteEnv = { ...req };
      remoteTunnel = { state: "disconnected" }; // saving resets any live tunnel, mirrors the real backend
      return r(remoteStatus());
    }
    case "remote_wg_connect": {
      remoteTunnel = { state: "failed", message: "this local preview cannot open a real WireGuard tunnel - connect from a packaged build against your own box" };
      return r(remoteStatus());
    }
    case "remote_wg_disconnect": { remoteTunnel = { state: "disconnected" }; return r(remoteStatus()); }
    case "remote_ssh_tail_start":
    case "remote_ssh_tail_stop": return r(remoteStatus());
    case "remote_ssh_check_reachable": return r(undefined);
    case "remote_ssh_read_file": return r({ content: "# preview: remote file reads run against your own box, not this local demo\n", valid_utf8: true, size_bytes: 74 });
    case "copilot_status": return r(copilotStatus());
    case "copilot_connect": {
      copilotConfig = {
        provider: String(args?.provider ?? "ollama"),
        model: String(args?.model ?? "unknown"),
        local: Boolean(args?.local),
      };
      return r({ ok: true });
    }
    case "bus_status": return r({ kind: "demo", dir: "/preview" });
    // The preview's own feeder emits conforming lines only, and the console's
    // seeding test asserts that, so the honest preview answer is zero refused.
    // Shown rather than omitted: the calm line is how a reader learns the check
    // runs at all, and an absent strip would read as an absent check.
    case "bus_quarantine": return r({
      measured: true,
      note:
        "Every line this bus has read conformed to the envelope. A producer that starts " +
        "emitting a broken one will appear here, and its agents would otherwise just look quiet.",
      total: 0,
      reasons: [],
    });

    case "money_overview": return r(mockOverview());
    case "money_runs": return r(mockRuns());
    case "money_incidents": return r(mockIncidents());
    case "money_savings": return r(mockSavings());
    case "money_kill_run": {
      const runId = String(args?.run_id ?? "");
      const reason = String(args?.reason ?? "");
      // The protagonist's OWN incident arc keeps its looping resolution (its
      // kill feeds `reconcileProtagonist`, and the arc re-arms after a beat -
      // that loop is the storyline, not a self-heal). EVERY other run's kill
      // goes into the STICKY `killedRuns` set instead of the old 20s-recovery
      // `manualRunKills`, so an operator kill persists visibly (reflected as
      // KILLED everywhere the agent appears) rather than looking like it did
      // nothing.
      const isProtagonistArc = runId === PROTAGONIST_RUN_ID && currentScenario === "incident";
      const already = isProtagonistArc ? manualRunKills.has(runId) : killedRuns.has(runId);
      if (runId) {
        if (isProtagonistArc) {
          if (!already) manualRunKills.set(runId, Date.now());
          reconcileProtagonist();
        } else {
          killedRuns.add(runId);
        }
      }
      emitConsoleCommand("console.kill_run", runId || "(unknown run)", "break_glass", "killed:true", `user://${ORG}/ops`);
      return r({
        summary: already ? `${runId || "run"} was already killed` : `Killed ${runId || "run"}${reason ? ` - ${reason}` : ""}`,
        http_status: 200,
        verify_result: "killed:true",
        sig_alg: "es256",
        sig_fpr: "software-signed",
        bus_recorded: true,
        bus_error: null,
      });
    }
    case "money_set_budget": {
      const runId = String(args?.run_id ?? "");
      const budgetUsd = Number(args?.budget_usd ?? 0);
      const reason = String(args?.reason ?? "");
      if (runId && budgetUsd > 0) manualRunBudgets.set(runId, budgetUsd);
      emitConsoleCommand("console.set_budget", runId || "(unknown run)", "break_glass", `budget_usd:${budgetUsd}`, `user://${ORG}/ops`);
      return r({
        summary: `Budget for ${runId || "run"} set to $${budgetUsd.toFixed(2)}${reason ? ` - ${reason}` : ""}`,
        http_status: 200,
        verify_result: `budget_usd:${budgetUsd}`,
        sig_alg: "es256",
        sig_fpr: "software-signed",
        bus_recorded: true,
        bus_error: null,
      });
    }
    case "money_ack_incident": {
      const id = String(args?.id ?? "");
      const already = ackedIncidentIds.has(id);
      if (id) ackedIncidentIds.add(id);
      emitConsoleCommand("console.ack_incident", id || "(unknown incident)", "allow", "acknowledged:true", `user://${ORG}/ops`);
      return r({
        summary: already ? `${id || "incident"} already acknowledged` : `Acknowledged ${id || "incident"}`,
        http_status: 200,
        verify_result: "acknowledged:true",
        sig_alg: "es256",
        sig_fpr: "software-signed",
        bus_recorded: true,
        bus_error: null,
      });
    }

    case "policy_list_approvals": return r(mockApprovals());
    case "policy_list_policies": return r(mockPolicies());
    // The preview's rules are file-loaded, exactly like a real seeded box: they
    // are enforced and do NOT appear in policy_list_policies. Reporting them
    // here is what keeps the posture panel from claiming a fail-open that the
    // rest of this preview contradicts.
    case "policy_enforcement_status": return r({
      policy_version: "3eba33b697e4",
      base_policies: mockPolicies().length,
      store_policies: 0,
      effective_policies: mockPolicies().length,
    });
    case "policy_decide_approval": {
      const id = String(args?.id ?? "");
      const decision: "grant" | "deny" = args?.decision === "grant" ? "grant" : "deny";
      emitConsoleCommand("console.decide_approval", id || "(unknown approval)", "allow", `decision:${decision}`, `user://${ORG}/ops`);
      return r(decideApprovalMock(id, decision));
    }

    case "identity_list_identities": return r(mockIdentities());
    case "identity_list_alerts": return r(mockAlerts());
    case "identity_list_remediations": return r([]);
    case "credentials_keys": return r(mockCredentialsKeys());
    case "admission_check": return r(mockAdmissionCheck(String(args?.key_id ?? ""), String(args?.agent_id ?? "")));
    case "admission_baseline": return r(mockAdmissionBaseline(String(args?.agent_id ?? "")));

    case "agent_graph": return r(mockGraph());
    case "agent_record": return r(mockAgentRecord(String(args?.agent_id ?? "")));
    case "user_record": return r(mockUserRecord(String(args?.user ?? args?.handle ?? "")));
    case "unit_record": return r(mockUnitRecord(String(args?.team ?? "")));
    case "agent_slice": return r(mockSlice(String(args?.agent_id ?? "")));
    case "org_directory": return r(orgDirectory());
    case "agent_set_budget": {
      const a = findById(String(args?.agent_id ?? ""));
      if (a) { a.budgetUsd = Number(args?.budget_usd) || a.budgetUsd; logChange(a, "budget_set", `per-run ceiling set to $${a.budgetUsd.toFixed(2)}`); }
      // Return the reflected record (`mockAgentRecord`) so the card keeps its
      // live `blocked`/`lifecycle` after an unrelated edit, rather than a bare
      // fixture spread whose `blocked` is no longer maintained on the fixture.
      return r(a ? mockAgentRecord(a.id) : null);
    }
    case "agent_reassign_unit": {
      const a = findById(String(args?.agent_id ?? ""));
      const team = String(args?.team ?? "");
      if (a && team && team !== a.team) {
        const from = a.team;
        const nowTs = new Date().toISOString();
        const open = a.segments.find((s) => s.to === null);
        if (open) open.to = nowTs;
        a.segments.push({ owner: a.owner, team, spentUsd: 0, from: nowTs, to: null });
        a.team = team;
        logChange(a, "transferred", `business unit ${from} -> ${team}`);
      }
      return r(a ? mockAgentRecord(a.id) : null);
    }
    case "agent_transfer_owner": {
      const a = findById(String(args?.agent_id ?? ""));
      const owner = String(args?.owner ?? "");
      if (a && owner && owner !== a.owner) {
        const from = a.owner;
        const nowTs = new Date().toISOString();
        const open = a.segments.find((s) => s.to === null);
        if (open) open.to = nowTs;
        a.segments.push({ owner, team: a.team, spentUsd: 0, from: nowTs, to: null });
        a.owner = owner;
        logChange(a, "transferred", `owner ${from} -> ${owner}`);
      }
      return r(a ? mockAgentRecord(a.id) : null);
    }
    case "agent_set_behaviour": {
      const a = findById(String(args?.agent_id ?? ""));
      const allowed = Array.isArray(args?.allowed) ? (args?.allowed as string[]) : null;
      if (a && allowed) { a.allowed = allowed; logChange(a, "transferred", "allowed behaviour updated"); }
      return r(a ? mockAgentRecord(a.id) : null);
    }
    // Freeze <-> Unfreeze a single agent: an idempotent toggle writing the
    // manual store (never the fixture), so it can coexist with a unit/user stop
    // and un-freezing leaves those in force. Returns the reflected record.
    case "agent_block": {
      const a = findById(String(args?.agent_id ?? ""));
      const blocked = Boolean(args?.blocked);
      if (a) {
        if (blocked) frozenAgents.add(a.id); else frozenAgents.delete(a.id);
        logChange(a, blocked ? "closed" : "launched", blocked ? "frozen by operator" : "unfrozen by operator");
        emitConsoleCommand(blocked ? "console.freeze_agent" : "console.unfreeze_agent", a.id, "allow", `frozen:${blocked}`, `user://${ORG}/ops`);
      }
      return r(a ? mockAgentRecord(a.id) : null);
    }
    // Stop <-> Start every agent a user owns: one flag on the user, so it
    // reflects app-wide via `manualAgentState` without stamping each agent.
    case "user_block": {
      const handle = String(args?.user ?? "").replace(/^user:\/\/[^/]+\//, "");
      const blocked = Boolean(args?.blocked);
      if (handle) {
        if (blocked) stoppedUsers.add(handle); else stoppedUsers.delete(handle);
        FLEET.filter((a) => a.owner === handle).forEach((a) => logChange(a, blocked ? "closed" : "launched", blocked ? `stopped with owner ${handle}` : `started with owner ${handle}`));
        emitConsoleCommand(blocked ? "console.stop_user" : "console.start_user", `user://${ORG}/${handle}`, "allow", `stopped:${blocked}`, `user://${ORG}/ops`);
      }
      return r(mockUserRecord(handle));
    }
    // Stop <-> Start every agent in a business unit: same one-flag shape.
    case "unit_block": {
      const team = String(args?.team ?? "");
      const blocked = Boolean(args?.blocked);
      if (team) {
        if (blocked) stoppedUnits.add(team); else stoppedUnits.delete(team);
        FLEET.filter((a) => a.team === team).forEach((a) => logChange(a, blocked ? "closed" : "launched", blocked ? `stopped with unit ${team}` : `started with unit ${team}`));
        emitConsoleCommand(blocked ? "console.stop_unit" : "console.start_unit", team, "allow", `stopped:${blocked}`, `user://${ORG}/ops`);
      }
      return r(mockUnitRecord(team));
    }
    case "agent_events": {
      const id = String(args?.agent_id ?? "");
      const limit = Number(args?.limit ?? 50);
      const evts = seedEvents(limit).filter((e) => e.agent_id === id);
      const base = evts.length ? evts : seedEvents(limit).slice(0, 6).map((e) => ({ ...e, agent_id: id }));
      // I11 fixture: the SAME two quality_drift events `recent_events` below
      // seeds (see `mockQualityDriftEvent`/`mockQualityOnTrackEvent`'s own
      // doc comments) must also show up on THIS agent's own per-agent feed -
      // Agent 360's Drift section reads `agent_events`, not `recent_events`.
      const drift = id === DRIFT_DEMO_AGENT_ID ? [mockQualityDriftEvent(), mockQualityOnTrackEvent()] : [];
      // Appended (oldest) so the live `base` events stay at the newest-first
      // head, while this agent's own Wardryx decisions still populate the
      // Policy section's `source === "wardryx"` filter for a governed agent.
      return r([...drift, ...base, ...mockAgentPolicyEvents(id)]);
    }
    // The seeded quality_drift event is APPENDED after the freshly-generated
    // ones (see mockQualityDriftEvent's own doc comment for why order matters
    // here), so `res.events[0]` (newest-first) is still whichever real event
    // `seedEvents` itself produced most recently. `recentCommandEvents` (an
    // operator's own kill/budget/ack/decide, newest-first already) goes in
    // front of all of it: a just-issued console_command is genuinely the
    // newest thing on the bus.
    case "money_owners": return r(mockOwners());
    case "stats_counts": return r(mockStatsCounts(Number(args?.window_days ?? 0)));
    case "egress_recent": return r(mockEgress());
    case "recent_events": return r([...recentCommandEvents, ...seedEvents(Number(args?.limit ?? 60)), mockQualityDriftEvent()]);
    case "run_events": return r(seedEvents(20));

    case "memory_stats": return r({ counts: { episodic: 31, semantic: 21, procedural: 0 }, facts_total: 21, facts_active: 21, entities: 0, db_size_bytes: 1_724_416, db_path: "/root/.taipan/engram.engram", agent_id: null, reflections: 0, vector_index_size: 31, facts_superseded: 0 });

    // ---- quality (Verdryx) ----
    case "quality_list_run_summaries": return r(mockQualityRuns());
    case "quality_run_scores": return r(mockQualityScores(String(args?.run_id ?? "eval-1000")));
    case "quality_list_baselines": return r(mockQualityBaselines());

    // ---- crypto (Qryx), on-demand scans ----
    case "crypto_scan_ncsc": return r(mockNcsc(String(args?.path ?? "/root")));
    case "crypto_scan_cbom": return r(mockCbom());
    case "crypto_scan_evidence": return r(mockCryptoEvidence(String(args?.path ?? "/root")));
    case "crypto_verify_evidence": return r({ verified: true, message: "digest matches; unsigned bundle, so there is no signature to check" });

    // ---- drills (Mockryx), on-demand ----
    case "drills_run": return r(mockDrillReport());

    // ---- evidence, on-demand ----
    case "evidence_build": return r(mockEvidenceBuild());

    // ---- onboard (the "new agent" wizard), on-demand ----
    case "onboard_status": return r(mockOnboardStatus());
    case "onboard_generate": return r(mockOnboardGenerate(args));

    // ---- Felyx ----
    case "copilot_ask": return r(mockCopilotAnswer(String(args?.question ?? "")));
    case "copilot_explain": return r(mockCopilotExplainAnswer(String(args?.incident_id ?? "")));

    default:
      if (/_list_|_events|_incidents|_runs|scores|baselines|summaries|remediations/.test(command)) return r([]);
      return r(null);
  }
}

export function mockSubscribe<T>(event: string, onEvent: (payload: T) => void): Promise<() => void> {
  if (event !== "bus:event") return Promise.resolve(() => {});
  const tick = () => {
    // A just-issued operator command is delivered before the next synthetic
    // event, so a Kill/Approve/Budget click shows up on a live Bus tab
    // within one tick instead of waiting on a random draw to cover it.
    const queued = pendingLiveDelivery.shift();
    onEvent((queued ?? makeEvent()) as unknown as T);
  };
  const id = window.setInterval(tick, 1300 + Math.floor(Math.random() * 900));
  return Promise.resolve(() => window.clearInterval(id));
}
