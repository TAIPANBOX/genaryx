/**
 * Onboard wizard wire types (docs/ONBOARD.md, D15/B2). Mirrors the Rust DTOs
 * in `genaryx_api::onboard::commands` field-for-field (same convention
 * `identityTypes.ts`/`evidenceTypes.ts` follow for their own panels).
 *
 * Unlike most other panels, `OnboardStatus` is a flat struct, not a tagged
 * `state` union: onboard has no connection to establish (no Cloud pairing,
 * no idryx server) - it re-reads the local filesystem fresh on every call,
 * so there is no "bootstrapping"/"unreachable" phase for a tag to name. A
 * missing backend surfaces as a thrown `OnboardError` instead (see
 * `lib/onboard.ts`'s `NO_ENVIRONMENT_ERROR`).
 */

/** Mirrors `onboard::commands::UnitOptionDto` - one business unit already in
 * the identity map, for the form's unit picker. */
export interface UnitOption {
  id: string;
  name: string | null;
  budget_usd_month: number | null;
}

/** Mirrors `onboard::commands::ProvisionedDto` - one already-provisioned
 * passport found in the passports dir. `in_map` is whether any `keys[].agents`
 * pattern in the loaded map matches this passport's id (literal or
 * trailing-`*` prefix) - "seen live traffic yet" is deliberately not here
 * (needs the Cloud; a named follow-up). */
export interface Provisioned {
  agent_id: string;
  owner: string;
  file: string;
  in_map: boolean;
}

/** Mirrors `onboard::commands::SkippedDto` - a passport file that could not
 * be parsed, with an honest reason. Never fails the listing: an unparseable
 * file lands here instead of the whole `onboard_status` call failing. */
export interface Skipped {
  file: string;
  reason: string;
}

/** Mirrors `onboard::commands::OnboardStatusDto`. */
export interface OnboardStatus {
  /** The identity map consulted: explicit arg, else the console process's
   * `TOKENFUSE_IDENTITY_MAP` env var, else `null`. */
  map_path: string | null;
  map_loaded: boolean;
  /** Parse/read problem, when `map_path` exists but is unusable. The wizard
   * still works (free-text unit) - honest, never fatal. */
  map_error: string | null;
  /** Units from the map for the picker (empty when no map). */
  units: UnitOption[];
  /** The staging dir consulted: explicit arg, else `$TAIPAN_HOME/passports`,
   * else `~/.taipan/passports`. Reported even when it does not exist yet. */
  passports_dir: string;
  passports: Provisioned[];
  skipped: Skipped[];
}

/** `onboard_status`'s optional overrides - mirrors
 * `onboard::commands::OnboardStatusRequest`. */
export interface OnboardStatusRequest {
  map_path: string | null;
  passports_dir: string | null;
}

/** The five attestation methods `onboard_generate` accepts (docs/ONBOARD.md) -
 * the form's attestation method `<select>` options, in the order shown. */
export const ATTESTATION_METHODS: readonly string[] = [
  "none",
  "oidc",
  "spiffe-svid",
  "enclave-key",
  "mtls-cert",
];

/** The two modes a declared filesystem scope may carry. */
export type FsScopeMode = "read" | "write";

/** Mirrors `onboard::commands::FsScopeDto` - one folder the agent may access,
 * plus the mode it may access it in. Declaration-only: the passport carries
 * this as information, it is never an enforced mount (docs/ONBOARD.md). */
export interface FsScope {
  path: string;
  mode: FsScopeMode;
}

/** Mirrors `onboard::commands::OnboardGenerateRequest` field-for-field. */
export interface OnboardGenerateRequest {
  trust_domain: string;
  path: string;
  unit: string;
  owner: string;
  display_name: string | null;
  runtime: string | null;
  attestation_method: string | null;
  /** Default when omitted: `path` with `/` -> `-`. */
  key_id: string | null;
  /** Default when omitted: the exact agent id. May end with one trailing `*`. */
  bind_pattern: string | null;
  require_human_above_usd: number | null;
  /** Only used when `unit` is NEW to the map. */
  unit_budget_usd_month: number | null;
  /** Folders this agent may access, each with a read or write mode. Empty is
   * the common case (no filesystem scopes declared). */
  filesystem: FsScope[];
  map_path: string | null;
  passports_dir: string | null;
}

/** Mirrors `onboard::commands::OnboardBundleDto` - `onboard_generate`'s
 * successful result. Propose, never mutate: nothing on disk changes from
 * this call alone - only a later, separate `onboard_write_passport` call
 * writes the one file this wizard is allowed to write. */
export interface OnboardBundle {
  /** `agent://<trust_domain>/<path>`. */
  agent_id: string;
  /** Pretty JSON, schema `taipanbox.dev/agent-passport/v0.1`. */
  passport_json: string;
  /** `<passports_dir>/<path with '/' -> '-'>.json`. */
  passport_path: string;
  /** Minted `gx_<32 hex>`, shown ONCE, never persisted by this console. */
  client_key_secret: string;
  /** `"<secret>:<key_id>"` - the line to append to `TOKENFUSE_CLIENT_KEYS`. */
  client_keys_line: string;
  key_id: string;
  /** Pretty JSON: the `keys` entry (+ a `units` entry when `unit_is_new`). */
  identity_map_fragment: string;
  unit_is_new: boolean;
  /** YAML. */
  wardryx_policy_stub: string;
  /** `taipan_agent_passport` + `taipan_wardryx_policy`. */
  terraform_snippet: string;
}

/** `onboard_write_passport`'s args - mirrors
 * `onboard::commands::OnboardWritePassportRequest`. */
export interface OnboardWritePassportRequest {
  passport_json: string;
  passport_path: string;
  passports_dir: string | null;
  overwrite: boolean;
}

/** Mirrors `onboard::commands::OnboardWriteDto` - `onboard_write_passport`'s
 * successful result. */
export interface OnboardWriteResult {
  written_path: string;
  created_dir: boolean;
}

/** Mirrors `onboard::commands::OnboardError`. Unlike most other planes'
 * tagged error unions (a distinct shape per variant), every onboard error
 * carries the SAME two fields - `kind` is left as an open `string` rather
 * than a closed literal union, since the plane's own validation vocabulary
 * is not fully enumerated in this console-side contract, and an
 * unrecognized value must still render (mirrors `IdryxIdentity.type`'s same
 * "not a closed enum on the wire" tolerance).
 *
 * `"io"` is the one value this UI branches on explicitly (existing-file
 * detection for the Overwrite confirm, see `OnboardView.tsx`). `"no_environment"`
 * is a console-side-only value this UI synthesizes when there is no backend
 * to call at all - the plane itself never sends it (see `lib/onboard.ts`'s
 * `NO_ENVIRONMENT_ERROR`). */
export interface OnboardError {
  kind: string;
  message: string;
}
