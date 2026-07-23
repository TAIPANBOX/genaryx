/**
 * Money + Overview wire types. Mirrors the Rust DTOs in
 * `src-tauri/src/money/commands.rs` and `src-tauri/src/money/env.rs`
 * field-for-field (same convention as `types.ts`'s `UiEvent` mirroring the
 * Bus Explorer's Rust `UiEvent`), including the exact serde tag/rename_all
 * shape of every enum so `invoke<T>(...)` results type-check honestly
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
  | { kind: "plan_required"; feature: string; org: string; upgrade_url: string }
  | { kind: "break_glass_missing_reason" }
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

/** Mirrors `money::commands::RunDto`. */
export interface Run {
  run_id: string;
  model: string;
  agent_id: string;
  spent_usd: number;
  budget_usd: number | null;
  calls: number;
  cache_hits: number;
  steps: number;
  last_seen: string;
  killed: boolean;
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
