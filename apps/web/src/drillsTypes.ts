/**
 * Drills wire types. Mirrors the Rust DTOs in
 * `src-tauri/src/drills/commands.rs` and `src-tauri/src/drills/env.rs`
 * field-for-field (same convention `qualityTypes.ts`/`cryptoTypes.ts` follow
 * for their own panels).
 *
 * `MockryxReport`/`MockryxResult`/`MockryxFinding`/`MockryxMetrics` mirror
 * `genaryx_connectors::Mockryx*` (`crates/connectors/src/mockryx.rs`)
 * directly - those Rust types already derive `Serialize` and cross the
 * Tauri IPC boundary as-is, so these interfaces exist only so the frontend
 * has names/types for the exact same wire shape, not because the Rust side
 * re-wraps anything. Every `Option<T>` field there serializes as `T | null`
 * (always present, never omitted); every `Vec<T>` (even one with
 * `#[serde(default)]`, which only affects deserialization) always
 * serializes as an array, possibly empty, never omitted.
 */

/** Mirrors `drills::env::EnvSource` (`#[serde(tag = "source", rename_all = "snake_case")]`).
 * A single variant - the gateway only ever comes from a `taipan up`
 * descriptor (see `drills::env`'s doc comment), same rationale
 * `identityTypes.ts`'s single-variant `EnvSource` documents for idryx. */
export type EnvSource = { source: "taipan"; name: string };

/** Mirrors `drills::commands::DrillsStatusDto`
 * (`#[serde(tag = "state", rename_all = "snake_case")]`). No `unreachable`
 * variant: mockryx has no serve process to confirm reachable at bootstrap -
 * see `drills::state`'s doc comment. `has_api_key` reports only WHETHER a
 * bearer resolved, never the value itself. */
export type DrillsStatus =
  | { state: "bootstrapping" }
  | { state: "no_environment" }
  | {
      state: "ready";
      source: EnvSource;
      mockryx_bin: string;
      gateway_url: string;
      has_api_key: boolean;
      scenario_dir: string | null;
    };

/** Mirrors `drills::commands::DrillsError` (`#[serde(tag = "kind", rename_all = "snake_case")]`). */
export type DrillsError =
  | { kind: "bootstrapping" }
  | { kind: "no_environment" }
  | { kind: "mockryx"; message: string };

/** Mirrors `genaryx_connectors::MockryxFinding` (`runner.Finding`) - one step
 * mismatch: exactly what was expected vs what the gateway returned. */
export interface MockryxFinding {
  scenario: string;
  step: string;
  attempt: number;
  expect_status: number;
  expect_header: Record<string, string> | null;
  got_status: number;
  got_headers: Record<string, string> | null;
  detail: string;
  /** Set only for a failed `expect.event` check (unused by the bundled
   * scenario set today - see the connector's own module doc). */
  expect_event_source: string | null;
  expect_event_type: string | null;
}

/** Mirrors `genaryx_connectors::MockryxMetrics` (`runner.Metrics`) - one
 * scenario's blast-radius metrics. */
export interface MockryxMetrics {
  calls: number;
  budget_burned_usd: number;
}

/** Mirrors `genaryx_connectors::MockryxResult` (`runner.Result`) - one
 * scenario's outcome. */
export interface MockryxResult {
  scenario: string;
  /** `passed | failed | skipped_not_configured` - read "gap" from
   * `findings`/`failed`, never from this alone (a bare skip is not a gap -
   * see `hasGaps`/`MockryxReport::has_gaps`'s doc). */
  status: string;
  findings: MockryxFinding[];
  /** Mismatches discarded because the scenario's guardrail was never
   * observed active (only non-empty on `skipped_not_configured`). */
  skipped_findings: MockryxFinding[];
  metrics: MockryxMetrics;
}

/** Mirrors `genaryx_connectors::MockryxReport` (`report.Report`) - the whole
 * drill report, `drills_run`'s return. */
export interface MockryxReport {
  run_id: string;
  gateway: string;
  /** RFC3339Nano UTC. NOTE the wire field is `generated_at`, not
   * `generated`. */
  generated_at: string;
  results: MockryxResult[];
}

/** Mirrors `genaryx_connectors::MockryxReport::has_gaps` exactly (Rust
 * methods do not cross the Tauri IPC boundary, so this is a deliberate,
 * one-for-one TypeScript re-implementation of that same logic, not a
 * divergent one): any scenario that outright `failed`, or any scenario
 * carrying findings (which, after `--fail-on-skip`, can include promoted
 * skips). A `skipped_not_configured` scenario with empty `findings` is NOT a
 * gap on its own. */
export function hasGaps(report: MockryxReport): boolean {
  return report.results.some((r) => r.status === "failed" || r.findings.length > 0);
}
