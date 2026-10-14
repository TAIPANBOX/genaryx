/**
 * Money + Overview wire types. Mirrors the Rust DTOs in
 * `crates/api/src/money/commands.rs` and `crates/api/src/money/env.rs`
 * field-for-field (same convention as `types.ts`'s `UiEvent` mirroring the
 * Bus Explorer's Rust `UiEvent`), including the exact serde tag/rename_all
 * shape of every enum so `invokeBackend<T>(...)` results type-check honestly
 * instead of being cast.
 */

/** Mirrors `money::env::EnvSource` (`#[serde(tag = "source", rename_all = "snake_case")]`). */
export type EnvSource = { source: "taipan"; name: string } | { source: "env_fallback" };

/** Mirrors `money::commands::MoneyStatusDto` (`#[serde(tag = "state", rename_all = "snake_case")]`). */
export type MoneyStatus =
  | { state: "bootstrapping" }
  | { state: "no_environment" }
  | { state: "pairing_failed"; source: EnvSource; cloud_url: string; reason: string }
  | { state: "ready"; source: EnvSource; cloud_url: string; org_domain: string };

/** Mirrors `money::commands::MoneyError` (`#[serde(tag = "kind", rename_all = "snake_case")]`).
 * `break_glass_missing_reason` (Phase-2 wave 3B) is the shell's own fail-closed
 * backstop for `money_kill_run`/`money_set_budget`: in normal use the break-glass
 * ceremony's confirm button is disabled until a reason is typed, so the frontend
 * should never actually see this - it exists so an empty reason can never reach
 * the Cloud, not as an expected UI state.
 *
 * `role_required` is the one variant NOT mirrored from the Rust command's own
 * `MoneyError` enum: it is `lib/money.ts`'s `toMoneyError` recognizing
 * `genaryx-web`'s command-chokepoint role gate (docs/CONSOLE-IDP.md), a 403
 * that happens BEFORE the command ever reaches `money::commands` - added here
 * client-side so the existing error banner can render it honestly. */
export type MoneyError =
  | { kind: "bootstrapping" }
  | { kind: "no_environment" }
  | { kind: "pairing_failed"; reason: string }
  // Legacy. Nothing in this stack is sold, so no current TokenFuse Cloud
  // emits this. Kept so a console pointed at an older plane reports the 402
  // honestly instead of going blank; `upgrade_url` is parsed and never shown.
  | { kind: "plan_required"; feature: string; org: string; upgrade_url: string }
  | { kind: "break_glass_missing_reason" }
  | { kind: "invalid_argument"; message: string }
  | { kind: "cloud"; status: number | null; message: string }
  | { kind: "role_required"; role: "viewer" | "approver" | "admin" };

/** Mirrors `money::commands::OverviewDto`. */
export interface Overview {
  total_spent_usd: number;
  total_calls: number;
  total_runs: number;
  active_runs: number;
  killed_runs: number;
  open_incidents: number;
  total_incidents: number;
  total_saved_usd: number;
}

/** Mirrors `money::commands::RunDto`.
 *
 * `lifecycle` is a MOCK-ONLY enrichment (not part of the Rust `RunDto`): the
 * preview world sets it so a run can read STOPPED/FROZEN precisely, not just
 * `killed`, when its agent was halted by an operator (its unit/user stopped,
 * or the agent frozen). A real box omits it, and every consumer falls back to
 * the `killed` boolean below - see `lib/lifecycleTypes.ts`'s module doc. */
export interface Run {
  run_id: string;
  /** The business unit the Cloud's identity map RESOLVED for this run
   * (`tokenfuse/docs/20-identity-map.md`): the attribution that actually
   * charged a unit budget.
   *
   * `""` when nothing resolved, which is a real answer and not a gap: a box
   * with no identity map configured returns it for every run. A caller must be
   * able to tell that from a resolved unit, which is why it is an empty string
   * rather than a missing field. */
  unit: string;
  /** The human at the root of this run's delegation chain (`RunAgg.owner`).
   * `""` means a Cloud older than this field sent nothing at all - "not
   * reported", not "nobody" (the money plane's separate `/v1/owners` view
   * says "nobody" through the literal `"unassigned"` bucket, never through
   * this field). */
  owner: string;
  model: string;
  agent_id: string;
  spent_usd: number;
  /** The run's budget on the Cloud (`GET /v1/budgets`). `null` means "no
   * budget" only when `budgets_read` is true; otherwise it means "could not
   * read", and a view must say so rather than print "no cap". */
  budget_usd: number | null;
  /** Whether the Cloud's budget map answered for this read. False when it did
   * not: only the budgets `/v1/alerts` lists are known then, so a `null`
   * budget is unknown, never "none" (invariant 17). */
  budgets_read: boolean;
  calls: number;
  cache_hits: number;
  steps: number;
  last_seen: string;
  killed: boolean;
  lifecycle?: import("./lib/lifecycleTypes").EntityLifecycleState;
  /* How the gateway decided this run, as TokenFuse Cloud reports it on every
   * `GET /v1/runs` row (v1.2.0 for `site`, v1.8.0 for the rest). The Rust
   * `RunDto` sends `null` for a field this Cloud does not report, and the
   * views print that as "not reported", never as a zero: a zero is a count
   * the Cloud made. Optional here as well, because the mock preview and any
   * older backend omit them; absent and `null` mean the same thing. Read
   * through `lib/runStatus.ts`, never directly. */
  /** The site (gateway) that pushed the run; `""` = reported, none named. */
  site?: string | null;
  /** Calls the gateway refused on this run. */
  blocked?: number | null;
  /** The latest call's decision: `allow`, `cache_hit`, or a refusal reason. */
  last_decision?: string | null;
  /** When `last_decision` was taken, ISO-8601 UTC. */
  last_decision_at?: string | null;
  /** Admitted calls charged at the fallback rate (no price-book row). */
  fallback_calls?: number | null;
  /** Admitted calls whose gateway named no price basis. */
  basis_unreported_calls?: number | null;
  /** Admitted calls forwarded under identity warn. */
  identity_warned?: number | null;
  /** The gateway's identity reason for the latest warned or refused call. */
  last_identity_reason?: string | null;
}

/** Mirrors `money::units::UnitCapState`: where a unit stands against its
 * central month cap. `no_cap` is a real answer from a cap map that was read;
 * `cap_unknown` means the map was not read and is never shown as "no cap". */
export type UnitCapState = "over_cap" | "near_cap" | "under_cap" | "no_cap" | "cap_unknown" | "spend_unknown";

/** Mirrors `money::units::UnitCapDto`. */
export interface UnitCap {
  unit: string;
  /** Month-to-date spend; `null` when the Cloud has no record of the unit this
   * month or `/v1/units` was not read. Never a zero it did not report. */
  month_spent_usd: number | null;
  month_calls: number | null;
  /** The central cap; `null` when none is set or the map was not read. */
  cap_usd: number | null;
  fraction: number | null;
  state: UnitCapState;
  /** False for the `unassigned` bucket, which no cap can apply to. */
  cappable: boolean;
}

/** Mirrors `money::units::UnitCapsDto`. */
export interface UnitCaps {
  /** The UTC month the spend covers (`2026-10`), `null` when not read. */
  month: string | null;
  units_read: boolean;
  caps_read: boolean;
  /** One sentence per read that did not answer. */
  notes: string[];
  units: UnitCap[];
}

/** Mirrors `money::commands::IncidentDto`. `severity` is a raw lowercase
 * string, same tolerant convention as `types.ts`'s bus `UiEvent.severity`. */
export interface Incident {
  id: string;
  run_id: string | null;
  agent_id: string | null;
  kind: string;
  severity: string;
  first_seen: string;
  last_seen: string;
  occurrences: number;
  acknowledged: boolean;
}

/** Mirrors `money::commands::OwnerDto`: the money plane's own per-person
 * rollup.
 *
 * The person here is the root of the delegation chain, so this says who STARTED
 * the run. The console's other owner view joins through idryx and says who OWNS
 * the agent. An agent owned by one person and run on another's behalf lands
 * under different names in the two, and both are right about their own
 * question, which is why nothing in this app adds them together. */
export interface Owner {
  owner: string;
  spent_usd: number;
  calls: number;
  runs: number;
  agents: number;
  last_seen: string;
  tool_calls: number;
}

/** Mirrors `money::commands::SavingsDto`. */
export interface Savings {
  blocked_spend_usd: number;
  cache_saved_usd: number;
  router_saved_usd: number;
  budget_breaks: number;
  total_saved_usd: number;
}

/** Mirrors `money::commands::MutationOutcome`. */
export interface MutationOutcome {
  summary: string;
  http_status: number;
  verify_result: string;
  sig_alg: string;
  sig_fpr: string;
  bus_recorded: boolean;
  bus_error: string | null;
}
