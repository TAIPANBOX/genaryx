/**
 * Crypto wire types. Mirrors the Rust DTOs in
 * `crates/api/src/crypto/commands.rs` field-for-field (same convention
 * `identityTypes.ts`/`qualityTypes.ts` follow for their own panels).
 *
 * `NcscReport`/`NcscDiscovery`/`NcscPriority`/`NcscFullMigration`/`NcscFinding`/
 * `EvidenceReport`/`EvidenceSummary`/`QryxSignature`/`VerifyOutcome` mirror
 * `genaryx_connectors::{NcscReport,...}` (`crates/connectors/src/qryx.rs`)
 * directly - those Rust types already derive `Serialize` (with
 * `#[serde(rename_all = "camelCase")]`/explicit renames matching qryx's own
 * Go JSON tags) and cross the genaryx-web JSON boundary as-is, so these interfaces
 * exist only so the frontend has names/types for the exact same
 * camelCase-on-the-wire shape, not because the Rust side re-wraps anything.
 */

/** Mirrors `crypto::commands::CryptoStatusDto` (`#[serde(tag = "state", rename_all = "snake_case")]`).
 * No `unreachable` variant: qryx has no service to confirm reachable at
 * bootstrap (no serve process, no healthz) - see `crypto::state`'s doc
 * comment. */
export type CryptoStatus =
  | { state: "bootstrapping" }
  | { state: "no_environment" }
  | { state: "ready"; qryx_bin: string; default_target: string };

/** Mirrors `crypto::commands::CryptoError` (`#[serde(tag = "kind", rename_all = "snake_case")]`). */
export type CryptoError =
  | { kind: "bootstrapping" }
  | { kind: "no_environment" }
  | { kind: "qryx"; message: string };

/** Mirrors `genaryx_connectors::NcscFinding` (`ncscFindingJSON`, `ncsc.go:168-178`) -
 * one quantum-vulnerable asset. */
export interface NcscFinding {
  algorithm: string;
  /** Asset type, e.g. `public-key`, `certificate`. */
  type: string;
  severity: string;
  occurrences: number;
  locations: string[];
  externallyFacing: boolean;
  longLivedData: boolean;
  planned: boolean;
}

/** Mirrors `genaryx_connectors::NcscDiscovery` - the 2028 "complete
 * discovery" milestone. */
export interface NcscDiscovery {
  /** `on-track` | `at-risk` | `not-started`. */
  verdict: string;
  coverageBySource: Record<string, number>;
  totalInventoried: number;
  quantumVulnerableCount: number;
  migrationPlanExists: boolean;
  migrationPlanNote: string;
  quantumVulnerableFindings: NcscFinding[];
}

/** Mirrors `genaryx_connectors::NcscPriority` - the 2031 "highest-priority
 * systems" milestone. */
export interface NcscPriority {
  verdict: string;
  criteria: string;
  count: number;
  /** Always `0`: qryx tracks no cross-run remediation state (see
   * `genaryx_connectors::qryx::NcscPriority::migrated_count`'s doc comment
   * in `crates/connectors/src/qryx.rs`). Render honestly - never as real
   * migration progress. */
  migratedCount: number;
  remainingCount: number;
  note: string;
  findings: NcscFinding[];
}

/** Mirrors `genaryx_connectors::NcscFullMigration` - the 2035 "all systems"
 * milestone. */
export interface NcscFullMigration {
  verdict: string;
  count: number;
  findings: NcscFinding[];
}

/** Mirrors `genaryx_connectors::NcscReport` - the whole PQC migration
 * timeline (`--format ncsc`), the Crypto panel's hero. */
export interface NcscReport {
  standard: string;
  generatedAt: string;
  root: string;
  discovery2028: NcscDiscovery;
  highestPriority2031: NcscPriority;
  fullMigration2035: NcscFullMigration;
}

/** Mirrors `genaryx_connectors::QryxSignature` (`attest.Signature`). `alg` is
 * one of `ed25519`, `ecdsa-p256`, or `ml-dsa-44|65|87`. */
export interface QryxSignature {
  alg: string;
  value: string;
  publicKey: string;
}

/** Mirrors `genaryx_connectors::EvidenceSummary` - the compliance rollup in
 * an evidence report. */
export interface EvidenceSummary {
  compliant: number;
  nonCompliant: number;
  issues: number;
  total: number;
  /** Integer percent. */
  scorePct: number;
  bySeverity: Record<string, number>;
}

/** Mirrors `genaryx_connectors::EvidenceReport` (`--format evidence`) - a
 * CNSA 2.0 compliance attestation with a self-verifying digest and an
 * optional detached signature. `assets` is kept untyped, same as the Rust
 * side: a large, display-only CNSA-per-asset shape. */
export interface EvidenceReport {
  tool: string;
  version: string;
  standard: string;
  generatedAt: string;
  root: string;
  summary: EvidenceSummary;
  assets: unknown[];
  /** `"sha256:<hex>"`. */
  digest: string;
  /** Present only when built with a signing key - always `null` for W1
   * (unsigned bundles only). */
  signature: QryxSignature | null;
}

/** Mirrors `genaryx_connectors::VerifyOutcome` (`qryx verify-evidence`'s
 * result). `verified: false` is a real "not verified" answer, not an
 * error. */
export interface VerifyOutcome {
  verified: boolean;
  message: string;
}
