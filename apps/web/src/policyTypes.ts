/**
 * Policy + Approvals wire types. Mirrors the Rust DTOs in
 * `crates/api/src/policy/commands.rs` and `crates/api/src/policy/env.rs`
 * field-for-field (same convention `moneyTypes.ts` follows for the Money
 * panel), including the exact serde tag/rename_all shape of every enum so
 * `invokeBackend<T>(...)` results type-check honestly instead of being cast.
 */

/** Mirrors `policy::env::EnvSource` (`#[serde(tag = "source", rename_all = "snake_case")]`).
 * Structurally identical to `moneyTypes.ts`'s `EnvSource` (both panels
 * discover from the same `taipan up` descriptor shape) but kept as its own
 * type rather than shared, matching the Rust side's own "parallel, not
 * coupled" convention between `policy::env` and `money::env`. */
export type EnvSource = { source: "taipan"; name: string } | { source: "env_fallback" };

/** Mirrors `policy::commands::PolicyStatusDto` (`#[serde(tag = "state", rename_all = "snake_case")]`). */
export type PolicyStatus =
  | { state: "bootstrapping" }
  | { state: "no_environment" }
  | { state: "unreachable"; source: EnvSource; wardryx_url: string; reason: string }
  | { state: "ready"; source: EnvSource; wardryx_url: string; org_domain: string };

/** Mirrors `policy::commands::PolicyError` (`#[serde(tag = "kind", rename_all = "snake_case")]`).
 *
 * `role_required` is the one variant NOT mirrored from the Rust command's own
 * `PolicyError` enum: it is `lib/policy.ts`'s `toPolicyError` recognizing
 * `genaryx-web`'s command-chokepoint role gate (docs/CONSOLE-IDP.md), a 403
 * that happens BEFORE the command ever reaches `policy::commands` - added
 * here client-side so the existing error banner can render it honestly. */
export type PolicyError =
  | { kind: "bootstrapping" }
  | { kind: "no_environment" }
  | { kind: "unreachable"; reason: string }
  | { kind: "wardryx"; status: number | null; message: string }
  | { kind: "role_required"; role: "viewer" | "approver" | "admin" };

/** Mirrors `policy::commands::ApprovalDto`. */
export interface Approval {
  approval_id: string;
  agent_id: string;
  run_id: string;
  requested_at: string;
  decided_at: string | null;
  decided_by: string | null;
  decision: string | null;
  pending: boolean;
  tool_names: string[];
  est_cost_usd: number | null;
  reason: string | null;
  on_behalf_of: string[] | null;
  policy_version: string | null;
  org: string | null;
  model: string | null;
}

/** Mirrors `policy::commands::PolicyRecordDto`. */
export interface PolicyRecord {
  id: string;
  name: string;
  target: string;
  deny_tool: string[];
  allow_domains: string[];
  require_human_above_usd: number;
  deny_above_usd: number;
  max_steps: number;
  deny_if_unattested: boolean;
  updated_at: string | null;
}

/** Mirrors `policy::commands::DecodedTokenDto`. `exp_unix` (not a
 * pre-computed "seconds remaining") so the UI can drive a live countdown. */
export interface DecodedToken {
  agent_id: string;
  run_id: string;
  tools: string[];
  cost_ceiling_usd: number;
  exp_unix: number;
}

/** Mirrors `policy::commands::DecideOutcome`. */
export interface DecideOutcome {
  summary: string;
  http_status: number;
  verify_result: string;
  sig_alg: string;
  sig_fpr: string;
  token: DecodedToken | null;
  bus_recorded: boolean;
  bus_error: string | null;
}

/** Mirrors `policy::commands::DecisionDto` (`#[serde(rename_all = "snake_case")]`) -
 * the operator's verdict, sent as the `decision` argument to `policy_decide_approval`. */
export type Decision = "grant" | "deny";
