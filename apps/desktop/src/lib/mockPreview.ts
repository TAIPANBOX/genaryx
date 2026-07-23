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
 */

import type { UiEvent } from "../types";

export const MOCK = import.meta.env.VITE_GENARYX_MOCK === "1";

const ORG = "meridian.io";
const DAY = 86_400_000;
const now = Date.now();
const ago = (ms: number) => new Date(now - ms).toISOString();

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
   * for cause). Blocking a user or a unit sets this on every one of their
   * agents at once. */
  blocked?: boolean;
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

function genHistory(a: { team: string; name: string; owner: string }): LifecycleEntry[] {
  const born = 8 * DAY + Math.floor(pseudo(a.name + "born") * 12 * DAY);
  const h: LifecycleEntry[] = [
    { ts: ago(born), kind: "launched", detail: `launched for ${a.team} work`, actor: a.owner },
    { ts: ago(born), kind: "owned", detail: `owned by ${a.team} / ${a.owner}`, actor: "system" },
  ];
  if (pseudo(a.name + "xfer") > 0.72) {
    h.push({ ts: ago(born / 2), kind: "transferred", detail: `reassigned within ${a.team}`, actor: a.owner });
  }
  return h;
}

function buildFleet(): FleetAgent[] {
  const out: FleetAgent[] = [];
  for (const u of UNITS) {
    u.kinds.forEach((k, i) => {
      const owner = u.users[i % u.users.length];
      const isRunaway = u.team === RUNAWAY_TEAM && k.suffix === RUNAWAY_NAME;
      const calls = isRunaway ? 1240 : 150 + Math.floor(pseudo(k.suffix + "c") * 2300);
      const per = PER_CALL_USD[k.model] ?? 0.01;
      const spentUsd = isRunaway
        ? 41.6
        : Number((calls * per * (0.55 + pseudo(k.suffix + "s") * 1.05)).toFixed(2));
      const budgetUsd = isRunaway ? 1.25 : Number((0.5 + pseudo(k.suffix + "b") * 3).toFixed(2));

      // Attribution: a few agents already carry a prior owner (or a prior unit),
      // so the spend split by ownership period is visible out of the box.
      const bornMs = 8 * DAY + Math.floor(pseudo(k.suffix + "born") * 12 * DAY);
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

export function mockAgentRecord(id: string): FleetAgent | null {
  return FLEET.find((a) => agentId(a) === id) ?? null;
}

function runawayAgent(): FleetAgent {
  return FLEET.find((a) => a.closed) ?? FLEET[0];
}

// ---------------------------------------------------------------------------
// Derived wire DTOs.
// ---------------------------------------------------------------------------

function runFor(a: FleetAgent) {
  const closed = Boolean(a.closed);
  const util = closed ? 1.18 : 0.4 + pseudo(a.name + "u") * 0.46;
  const budget = Number((a.spentUsd / util).toFixed(2));
  return {
    run_id: `${a.name}-live`,
    model: a.model,
    agent_id: agentId(a),
    spent_usd: a.spentUsd,
    budget_usd: budget,
    calls: a.calls,
    cache_hits: Math.round(a.calls * 0.12),
    steps: Math.min(a.calls, 40),
    last_seen: ago(Math.random() * 60_000),
    killed: closed || Boolean(a.blocked),
  };
}

function mockRuns() {
  return FLEET.map(runFor);
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
  return {
    blocked_spend_usd: 38.9,
    cache_saved_usd: 0,
    router_saved_usd: 0,
    budget_breaks: 61,
    total_saved_usd: 38.9,
  };
}

function mockIncidents() {
  const ra = runawayAgent();
  const rid = agentId(ra);
  const out = [
    { id: "spend_spike:", run_id: null as string | null, agent_id: null as string | null, kind: "spend_spike", severity: "high", first_seen: ago(9 * 60_000), last_seen: ago(60_000), occurrences: 5, acknowledged: false },
    { id: `fanout_explosion:${ra.name}`, run_id: `${ra.name}-live`, agent_id: rid, kind: "fanout_explosion", severity: "high", first_seen: ago(8 * 60_000), last_seen: ago(90_000), occurrences: 12, acknowledged: false },
    { id: "sustained_loop:query-cost-optimizer", run_id: "query-cost-optimizer-live", agent_id: `agent://${ORG}/data/query-cost-optimizer`, kind: "sustained_loop", severity: "medium", first_seen: ago(30 * 60_000), last_seen: ago(12 * 60_000), occurrences: 4, acknowledged: true },
  ];
  for (let i = 51; i >= 47; i--) {
    out.push({ id: `budget_exhausted:${ra.name}-${i}`, run_id: `${ra.name}-${i}`, agent_id: rid, kind: "budget_exhausted", severity: "high", first_seen: ago((60 - i) * 60_000), last_seen: ago((58 - i) * 60_000), occurrences: 2, acknowledged: false });
  }
  return out;
}

function mockGraph() {
  const nodes: { id: string; kind: "user" | "agent" | "other"; event_count: number; x: number; y: number }[] = [];
  const users = [...new Set(FLEET.map((a) => a.owner))];
  users.forEach((u, i) => {
    nodes.push({ id: userId(u), kind: "user", event_count: 0, x: 120, y: 70 + i * 66 });
  });
  // Agents in a tidy grid so labels never overlap, six per row.
  FLEET.forEach((a, k) => {
    const col = k % 6;
    const row = Math.floor(k / 6);
    nodes.push({ id: agentId(a), kind: "agent", event_count: a.calls, x: 440 + col * 200, y: 80 + row * 96 });
  });
  const edges = FLEET.map((a) => ({ from: userId(a.owner), to: agentId(a) }));
  return { nodes, edges, width: 1700, height: Math.max(1500, users.length * 66 + 120) };
}

function mockSlice(id: string) {
  const a = mockAgentRecord(id);
  if (!a) return { node: null, parents: [], children: [] };
  return {
    node: { id, kind: "agent" as const, event_count: a.calls, last_ts: ago(30_000) },
    parents: [{ id: userId(a.owner), kind: "user" as const, event_count: 0, last_ts: "" }],
    children: [],
  };
}

// Agents whose envelope requires a human sign-off produce the pending approvals.
function mockApprovals() {
  const needHuman = FLEET.filter((a) => a.allowed.some((x) => x.includes("human")));
  const pick = needHuman.slice(0, 6);
  return pick.map((a, i) => ({
    approval_id: `ap_${(pseudo(a.name + "ap") * 1e12).toString(16).slice(0, 10)}`,
    agent_id: agentId(a),
    run_id: `${a.name}-live`,
    requested_at: ago((1 + i) * 90_000),
    decided_at: null as string | null,
    decided_by: null as string | null,
    decision: null as string | null,
    pending: true,
    tool_names: a.allowed.filter((x) => x.includes("_read") || x.includes("run") || x.includes("apply")).slice(0, 2),
    est_cost_usd: Number((8 + pseudo(a.name + "c") * 40).toFixed(1)),
    reason: `estimated cost exceeds the ${a.team} human-approval threshold; approval required`,
    on_behalf_of: [userId(a.owner)],
    policy_version: "356f49daa246",
    org: ORG,
    model: a.model,
  }));
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
    pol({ id: "finops-spend-cap", name: "finops-spend-cap", target: `agent://${ORG}/finops/*`, deny_above_usd: 20 }),
    pol({ id: "data-pii-attestation", name: "data-pii-attestation", target: `agent://${ORG}/data/*`, deny_if_unattested: true }),
    pol({ id: "platform-secret-dlp", name: "platform-secret-dlp", target: `agent://${ORG}/platform/secret-scanner`, deny_tool: ["external_send"] }),
    pol({ id: "rca-max-steps", name: "rca-max-steps", target: `agent://${ORG}/sre/rca-copilot`, max_steps: 12 }),
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

function mockIdentities() {
  return FLEET.map((a) => ({
    id: agentId(a),
    type: "agent",
    privileged: a.team === "sre" || a.name.includes("secret") || a.name.includes("pii"),
    source: "tokenfuse",
    owner: userId(a.owner),
    created: utcStamp(15 * DAY),
    last_used: utcStamp(Math.random() * 60_000),
    runtime: a.model,
    on_behalf_of: [userId(a.owner)],
    permissions: [],
    remediation: null,
    rotation: null,
    events: a.calls,
    alerts: a.closed ? 2 : 0,
    team: a.team,
  }));
}

function mockAlerts() {
  const ra = runawayAgent();
  return [
    { detector: "runaway_agent", identity: agentId(ra), severity: "high", time: ago(18 * 60_000), summary: "runaway_agent: 26 budget_exceeded blocks across shards on rca-copilot" },
    { detector: "excessive_agency", identity: agentId(ra), severity: "medium", time: ago(40 * 60_000), summary: "excessive_agency: agent opened 22 sub-runs in one window" },
    { detector: "over_privileged_nhi", identity: `agent://${ORG}/platform/secret-scanner`, severity: "low", time: ago(3 * DAY), summary: "over_privileged_nhi: secret-scanner holds unused repo_write" },
    { detector: "attestation_missing", identity: `agent://${ORG}/data/pii-scanner`, severity: "medium", time: ago(2 * DAY), summary: "attestation_missing: attestation=none on a data agent under data-pii-attestation" },
  ];
}

// Owner and unit aggregates, so a card can navigate agent -> owner -> unit.
function entityAgentFor(a: FleetAgent, portion: number, current: boolean) {
  return {
    agentId: a.id,
    name: a.name,
    team: a.team,
    owner: a.owner,
    model: a.model,
    spentUsd: Number(portion.toFixed(2)),
    calls: a.calls,
    closed: Boolean(a.closed),
    blocked: Boolean(a.blocked),
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
    { id: "base-eom", eval_run_id: "eval-1000", mean_score: 0.92, created_at: ago(20 * DAY), label: "end-of-month gate" },
    { id: "base-release", eval_run_id: "eval-1002", mean_score: 0.88, created_at: ago(9 * DAY), label: "pre-release" },
    { id: "base-haiku", eval_run_id: "eval-1004", mean_score: 0.85, created_at: ago(4 * DAY), label: "haiku cost-tier check" },
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
    agent_id: `agent://${ORG}/data/data-quality-checker`,
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

// --- Felyx (copilot) canned answers, keyed loosely by the question ---
function mockCopilotAnswer(question: string) {
  const q = question.toLowerCase();
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
// `pnpm dev:mock` (a real Tauri/genaryx-web backend was never affected -
// `crates/api/src/copilot/commands.rs::copilot_explain` always returns a
// real `Answer`). Fixed here as a genuine mock-fidelity gap, not a new
// feature: same canned-answer shape `mockCopilotAnswer` above already uses,
// grounded in the SAME root-cause chain docs/PHASE6-C1.md's prompt asks
// Felyx to build (cause -> effect -> effect, citing the run/incident/policy
// ids it "used").
function mockCopilotExplainAnswer(incidentId: string) {
  const ra = runawayAgent();
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

function makeEvent(): UiEvent {
  const a = FLEET[Math.floor(Math.random() * FLEET.length)];
  const ra = runawayAgent();
  const id = agentId(a);
  const roll = Math.random();
  eventSeq += 1;
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
  if (a.name === ra.name && roll < 0.5) {
    return { ...base, source: "tokenfuse", type: "breaker_tripped", severity: "critical", run_id: `${a.name}-${47 + Math.floor(Math.random() * 6)}`, data: { reason: "budget_exceeded", budget_usd: a.budgetUsd, spent_usd: Number((a.budgetUsd * 0.97).toFixed(4)), detail: "per-run budget exceeded", policy_id: "default" }, raw: "" };
  }
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
// Example CloudServer rows for the AWS/GCP/Azure live-listing (preview only;
// the real connector shells out to the operator's own aws/gcloud/az CLI). RFC
// 5737 documentation IPs, never a real host.
function mockCloudList(provider: string) {
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

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export async function mockInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const r = (v: unknown) => v as T;
  switch (command) {
    case "money_status": return r(READY({ cloud_url: "http://127.0.0.1:8080", org_domain: ORG }));
    case "policy_status": return r(READY({ wardryx_url: "http://127.0.0.1:8090", org_domain: "live" }));
    case "identity_status": return r(READY({ idryx_url: "http://127.0.0.1:8081", rescan_available: true }));
    case "credentials_status": return r(READY({ gateway_url: "http://127.0.0.1:4100" }));
    case "quality_status": return r(READY({ db_path: "/root/.taipan/verdryx.db" }));
    case "memory_status": return r(READY({ db_path: "/root/.taipan/engram.engram", engram_mcp_bin: "/root/.taipan/bin/engram-mcp" }));
    case "crypto_status": return r({ state: "ready", default_target: "/root", qryx_bin: "/root/.taipan/bin/qryx" });
    case "drills_status": return r(READY({ gateway_url: "http://127.0.0.1:4100", has_api_key: true, mockryx_bin: "/root/.taipan/bin/mockryx", scenario_dir: "/root/.stack-up/repos/mockryx/scenarios" }));
    case "evidence_status": return r({ state: "ready", qryx_available: true, qryx_bin: "/root/.taipan/bin/qryx", qryx_default_target: "/root", idryx_available: true, idryx_bin: "/root/.taipan/bin/idryx", idryx_load_sources: ["tokenfuse:/root/.stack-up/events/tokenfuse.ndjson"], tokenfuse_available: true, tokenfuse_bin: "/root/.taipan/bin/tokenfuse-cloud", tokenfuse_default_traces_dir: "/root/.stack-up/traces" });
    case "remote_status": return r(remoteStatus());
    case "remote_hetzner_list": return r(mockHetznerList());
    case "remote_cloud_list": return r(mockCloudList(String(args?.provider ?? "aws")));
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
    case "pocket_status": return r({ state: "relay_unreachable", message: "no relay in the preview" });
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

    case "money_overview": return r(mockOverview());
    case "money_runs": return r(mockRuns());
    case "money_incidents": return r(mockIncidents());
    case "money_savings": return r(mockSavings());

    case "policy_list_approvals": return r(mockApprovals());
    case "policy_list_policies": return r(mockPolicies());

    case "identity_list_identities": return r(mockIdentities());
    case "identity_list_alerts": return r(mockAlerts());
    case "identity_list_remediations": return r([]);
    case "credentials_keys": return r(mockCredentialsKeys());

    case "agent_graph": return r(mockGraph());
    case "agent_record": return r(mockAgentRecord(String(args?.agent_id ?? "")));
    case "user_record": return r(mockUserRecord(String(args?.user ?? args?.handle ?? "")));
    case "unit_record": return r(mockUnitRecord(String(args?.team ?? "")));
    case "agent_slice": return r(mockSlice(String(args?.agent_id ?? "")));
    case "org_directory": return r(orgDirectory());
    case "agent_set_budget": {
      const a = findById(String(args?.agent_id ?? ""));
      if (a) { a.budgetUsd = Number(args?.budget_usd) || a.budgetUsd; logChange(a, "budget_set", `per-run ceiling set to $${a.budgetUsd.toFixed(2)}`); }
      return r(a ? { ...a } : null);
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
      return r(a ? { ...a } : null);
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
      return r(a ? { ...a } : null);
    }
    case "agent_set_behaviour": {
      const a = findById(String(args?.agent_id ?? ""));
      const allowed = Array.isArray(args?.allowed) ? (args?.allowed as string[]) : null;
      if (a && allowed) { a.allowed = allowed; logChange(a, "transferred", "allowed behaviour updated"); }
      return r(a ? { ...a } : null);
    }
    case "agent_block": {
      const a = findById(String(args?.agent_id ?? ""));
      const blocked = Boolean(args?.blocked);
      if (a) { a.blocked = blocked; logChange(a, blocked ? "closed" : "launched", blocked ? "disabled by operator" : "re-enabled by operator"); }
      return r(a ? { ...a } : null);
    }
    case "user_block": {
      const handle = String(args?.user ?? "").replace(/^user:\/\/[^/]+\//, "");
      const blocked = Boolean(args?.blocked);
      FLEET.filter((a) => a.owner === handle).forEach((a) => { a.blocked = blocked; logChange(a, blocked ? "closed" : "launched", blocked ? `disabled with owner ${handle}` : `re-enabled with owner ${handle}`); });
      return r(mockUserRecord(handle));
    }
    case "unit_block": {
      const team = String(args?.team ?? "");
      const blocked = Boolean(args?.blocked);
      FLEET.filter((a) => a.team === team).forEach((a) => { a.blocked = blocked; logChange(a, blocked ? "closed" : "launched", blocked ? `disabled with unit ${team}` : `re-enabled with unit ${team}`); });
      return r(mockUnitRecord(team));
    }
    case "agent_events": {
      const id = String(args?.agent_id ?? "");
      const limit = Number(args?.limit ?? 50);
      const evts = seedEvents(limit).filter((e) => e.agent_id === id);
      return r(evts.length ? evts : seedEvents(limit).slice(0, 6).map((e) => ({ ...e, agent_id: id })));
    }
    // The seeded quality_drift event is APPENDED after the freshly-generated
    // ones (see mockQualityDriftEvent's own doc comment for why order matters
    // here), so `res.events[0]` (newest-first) is still whichever real event
    // `seedEvents` itself produced most recently.
    case "recent_events": return r([...seedEvents(Number(args?.limit ?? 60)), mockQualityDriftEvent()]);
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
  const tick = () => onEvent(makeEvent() as unknown as T);
  const id = window.setInterval(tick, 1300 + Math.floor(Math.random() * 900));
  return Promise.resolve(() => window.clearInterval(id));
}
