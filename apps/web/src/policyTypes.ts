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
  | { kind: "approver_statement"; reason: string }
  | { kind: "standing_grant"; reason: string }
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
  /** The one call this hold is about, `null` when the hold named no single call. */
  tool_call: HeldToolCall | null;
  /** `no_grant` on a tool call nothing grants, the one hold a standing grant ends
   * for good; `null` on every other hold. */
  hold_kind: string | null;
  /** How the enforcement point knew the agent: `agent-token`, `client-key` or
   * `none`; `null` when the hold did not say. */
  identity_source: string | null;
}

/** Mirrors `policy::commands::HeldToolCallDto`: the call by name, target and the
 * digest a grant binds its token to. Wardryx leaves the arguments out on purpose,
 * so there is no field for them. Each field is `null` when the context did not
 * carry it in a readable shape. */
export interface HeldToolCall {
  name: string | null;
  target: string | null;
  /** True when the enforcement point could not send the arguments, so the digest
   * binds only the name and the target. */
  arguments_truncated: boolean | null;
  digest: string | null;
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
  /** The tools this policy lifts the no-grant floor for; `*` is every tool. */
  grant_tool: string[];
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
  /** The digest of the one call the token is bound to, `null` for a token granted
   * on a hold that named no single call. */
  tool_call_digest: string | null;
}

/** Mirrors `policy::commands::DecideOutcome`. */
export interface DecideOutcome {
  summary: string;
  http_status: number;
  verify_result: string;
  sig_alg: string;
  sig_fpr: string;
  token: DecodedToken | null;
  /** The raw token, on a grant only. A live credential and the one copy of it:
   * `lib/approvalToken.ts` says where it may go. */
  approval_token: string | null;
  bus_recorded: boolean;
  bus_error: string | null;
}

/** Mirrors `policy::commands::GrantToolOutcome`. */
export interface GrantToolOutcome {
  summary: string;
  policy_id: string;
  agent_id: string;
  /** Every tool the agent now holds through this policy. */
  tools: string[];
  http_status: number;
  verify_result: string;
  sig_alg: string;
  sig_fpr: string;
  bus_recorded: boolean;
  bus_error: string | null;
}

/** Mirrors `policy::commands::DecisionDto` (`#[serde(rename_all = "snake_case")]`) -
 * the operator's verdict, sent as the `decision` argument to `policy_decide_approval`. */
export type Decision = "grant" | "deny";

/** Mirrors `genaryx_connectors::WardryxStatus` (`GET /v1/status`): what the PDP
 * actually evaluates against, and where those rules came from.
 *
 * `effective_policies` is the only field that answers "is anything enforced at
 * all". `base_policies` are file-loaded rules that never appear in
 * `policy_list_policies`, which is why judging enforcement by that list reports
 * an enforcing plane as wide open. */
export interface WardryxStatus {
  policy_version: string;
  base_policies: number;
  store_policies: number;
  effective_policies: number;
}
