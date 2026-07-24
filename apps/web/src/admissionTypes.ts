/**
 * Admission-gate wire types (I6, docs/ADMISSION.md). Mirrors the Rust DTOs in
 * `genaryx_api::admission::commands` field-for-field (same convention
 * `onboardTypes.ts`/`drillsTypes.ts` follow for their own panels).
 *
 * `GatewayKeyEntry` is deliberately NOT redeclared here: `admission_check`'s
 * `key` field is the exact same wire shape `lib/credentials.ts` already
 * types and derives helpers over (`deriveKeyStatus`, `totalCalls`,
 * `maxLastSeenMillis`, `lastSeenLabel`) - importing it from there means the
 * Verify scoreboard reuses those helpers unchanged instead of a second,
 * parallel implementation.
 */
import type { GatewayKeyEntry } from "./lib/credentials";

export type { GatewayKeyEntry };

/** Mirrors `admission::env::EnvSource` (the gateway leg's own source tag,
 * `#[serde(tag = "source", rename_all = "snake_case")]`) - a single variant,
 * same rationale as `credentials.ts`'s/`drillsTypes.ts`'s own `EnvSource`.
 * Named distinctly (not `EnvSource`) so a component that imports both this
 * module and `lib/credentials.ts` never has to alias either. */
export type GatewaySource = { source: "taipan"; name: string };

/** Mirrors `admission::commands::GatewayStatusDto` - the gateway leg's own
 * connection state, structurally identical to `CredentialsStatus`
 * (`lib/credentials.ts`) since both are built the same way over the same
 * `GatewayClient`. */
export type GatewayStatus =
  | { state: "bootstrapping" }
  | { state: "no_environment" }
  | { state: "unreachable"; source: GatewaySource; gateway_url: string; reason: string }
  | { state: "ready"; source: GatewaySource; gateway_url: string };

/** Mirrors `admission::env::VerdryxDbSource`. */
export type VerdryxDbSource = { source: "taipan"; name: string } | { source: "well_known" };

/** Mirrors `admission::commands::VerdryxDbStatusDto`. */
export interface VerdryxDbStatus {
  source: VerdryxDbSource;
  path: string;
}

/** Mirrors `admission::commands::AdmissionStatusDto` - `admission_status`'s
 * result. Every leg reported independently and honestly (see
 * `admission::env`'s own doc comment, "Honest per-piece resolution
 * states"): the gateway's connection state, the verdryx binary's presence,
 * the verdryx db's resolution, and whether the Drills plane's own scenario
 * dir default exists - never fails. */
export interface AdmissionStatus {
  gateway: GatewayStatus;
  /** The one candidate path this plane looks for the `verdryx` binary at,
   * always named even when it does not exist. */
  verdryx_bin: string;
  verdryx_bin_present: boolean;
  /** `null` when no `verdryx.db` candidate resolved at all. */
  verdryx_db: VerdryxDbStatus | null;
  /** `crate::drills::env`'s own well-known scenario dir, when it exists. */
  drills_scenario_dir: string | null;
}

/** Mirrors `admission::commands::AdmissionCheckDto` - `admission_check`'s
 * result: a viewer-safe read straight off the gateway's key-lifecycle
 * report, plus the docs/20 `in_map` check. */
export interface AdmissionCheck {
  key_id: string;
  agent_id: string;
  /** `"off" | "warn" | "enforce"` (tokenfuse docs/20) - not a closed enum on
   * the wire, same tolerance `GatewayKeysReport.strict_mode` already has in
   * `lib/credentials.ts`. */
  strict_mode: "off" | "warn" | "enforce" | (string & {});
  identity_map_configured: boolean;
  /** `null` when no entry in the report has this `key_id` at all - "key
   * unknown to the gateway", the scoreboard's most basic red flag. */
  key: GatewayKeyEntry | null;
  /** Whether `agent_id` matches ANY `agents` pattern on ANY key entry in the
   * report (docs/20 grammar: literal, or a single trailing `*`). */
  in_map: boolean;
}

/** Mirrors `admission::commands::AdmissionBaselineDto` - `admission_baseline`'s
 * result. */
export interface AdmissionBaseline {
  run_id: string;
  /** `null` when the run scored zero cases (never a fabricated 0.0). */
  mean_score: number | null;
  case_count: number;
  total_cost_usd: number;
  /** The parsed baseline id when `verdryx baseline`'s stdout could be read,
   * else the `--label` this call requested (`admission-<agent_id>`) - still
   * a genuine, queryable handle on the baseline even when the id itself
   * could not be parsed. */
  baseline_id_or_label: string;
}

/** Mirrors `admission::commands::AdmissionError`
 * (`#[serde(tag = "kind", rename_all = "snake_case")]`). */
export type AdmissionError =
  | { kind: "bootstrapping" }
  | { kind: "no_environment" }
  | { kind: "unreachable"; reason: string }
  | { kind: "gateway"; status: number | null; message: string }
  | { kind: "verdryx_bin_missing"; path: string }
  | { kind: "verdryx_db_missing" }
  | { kind: "verdryx"; message: string }
  | { kind: "unparseable_output"; context: string; stdout_excerpt: string }
  | { kind: "run_not_found"; run_id: string };
