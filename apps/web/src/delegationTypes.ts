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
