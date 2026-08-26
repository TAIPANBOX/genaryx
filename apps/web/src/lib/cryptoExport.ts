/**
 * What the Crypto, Quality and Routines panels SHOW and what they let an
 * operator take away, as pure functions.
 *
 * # WHY THIS MODULE EXISTS AT ALL
 *
 * Two of the three NCSC milestones arrive from qryx with their own finding
 * list attached (`NcscPriority.findings`, `NcscFullMigration.findings`) and
 * the console rendered a count for them and dropped the list. That is not a
 * missing feature, it is a field on the wire that never reached the screen,
 * and the reason it survived is that the decision lived inside JSX where
 * nothing could assert on it. It lives here now, so a test can.
 *
 * The same applies to every export below. A file that leaves this console is
 * a document (see `lib/download.ts`'s own header for why the provenance block
 * is mandatory), and the interesting part of building one is not the CSV
 * quoting, it is deciding what each empty cell MEANS. That decision is pure,
 * so it is testable, so it is here rather than in a click handler.
 *
 * # THE RULE THIS MODULE KEEPS
 *
 * A field that is absent, empty or unrecorded is reported as unrecorded. It
 * is never rendered or exported as `0`, `""` or `-` where those read as a
 * measurement. CLAUDE.md invariant 4 is about fabricated ROWS; this is the
 * same instinct one field down, and `lib/download.ts` already takes the other
 * half of it (`null`/`undefined` become an EMPTY cell, never "0").
 */
import type { EvidenceReport, NcscFinding, NcscReport } from "../cryptoTypes";
import type { VerdryxBaseline, VerdryxRunSummary } from "../qualityTypes";
import type { RoutineRunDto, RoutinesHistoryDto } from "../routinesTypes";
import type { ExportMeta } from "./download";

// ============================================================================
// The three NCSC milestones, each with the list behind its count
// ============================================================================

export type MilestoneKey = "discovery2028" | "highestPriority2031" | "fullMigration2035";

/** One NCSC milestone as the panel shows it: the count qryx reported, the
 * verdict, and the finding list qryx carried WITH that count. */
export interface MilestoneView {
  key: MilestoneKey;
  /** Tab label, short enough for a row of three. */
  label: string;
  /** The milestone's own count field, exactly as qryx reported it. Never
   * derived from `findings.length`: they answer different questions (the
   * 2028 count is occurrence-based, the list is per algorithm+asset type). */
  count: number;
  /** The word this milestone's count counts, so a tile never implies the
   * three milestones are measuring the same thing. */
  countNoun: string;
  verdict: string;
  findings: NcscFinding[];
  /** What an EMPTY `findings` means for this milestone, given its own count.
   * `null` whenever there is a list to show. */
  emptyNote: string | null;
}

/**
 * PRE-FIX BODY, kept deliberately: this is what `CryptoView.tsx` does today,
 * transcribed so the suite can go red against the real behaviour rather than
 * against a stub nobody ever shipped. `CryptoView.tsx:185` passes exactly
 * `ncsc.discovery2028.quantumVulnerableFindings` to the findings table and
 * nothing else, so exactly one milestone is returned here.
 */
export function milestoneViews(report: NcscReport): MilestoneView[] {
  return [
    {
      key: "discovery2028",
      label: "2028 · discovery",
      count: report.discovery2028.quantumVulnerableCount,
      countNoun: "quantum-vulnerable",
      verdict: report.discovery2028.verdict,
      findings: report.discovery2028.quantumVulnerableFindings,
      emptyNote: null,
    },
  ];
}

// ============================================================================
// Report provenance: which standard, generated when, over what root
// ============================================================================

/** One "here is where this came from" line. `missing` is the honest half:
 * the wire type says `string`, and an EMPTY string is qryx not recording the
 * field, not a value. */
export interface ProvenanceLine {
  label: string;
  value: string;
  missing: boolean;
}

/** PRE-FIX BODY: no part of `NcscReport.standard`/`generatedAt`/`root`
 * reaches the screen today. */
export function ncscProvenance(_report: NcscReport): ProvenanceLine[] {
  return [];
}

/** PRE-FIX BODY: `EvidenceReport.standard`/`generatedAt`/`root`/`tool`/
 * `version` are all dropped by `CryptoEvidence.tsx` today. */
export function evidenceProvenance(_report: EvidenceReport): ProvenanceLine[] {
  return [];
}

// ============================================================================
// Evidence: the summary's total and its by-severity breakdown
// ============================================================================

export interface SeverityCount {
  severity: string;
  count: number;
}

/** PRE-FIX BODY: `EvidenceSummary.bySeverity` reaches no component. */
export function evidenceSeverityRows(_report: EvidenceReport): SeverityCount[] {
  return [];
}

/** PRE-FIX BODY: with nothing rendering the breakdown, nothing says what its
 * absence means either. */
export function evidenceSeverityNote(_report: EvidenceReport): string | null {
  return null;
}

// ============================================================================
// CBOM: the tolerant read, shared by the table and the export
// ============================================================================

/** A tolerant, partial view of one CycloneDX 1.6 `components[]` entry
 * (`--format cbom`'s crypto extension). Every field optional: this console
 * renders the CBOM, it does not validate it against the external schema. */
export interface CbomComponentLike {
  name?: string;
  type?: string;
  version?: string;
  cryptoProperties?: {
    assetType?: string;
    algorithmProperties?: {
      primitive?: string;
      parameterSetIdentifier?: string;
    };
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Best-effort extraction of `value.components[]`, tolerant of anything that
 * does not match the expected CycloneDX shape. Never throws, never assumes:
 * an unexpected top-level shape yields an empty list, which the table renders
 * as "no components found" rather than crashing.
 *
 * Lives here rather than in `CryptoCbomTable.tsx` so the table and the export
 * read the CBOM through the SAME function. Two tolerant readers of an untyped
 * document drift, and then the file says something the screen does not. */
export function cbomComponents(value: unknown): CbomComponentLike[] {
  if (!isRecord(value)) return [];
  const components = value.components;
  if (!Array.isArray(components)) return [];
  return components.filter(isRecord) as CbomComponentLike[];
}

// ============================================================================
// Export rows
//
// Every row type below uses `null` for "not recorded". `lib/download.ts`
// writes that as an EMPTY cell, which is the whole point: an empty cell reads
// as "nobody recorded this", a `0` reads as "somebody measured zero".
// ============================================================================

export interface FindingExportRow {
  milestone: string;
  algorithm: string;
  type: string;
  severity: string;
  occurrences: number;
  locations: string | null;
  externally_facing: boolean;
  long_lived_data: boolean;
  planned: boolean;
}

export const FINDING_EXPORT_COLUMNS: { key: keyof FindingExportRow & string; header: string }[] = [
  { key: "milestone", header: "milestone" },
  { key: "algorithm", header: "algorithm" },
  { key: "type", header: "type" },
  { key: "severity", header: "severity" },
  { key: "occurrences", header: "occurrences" },
  { key: "locations", header: "locations" },
  { key: "externally_facing", header: "externally_facing" },
  { key: "long_lived_data", header: "long_lived_data" },
  { key: "planned", header: "planned" },
];

/** PRE-FIX BODY: no findings export exists, so zero rows leave this console. */
export function findingExportRows(_report: NcscReport): FindingExportRow[] {
  return [];
}

export interface CbomExportRow {
  name: string | null;
  type: string | null;
  version: string | null;
  asset_type: string | null;
  primitive: string | null;
  parameter_set: string | null;
}

export const CBOM_EXPORT_COLUMNS: { key: keyof CbomExportRow & string; header: string }[] = [
  { key: "name", header: "component" },
  { key: "type", header: "type" },
  { key: "version", header: "version" },
  { key: "asset_type", header: "crypto_asset_type" },
  { key: "primitive", header: "primitive" },
  { key: "parameter_set", header: "parameter_set" },
];

/** PRE-FIX BODY: no CBOM export exists. */
export function cbomExportRows(_value: unknown): CbomExportRow[] {
  return [];
}

export interface QualityRunExportRow {
  run_id: string;
  model: string;
  started_at: string;
  finished_at: string | null;
  case_count: number;
  mean_score: number | null;
  total_tokens: number;
  total_cost_usd: number;
}

export const QUALITY_RUN_EXPORT_COLUMNS: { key: keyof QualityRunExportRow & string; header: string }[] = [
  { key: "run_id", header: "run" },
  { key: "model", header: "model" },
  { key: "started_at", header: "started" },
  { key: "finished_at", header: "finished" },
  { key: "case_count", header: "cases" },
  { key: "mean_score", header: "mean_score" },
  { key: "total_tokens", header: "total_tokens" },
  { key: "total_cost_usd", header: "total_cost_usd" },
];

/** PRE-FIX BODY: no eval-runs export exists. */
export function qualityRunExportRows(_runs: VerdryxRunSummary[]): QualityRunExportRow[] {
  return [];
}

export interface BaselineExportRow {
  label: string | null;
  baseline_id: string;
  eval_run_id: string;
  source_run_model: string | null;
  mean_score: number;
  created_at: string;
}

export const BASELINE_EXPORT_COLUMNS: { key: keyof BaselineExportRow & string; header: string }[] = [
  { key: "label", header: "label" },
  { key: "baseline_id", header: "baseline_id" },
  { key: "eval_run_id", header: "eval_run_id" },
  { key: "source_run_model", header: "source_run_model" },
  { key: "mean_score", header: "mean_score" },
  { key: "created_at", header: "created" },
];

/** PRE-FIX BODY: no baselines export exists. */
export function baselineExportRows(
  _baselines: VerdryxBaseline[],
  _runs: VerdryxRunSummary[] | null,
): BaselineExportRow[] {
  return [];
}

export interface RoutineHistoryExportRow {
  routine: string;
  status: string;
  started_at: string;
  finished_at: string;
  exit_code: number;
  reason: string | null;
  artifact: string | null;
  summary: string | null;
  schema: string;
}

export const ROUTINE_HISTORY_EXPORT_COLUMNS: { key: keyof RoutineHistoryExportRow & string; header: string }[] = [
  { key: "routine", header: "routine" },
  { key: "status", header: "status" },
  { key: "started_at", header: "started" },
  { key: "finished_at", header: "finished" },
  { key: "exit_code", header: "exit_code" },
  { key: "reason", header: "reason" },
  { key: "artifact", header: "artifact" },
  { key: "summary", header: "summary" },
  { key: "schema", header: "schema" },
];

/** PRE-FIX BODY: no routines-history export exists. */
export function routineHistoryExportRows(_records: RoutineRunDto[]): RoutineHistoryExportRow[] {
  return [];
}

// ============================================================================
// Provenance blocks
//
// `ExportMeta.caveats` is the part that earns the file. A table that merges
// two windows, is capped, or is a partial projection of a bigger document has
// to say so, or it reads as complete and is not.
// ============================================================================

function baseMeta(subject: string, takenAt: string, environment: string): ExportMeta {
  return { subject, environment, takenAt, windows: [], caveats: [] };
}

/** PRE-FIX BODY: no export, so no provenance block either. */
export function findingExportMeta(_report: NcscReport, takenAt: string, environment: string): ExportMeta {
  return baseMeta("Genaryx crypto findings", takenAt, environment);
}

/** PRE-FIX BODY. */
export function cbomExportMeta(
  _value: unknown,
  _scanTarget: string,
  takenAt: string,
  environment: string,
): ExportMeta {
  return baseMeta("Genaryx CBOM inventory", takenAt, environment);
}

/** PRE-FIX BODY. */
export function qualityRunExportMeta(
  _runs: VerdryxRunSummary[],
  _dbPath: string,
  takenAt: string,
  environment: string,
): ExportMeta {
  return baseMeta("Genaryx eval runs", takenAt, environment);
}

/** PRE-FIX BODY. */
export function baselineExportMeta(
  _baselines: VerdryxBaseline[],
  _runs: VerdryxRunSummary[] | null,
  _dbPath: string,
  takenAt: string,
  environment: string,
): ExportMeta {
  return baseMeta("Genaryx quality baselines", takenAt, environment);
}

/** PRE-FIX BODY. */
export function routineHistoryExportMeta(
  _routine: string,
  _history: RoutinesHistoryDto,
  takenAt: string,
  environment: string,
): ExportMeta {
  return baseMeta("Genaryx routine history", takenAt, environment);
}
