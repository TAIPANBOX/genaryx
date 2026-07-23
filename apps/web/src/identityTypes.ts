/**
 * Identity wire types. Mirrors the Rust DTOs in
 * `src-tauri/src/identity/commands.rs` and `src-tauri/src/identity/env.rs`
 * field-for-field (same convention `policyTypes.ts`/`moneyTypes.ts` follow
 * for their own panels), including the exact serde tag/rename_all shape of
 * every enum so `invoke<T>(...)` results type-check honestly instead of
 * being cast.
 *
 * `IdryxIdentity`/`IdryxAlert`/`IdryxRecommendation`/`IdryxPermission`/
 * `IdryxRemediation` mirror `genaryx_connectors::Idryx*` (`crates/connectors/src/idryx.rs`)
 * directly - those Rust types already derive `Serialize` and cross the
 * Tauri IPC boundary as-is (docs/PHASE3.md W2: "you do NOT need to write
 * UI-mirror DTOs for them"), so these interfaces exist only so the
 * frontend has names/types for the exact same wire shape, not because the
 * Rust side re-wraps anything.
 */

/** Mirrors `identity::env::EnvSource` (`#[serde(tag = "source", rename_all = "snake_case")]`).
 * A single variant today - unlike `policyTypes.ts`/`moneyTypes.ts`'s
 * `EnvSource`, idryx has no bearer at all to gate a hand-started-idryx env
 * fallback on, so this module only ever discovers from a `taipan up`
 * descriptor (see `identity::env`'s doc comment). Kept as a tagged union
 * rather than a bare `{ name: string }` so it stays structurally parallel
 * to the other two panels' `EnvSource`, in case a second discovery path is
 * ever added here too. */
export type EnvSource = { source: "taipan"; name: string };

/** Mirrors `identity::commands::IdentityStatusDto` (`#[serde(tag = "state", rename_all = "snake_case")]`).
 * `Ready.rescan_available` says whether `~/.taipan/bin/idryx` resolved, so
 * the Rescan button can disable itself up front with an honest tooltip
 * instead of only discovering unavailability after a click. */
export type IdentityStatus =
  | { state: "bootstrapping" }
  | { state: "no_environment" }
  | { state: "unreachable"; source: EnvSource; idryx_url: string; reason: string }
  | { state: "ready"; source: EnvSource; idryx_url: string; rescan_available: boolean };

/** Mirrors `identity::commands::IdentityError` (`#[serde(tag = "kind", rename_all = "snake_case")]`).
 * `rescan_unavailable` is specific to this panel: Rescan was requested but
 * no `idryx` binary was ever resolved - reported honestly rather than a
 * generic transport-shaped error. */
export type IdentityError =
  | { kind: "bootstrapping" }
  | { kind: "no_environment" }
  | { kind: "unreachable"; reason: string }
  | { kind: "idryx"; status: number | null; message: string }
  | { kind: "rescan_unavailable" };

/** Mirrors `genaryx_connectors::IdryxPermission` (`apiPermission`, idryx
 * `server.go:82-86`). The underlying ARN is deliberately not exposed on
 * the wire at all - there is no field for it here to omit. */
export interface IdryxPermission {
  name: string;
  admin: boolean;
  used: boolean;
}

/** Mirrors `genaryx_connectors::IdryxRemediation` (`apiRemediation`, idryx
 * `server.go:88-93`) - a right-size or rotation suggestion. `kind` is
 * `"right_size"` or `"rotation"`. */
export interface IdryxRemediation {
  kind: string;
  explanation: string;
  code: string;
  created_at: string;
}

/** Mirrors `genaryx_connectors::IdryxIdentity` (`apiIdentity`, idryx
 * `server.go:119-134`) field-for-field, including the Rust
 * `#[serde(rename = "type")]` on `identity_type` - the wire (and this
 * interface) uses the bare key `type`. `events`/`alerts` are integer
 * COUNTS, not the underlying objects (idryx `server.go:200-201`) - render
 * them as counts, never as if they were lists. `attestation` is
 * deliberately NOT a field here at all: idryx has none (see
 * `attestation_missing`/`bom_incomplete` alerts instead). */
export interface IdryxIdentity {
  id: string;
  /** `human | service_account | key | agent | mcp_server`. Not a closed
   * enum on the wire - an unrecognized value must still render. */
  type: string;
  privileged: boolean;
  /** e.g. `aws_iam`, `gcp_iam`, `agents`, `mcp`, `okta`, `tokenfuse`, `wardryx`. */
  source: string;
  owner: string;
  /** `"YYYY-MM-DD HH:MM:SS UTC"` when known, else `""` (a different format
   * from `IdryxAlert.time`). */
  created: string;
  last_used: string;
  runtime: string;
  /** The delegation chain, root-first, max depth 32. */
  on_behalf_of: string[];
  permissions: IdryxPermission[];
  remediation: IdryxRemediation | null;
  rotation: IdryxRemediation | null;
  events: number;
  alerts: number;
}

/** Mirrors `genaryx_connectors::IdryxAlert` (`apiAlert`/`jsonAlert`, idryx
 * `server.go:50-56` / `report.go:48-56` - byte-identical shape for both the
 * REST snapshot and a `detect --format json` Rescan). */
export interface IdryxAlert {
  /** One of the 21 detector ids - see `DETECTOR_IDS` below. */
  detector: string;
  /** The identity id this alert is about (joins to `IdryxIdentity.id`). */
  identity: string;
  /** `critical | high | medium | low | info | none`. Dynamic per detector,
   * so filter on `detector` AND `severity`, never a hard-coded mapping. */
  severity: string;
  /** `"YYYY-MM-DDTHH:MM:SSZ"` (UTC, no fractional). */
  time: string;
  /** Free text. For `attestation_missing` this embeds `attestation=<value>` -
   * the only place attestation status reaches this console. */
  summary: string;
}

/** Mirrors `genaryx_connectors::IdryxRecommendation` (`apiRecommendation`,
 * idryx `server.go:111-117`). */
export interface IdryxRecommendation {
  identity: string;
  kind: string;
  explanation: string;
  code: string;
  created_at: string;
}

/** The five identity types idryx's `GET /api/identities` emits (an empty
 * `type` on the wire is defaulted server-side to `"human"` before it ever
 * reaches this console - `server.go:163-166`). Not a closed set on this
 * side either: a value outside this list still renders, just without a
 * dedicated color/label - mirrors `SeverityBadge`'s "never look more
 * assured than the data actually is" tolerance for an unrecognized
 * `severity`. */
export const IDENTITY_TYPES: readonly string[] = [
  "human",
  "service_account",
  "key",
  "agent",
  "mcp_server",
];

/** The 21 detector ids, exact order from docs/PHASE3.md's grounded
 * contract (`cmd/idryx/main.go:314-336`) - the Alerts section's detector
 * filter. */
export const DETECTOR_IDS: readonly string[] = [
  "impossible_travel",
  "mfa_fatigue",
  "new_device",
  "behavior_anomaly",
  "stale_nhi",
  "over_privileged_nhi",
  "orphaned_nhi",
  "excessive_agency",
  "shadow_ai",
  "least_privilege",
  "privilege_escalation",
  "shared_credential",
  "shadow_mcp",
  "agent_shadow_tool",
  "runaway_agent",
  "attestation_missing",
  "bom_incomplete",
  "data_exfiltration",
  "tainted_agent",
  "mcp_drift",
  "unmanaged_egress",
];

/** The two detectors that carry attestation status as free text (idryx has
 * no structured attestation field at all - docs/PHASE3.md: "the Identity
 * panel surfaces attestation status via the `attestation_missing`/
 * `bom_incomplete` alerts"). */
export const ATTESTATION_DETECTORS: ReadonlySet<string> = new Set([
  "attestation_missing",
  "bom_incomplete",
]);
