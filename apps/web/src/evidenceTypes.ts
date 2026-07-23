/**
 * Evidence Center wire types. Mirrors the Rust DTOs in
 * `crates/api/src/evidence/commands.rs` field-for-field (same convention
 * `drillsTypes.ts`/`cryptoTypes.ts` follow for their own panels).
 *
 * `EvidenceManifest`/`ManifestArtifact`/`MissingSource` mirror
 * `genaryx_core::evidence::{EvidenceManifest,ManifestArtifact,MissingSource}`
 * directly - those Rust types derive `Serialize` with NO `rename_all` (plain
 * struct field names), so their wire shape is snake_case verbatim, same as
 * every other core-owned DTO in this app.
 */

/** Mirrors `evidence::commands::EvidenceStatusDto`
 * (`#[serde(tag = "state", rename_all = "snake_case")]`). Says NOTHING about
 * Cloud availability on purpose - the panel combines this with a separate
 * `money_status` read for that (see the Rust module's doc comment). */
export type EvidenceStatus =
  | { state: "bootstrapping" }
  | {
      state: "ready";
      qryx_available: boolean;
      qryx_bin: string | null;
      qryx_default_target: string | null;
      idryx_available: boolean;
      idryx_bin: string | null;
      idryx_load_sources: string[];
      tokenfuse_available: boolean;
      tokenfuse_bin: string | null;
      tokenfuse_default_traces_dir: string | null;
    };

/** Mirrors `evidence::commands::EvidenceError` (`#[serde(tag = "kind", rename_all = "snake_case")]`). */
export type EvidenceError = { kind: "bootstrapping" } | { kind: "build"; message: string };

/** Mirrors `evidence::commands::EvidenceBuildRequest` - what the "Build
 * evidence pack" button sends. A blank `qryx_target`/`tokenfuse_traces_dir`
 * means "use the resolved environment's own default", same convention
 * `drills_run`'s `api_key`/`save_path` overrides use. */
export interface EvidenceBuildRequest {
  include_cloud: boolean;
  include_qryx: boolean;
  qryx_target: string | null;
  include_idryx: boolean;
  include_tokenfuse: boolean;
  tokenfuse_traces_dir: string | null;
}

/** Mirrors `genaryx_core::evidence::MissingSource` - a source that was
 * requested but could not be included, with an honest reason. */
export interface MissingSource {
  name: string;
  reason: string;
}

/** Mirrors `genaryx_core::evidence::ManifestArtifact` - one artifact's entry
 * in the signed manifest (the bytes themselves live in the downloaded zip,
 * never here). */
export interface ManifestArtifact {
  name: string;
  filename: string;
  content_type: string;
  source: string;
  tool_version: string | null;
  /** The artifact's OWN self-verification status when it self-verifies
   * (Qryx evidence, the Cloud audit-chain verdict), else `null`. */
  verify_status: string | null;
  /** `"sha256:<hex>"`. */
  sha256: string;
  size_bytes: number;
}

/** Mirrors `genaryx_core::evidence::EvidenceManifest` - the pack's
 * tamper-evident index, what the console signs and what the manifest view
 * renders. */
export interface EvidenceManifest {
  pack_version: string;
  /** UTC ISO-8601. */
  generated_at: string;
  operator: string;
  org: string;
  artifacts: ManifestArtifact[];
  /** Sources explicitly NOT included - the "honest partial pack" list,
   * rendered as its own clearly-separate section from `artifacts`. */
  missing: MissingSource[];
}

/** Mirrors `evidence::commands::EvidenceBuildDto` - `evidence_build`'s
 * successful result. `cloud_included`/`journal_error` are honest additions
 * beyond the bare minimum (mirrors `MutationOutcome`'s
 * `bus_recorded`/`bus_error` pairing): `cloud_included` may be `false` even
 * when the operator checked the Cloud box, if Money was not paired at build
 * time - see the Rust module's doc comment. */
export interface EvidenceBuildResult {
  zip_base64: string;
  filename: string;
  manifest: EvidenceManifest;
  signed: boolean;
  cloud_included: boolean;
  journaled: boolean;
  journal_error: string | null;
}
