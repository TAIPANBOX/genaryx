/**
 * Browser-preview fallback data: the same ~40-event timeline as the Rust
 * mock in `src-tauri/src/events.rs` (kept in sync by hand; both mirror the
 * canonical event/type/severity shapes `genaryx_core::demo` uses), so a
 * plain `vite build` / `vite preview` without the Tauri runtime still
 * renders a realistic Bus Explorer. See `lib/recentEvents.ts` for when this
 * is used instead of the real `invoke("recent_events")`.
 */
import type { Severity, UiEvent } from "./types";

const TOPICS = [
  "customer_refund_policy",
  "kyc_verification_steps",
  "fraud_hold_criteria",
  "sla_response_times",
  "chargeback_procedure",
  "aml_screening_rules",
] as const;

const EVAL_SUITES = [
  "refund-policy-qa",
  "kyc-accuracy-qa",
  "fraud-triage-qa",
  "sla-compliance-qa",
  "aml-screening-qa",
] as const;

const SCENARIOS = [
  "prod-deploy-rehearsal",
  "budget-exhaustion-drill",
  "policy-bypass-drill",
  "credential-leak-drill",
  "runaway-agent-drill",
] as const;

const SCHEMA_V0_1 = "taipanbox.dev/agent-event/v0.1";
const SCHEMA_V0_2 = "taipanbox.dev/agent-event/v0.2";

interface Seed {
  source: string;
  v2: boolean;
  type: string;
  severity: Severity;
  agent: string;
  run: number;
  delegated: boolean;
}

// Oldest first (reversed to newest-first at the end), mirroring the Rust
// seed table exactly so preview and the real shell look the same.
const SEEDS: Seed[] = [
  { source: "wardryx", v2: true, type: "policy_allow", severity: "info", agent: "tier1-bot", run: 1, delegated: false },
  { source: "tokenfuse", v2: false, type: "budget_exhausted", severity: "critical", agent: "tier1-bot", run: 1, delegated: false },
  { source: "engram", v2: false, type: "memory_written", severity: "info", agent: "tier1-bot", run: 1, delegated: false },
  { source: "wardryx", v2: true, type: "policy_allow", severity: "info", agent: "tier2-bot", run: 2, delegated: false },
  { source: "tokenfuse", v2: false, type: "breaker_tripped", severity: "critical", agent: "tier2-bot", run: 2, delegated: false },
  { source: "engram", v2: false, type: "memory_written", severity: "info", agent: "tier2-bot", run: 2, delegated: false },
  { source: "wardryx", v2: true, type: "policy_deny", severity: "high", agent: "ci-fixer", run: 3, delegated: true },
  { source: "wardryx", v2: true, type: "approval_requested", severity: "medium", agent: "ci-fixer", run: 3, delegated: true },
  { source: "wardryx", v2: true, type: "approval_granted", severity: "info", agent: "ci-fixer", run: 3, delegated: true },
  { source: "tokenfuse", v2: false, type: "spend_spike", severity: "high", agent: "fraud-bot", run: 4, delegated: false },
  { source: "verdryx", v2: true, type: "quality_score", severity: "info", agent: "fraud-bot", run: 4, delegated: false },
  { source: "engram", v2: false, type: "memory_written", severity: "info", agent: "kyc-bot", run: 5, delegated: false },
  { source: "engram", v2: false, type: "contradiction_found", severity: "medium", agent: "kyc-bot", run: 5, delegated: false },
  { source: "verdryx", v2: true, type: "quality_score", severity: "info", agent: "refund-bot", run: 6, delegated: false },
  { source: "verdryx", v2: true, type: "quality_drift", severity: "high", agent: "refund-bot", run: 6, delegated: false },
  { source: "mockryx", v2: true, type: "sim_run", severity: "info", agent: "audit-bot", run: 7, delegated: false },
  { source: "mockryx", v2: true, type: "sim_finding", severity: "medium", agent: "audit-bot", run: 7, delegated: false },
  { source: "mockryx", v2: true, type: "blast_radius_measured", severity: "medium", agent: "audit-bot", run: 7, delegated: false },
  { source: "qryx", v2: false, type: "crypto_finding", severity: "medium", agent: "verifier", run: 8, delegated: false },
  { source: "qryx", v2: false, type: "evidence_signed", severity: "info", agent: "verifier", run: 8, delegated: false },
  { source: "tokenfuse", v2: false, type: "sustained_loop", severity: "high", agent: "router", run: 9, delegated: false },
  { source: "wardryx", v2: true, type: "policy_allow", severity: "info", agent: "router", run: 9, delegated: false },
  { source: "tokenfuse", v2: false, type: "fanout_explosion", severity: "high", agent: "orchestrator", run: 10, delegated: false },
  { source: "engram", v2: false, type: "memory_written", severity: "info", agent: "orchestrator", run: 10, delegated: false },
  { source: "wardryx", v2: true, type: "policy_deny", severity: "high", agent: "deploy-bot", run: 11, delegated: true },
  { source: "wardryx", v2: true, type: "approval_requested", severity: "medium", agent: "deploy-bot", run: 11, delegated: true },
  { source: "wardryx", v2: true, type: "approval_granted", severity: "info", agent: "deploy-bot", run: 11, delegated: true },
  { source: "verdryx", v2: true, type: "quality_score", severity: "info", agent: "collections-bot", run: 12, delegated: false },
  { source: "mockryx", v2: true, type: "sim_run", severity: "info", agent: "sentinel", run: 13, delegated: false },
  { source: "mockryx", v2: true, type: "sim_finding", severity: "medium", agent: "sentinel", run: 13, delegated: false },
  { source: "qryx", v2: false, type: "crypto_finding", severity: "medium", agent: "auditor", run: 14, delegated: false },
  { source: "qryx", v2: false, type: "evidence_signed", severity: "info", agent: "auditor", run: 14, delegated: false },
  { source: "tokenfuse", v2: false, type: "budget_exhausted", severity: "critical", agent: "billing-bot", run: 15, delegated: false },
  { source: "engram", v2: false, type: "memory_written", severity: "info", agent: "billing-bot", run: 15, delegated: false },
  { source: "wardryx", v2: true, type: "policy_allow", severity: "info", agent: "scheduler", run: 16, delegated: false },
  { source: "verdryx", v2: true, type: "quality_drift", severity: "high", agent: "onboarding-bot", run: 17, delegated: false },
  { source: "mockryx", v2: true, type: "blast_radius_measured", severity: "medium", agent: "reconciler", run: 18, delegated: false },
  { source: "qryx", v2: false, type: "crypto_finding", severity: "medium", agent: "translator", run: 19, delegated: false },
  { source: "wardryx", v2: true, type: "policy_deny", severity: "high", agent: "support-bot", run: 20, delegated: true },
  { source: "tokenfuse", v2: false, type: "breaker_tripped", severity: "critical", agent: "support-bot", run: 20, delegated: true },
];

function seedData(type: string, run: number, agentId: string): unknown {
  const topic = TOPICS[run % TOPICS.length];
  const evalSuite = EVAL_SUITES[run % EVAL_SUITES.length];
  const scenario = SCENARIOS[run % SCENARIOS.length];

  switch (type) {
    case "budget_exhausted":
      return { budget_usd: 0.0012, spent_usd: 0.0028, reason: "budget_exceeded", policy_id: "default" };
    case "breaker_tripped":
      return { budget_usd: 0.0009, spent_usd: 0.0021, reason: "budget_exceeded", policy_id: "default" };
    case "spend_spike":
      return { window_s: 60, spend_usd: 7.42, baseline_usd: 1.15, multiplier: 6.4 };
    case "sustained_loop":
      return { calls: 88, window_s: 120, pattern: "repeated_tool_call" };
    case "fanout_explosion":
      return { child_agents: 7, depth: 3, budget_usd: 2.85 };
    case "policy_allow":
      return { policy: "default-allow", reason: "within policy" };
    case "policy_deny":
      return { policy: "prod-deploy-requires-approval", reason: "no approval on file for deploy:prod scope" };
    case "approval_requested":
      return { policy: "prod-deploy-requires-approval", reason: "awaiting operator approval" };
    case "approval_granted":
      return { policy: "prod-deploy-requires-approval", granted_by: "user://taipanbox.dev/j.doe" };
    case "memory_written":
      return { memory_id: `mem-${String(3000 + run).padStart(4, "0")}`, topic };
    case "contradiction_found":
      return {
        memory_id: `mem-${String(3000 + run).padStart(4, "0")}`,
        conflicting_memory_id: `mem-${String(2000 + run).padStart(4, "0")}`,
        topic,
      };
    case "quality_score":
      return { eval_suite: evalSuite, current_score: 0.93 };
    case "quality_drift":
      return { eval_suite: evalSuite, baseline_score: 0.97, current_score: 0.89, delta: -0.08 };
    case "sim_run":
      return { scenario, status: "completed" };
    case "sim_finding":
      return { scenario, finding: "gap_found" };
    case "blast_radius_measured":
      return { scenario, blast_radius_score: 0.52, affected_resources: 14 };
    case "crypto_finding":
      return { algorithm: "rsa-2048", risk: "quantum-vulnerable", recommended: "ml-dsa-65" };
    case "evidence_signed":
      return { evidence_id: `ev-${55_000 + run}`, algorithm: "ml-dsa-65", subject: agentId };
    default:
      return {};
  }
}

function buildMockEvents(): UiEvent[] {
  const now = Date.now();
  const n = SEEDS.length;

  const events = SEEDS.map((s, i) => {
    const id = i + 1;
    const tsMs = now - (n - i) * 45_000;
    const ts = new Date(tsMs).toISOString();
    const schema = s.v2 ? SCHEMA_V0_2 : SCHEMA_V0_1;
    const agent_id = `agent://taipanbox.dev/demo/${s.agent}`;
    const run_id = `demo-run-${String(s.run).padStart(3, "0")}`;
    const data = seedData(s.type, s.run, agent_id);
    const on_behalf_of = s.delegated
      ? ["user://taipanbox.dev/j.doe", "agent://taipanbox.dev/demo/orchestrator"]
      : [];

    const rawObj: Record<string, unknown> = {
      schema,
      ts,
      source: s.source,
      type: s.type,
      agent_id,
      severity: s.severity,
      run_id,
    };
    if (on_behalf_of.length > 0) rawObj.on_behalf_of = on_behalf_of;
    rawObj.data = data;

    const event: UiEvent = {
      id,
      env: "local",
      ts,
      source: s.source,
      type: s.type,
      agent_id,
      run_id,
      severity: s.severity,
      schema,
      on_behalf_of,
      data,
      prev_hash: null,
      raw: JSON.stringify(rawObj),
      file: `~/.taipan/events/${s.source}.ndjson`,
      off: i * 128,
    };
    return event;
  });

  events.reverse(); // newest first by id
  return events;
}

/** Computed once per module load so timestamps stay stable within a session
 * (each call to `fetchRecentEvents` re-slices the same list, it does not
 * regenerate it). */
export const MOCK_EVENTS: UiEvent[] = buildMockEvents();
