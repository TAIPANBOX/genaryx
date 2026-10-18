/**
 * Delegation wire types. Mirrors the Rust DTOs in
 * `crates/api/src/delegation/commands.rs` field-for-field (same convention
 * as `moneyTypes.ts` mirroring `money::commands`), including the exact
 * serde tag/rename_all shape of the error enum so `invokeBackend<T>(...)`
 * results type-check honestly instead of being cast.
 */

/** Mirrors `delegation::commands::DelegationError`
 * (`#[serde(tag = "kind", rename_all = "snake_case")]`).
 *
 * `role_required` is not part of the Rust command's own enum, same reason as
 * `MoneyError`'s own copy of this comment: `lib/delegation.ts`'s
 * `toDelegationError` recognizes `genaryx-web`'s command-chokepoint role
 * gate, a 403 that happens BEFORE the command ever reaches
 * `delegation::commands`, added here client-side so the existing error
 * banner can render it honestly. */
export type DelegationError =
  | { kind: "not_configured" }
  | { kind: "exactly_one_target" }
  | { kind: "invalid_subject"; subject: string }
  | { kind: "invalid_reason"; chars: number }
  | { kind: "refused"; status: number; error: string }
  | { kind: "not_durable" }
  | { kind: "unreachable"; detail: string }
  | { kind: "role_required"; role: "viewer" | "approver" | "admin" };

/** Mirrors `delegation::commands::RevokeOutcome`. `vouchryx_response` is
 * vouchryx's own JSON body, forwarded whole rather than re-typed - see the
 * Rust field's own doc for why. */
export interface RevokeOutcome {
  http_status: number;
  verify_result: string;
  vouchryx_response: unknown;
  bus_recorded: boolean;
  bus_error: string | null;
}

/** Mirrors `delegation::stop::StopStep`: one step of a stop or a lift, as
 * the console journaled it. */
export interface StopStep {
  step: "ceremony" | "ban" | "list_runs" | "kill_run" | "lift" | "verdict" | string;
  target: string;
  ok: boolean;
  http_status: number;
  verify_result: string;
  journaled: boolean;
  journal_error: string | null;
}

/** Mirrors `delegation::stop::StopOutcome`. */
export interface StopOutcome {
  subject: string;
  stopped: boolean;
  ban_durable: boolean | null;
  open_runs: number | null;
  runs_killed: number;
  warnings: string[];
  steps: StopStep[];
}

/** Mirrors `delegation::stop::LiftOutcome`. */
export interface LiftOutcome {
  subject: string;
  lifted: boolean;
  lift_durable: boolean | null;
  warnings: string[];
  steps: StopStep[];
}

/** Mirrors `delegation::stop::StopError` (`tag = "kind"`), plus the role
 * gate's refusal and a transport failure, which happen before the command. */
export type StopError =
  | { kind: "invalid_subject"; subject: string }
  | { kind: "invalid_reason"; chars: number }
  | { kind: "not_stopped"; outcome: StopOutcome }
  | { kind: "not_lifted"; outcome: LiftOutcome }
  | { kind: "role_required"; role: "viewer" | "approver" | "admin" }
  | { kind: "failed"; detail: string };

/** What a stop came back with: the command's Ok or its Err, never thrown. */
export type StopResult = { ok: true; outcome: StopOutcome } | { ok: false; error: StopError };

/** What a lift came back with. */
export type LiftResult = { ok: true; outcome: LiftOutcome } | { ok: false; error: StopError };
