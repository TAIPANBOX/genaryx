/**
 * Quality wire types. Mirrors the Rust DTOs in
 * `crates/api/src/quality/commands.rs` and `crates/api/src/quality/env.rs`
 * field-for-field (same convention `identityTypes.ts` follows for its own
 * panel), including the exact serde tag/rename_all shape of every enum so
 * `invokeBackend<T>(...)` results type-check honestly instead of being cast.
 *
 * `VerdryxEvalRun`/`VerdryxScore`/`VerdryxBaseline`/`VerdryxRunSummary`
 * mirror `genaryx_connectors::Verdryx*` (`crates/connectors/src/verdryx.rs`)
 * directly - those Rust types already derive `Serialize` and cross the
 * genaryx-web JSON boundary as-is, so these interfaces exist only so the frontend
 * has names/types for the exact same wire shape, not because the Rust side
 * re-wraps anything.
 */

/** Mirrors `quality::env::EnvSource` (`#[serde(tag = "source", rename_all = "snake_case")]`).
 * Unlike Identity's single-variant `EnvSource`, Quality has a (today mostly
 * theoretical) second tier: `well_known` is the common case in practice,
 * since no live taipan descriptor populates a `services.verdryx` entry yet -
 * see `quality::env`'s doc comment. */
export type EnvSource = { source: "taipan"; name: string } | { source: "well_known" };

/** Mirrors `quality::commands::QualityStatusDto`
 * (`#[serde(tag = "state", rename_all = "snake_case")]`). */
export type QualityStatus =
  | { state: "bootstrapping" }
  | { state: "no_environment" }
  | { state: "unreachable"; source: EnvSource; db_path: string; reason: string }
  | { state: "ready"; source: EnvSource; db_path: string };

/** Mirrors `quality::commands::QualityError` (`#[serde(tag = "kind", rename_all = "snake_case")]`). */
export type QualityError =
  | { kind: "bootstrapping" }
  | { kind: "no_environment" }
  | { kind: "unreachable"; reason: string }
  | { kind: "verdryx"; message: string };

/** Mirrors `genaryx_connectors::VerdryxEvalRun` (`eval_runs`, `store.py:20-25`).
 * One invocation of `verdryx eval`. */
export interface VerdryxEvalRun {
  id: string;
  model: string;
  /** UTC ISO-8601. */
  started_at: string;
  /** UTC ISO-8601, or `null` while the run is still in flight. */
  finished_at: string | null;
}

/** Mirrors `genaryx_connectors::VerdryxScore` (`scores`, `store.py:27-34`) -
 * one case's quality score within a run. */
export interface VerdryxScore {
  id: number;
  run_id: string;
  case_id: string;
  /** In `[0.0, 1.0]`. */
  value: number;
  tokens: number;
  cost_usd: number;
}

/** Mirrors `genaryx_connectors::VerdryxBaseline` (`baselines`, `store.py:38-44`) -
 * a saved mean-score snapshot a later run's drift is measured against. */
export interface VerdryxBaseline {
  id: string;
  eval_run_id: string;
  mean_score: number;
  /** UTC ISO-8601. */
  created_at: string;
  /** May be empty (`baselines.label` defaults to `''`). */
  label: string;
}

/** Mirrors `genaryx_connectors::VerdryxUnansweredReasonCount` - one
 * `unanswered.reason` bucket for a run (verdryx 2bfd3c8). */
export interface VerdryxUnansweredReasonCount {
  reason: string;
  count: number;
}

/** Mirrors `genaryx_connectors::VerdryxRunSummary` - a derived per-run
 * rollup. `mean_score` is `null` for a run with zero ANSWERED cases (never a
 * fabricated `0`) - render as "unmeasured", not `0` and not a bare "n/a".
 *
 * `unanswered_supported` is `false` on a `verdryx.db` written before verdryx
 * 2bfd3c8 added the `unanswered` table: render NO unanswered column at all in
 * that case, never a `0` that looks measured. */
export interface VerdryxRunSummary {
  run: VerdryxEvalRun;
  /** Answered (scored) cases, not "asked" - an unanswered case never reaches
   * `scores`. */
  case_count: number;
  mean_score: number | null;
  total_tokens: number;
  total_cost_usd: number;
  unanswered_count: number;
  unanswered_by_reason: VerdryxUnansweredReasonCount[];
  unanswered_supported: boolean;
}

/** Mirrors `genaryx_connectors::VerdryxBaselineSummary` - a saved baseline
 * plus the unanswered accounting of the run it was snapshotted from. */
export interface VerdryxBaselineSummary {
  baseline: VerdryxBaseline;
  unanswered_count: number;
  unanswered_by_reason: VerdryxUnansweredReasonCount[];
  unanswered_supported: boolean;
}
