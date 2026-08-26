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
 *
 * Invariant 8 is the other half again, and it is why several caveats below
 * are conditional. "2031: 1 in scope" beside an empty finding table is a
 * number that is accurate about itself and false about what was asked. The
 * milestone views and the export provenance blocks say which is which.
 */
import type { EvidenceReport, EvidenceSummary, NcscFinding, NcscReport } from "../cryptoTypes";
import type { VerdryxBaseline, VerdryxRunSummary } from "../qualityTypes";
import type { RoutineRunDto, RoutinesHistoryDto } from "../routinesTypes";
import { SEVERITIES } from "../types";
import type { ExportMeta } from "./download";

/** The text every unrecorded field renders as ON SCREEN. In a FILE the same
 * state is an empty cell instead: `lib/download.ts` writes `null` that way on
 * purpose, and a spreadsheet full of the words "not recorded" would be worse
 * than a blank a reader can filter on. */
const UNRECORDED = "not recorded";

/** A wire string, judged. The Rust DTOs type these as `String` rather than
 * `Option<String>`, so an unrecorded field arrives as `""` and never as
 * `null`: without this, "" renders as a value nobody notices is absent. */
function recorded(raw: string | null | undefined): { value: string; missing: boolean } {
  const trimmed = (raw ?? "").trim();
  return trimmed.length > 0 ? { value: trimmed, missing: false } : { value: UNRECORDED, missing: true };
}

/** The same judgement for a cell that is going into a FILE: unrecorded is
 * `null`, which `toCsv` writes as an empty cell. */
function blank(raw: string | null | undefined): string | null {
  const trimmed = (raw ?? "").trim();
  return trimmed.length > 0 ? trimmed : null;
}

function line(label: string, raw: string | null | undefined): ProvenanceLine {
  const { value, missing } = recorded(raw);
  return { label, value, missing };
}

// ============================================================================
// The three NCSC milestones, each with the list behind its count
// ============================================================================

export type MilestoneKey = "discovery2028" | "highestPriority2031" | "fullMigration2035";

/** One NCSC milestone as the panel shows it: the count qryx reported, the
 * verdict, and the finding list qryx carried WITH that count. */
export interface MilestoneView {
  key: MilestoneKey;
  /** Tab label, matching `CryptoTimeline`'s own milestone card titles so the
   * two readings of the same milestone cannot drift apart on screen. */
  label: string;
  /** The same milestone without the typographic separator, for a file. */
  exportLabel: string;
  /** The milestone's own count field, exactly as qryx reported it. Never
   * derived from `findings.length`: they answer different questions. The
   * connector's own fixture has 2028 counting 2 quantum-vulnerable from ONE
   * finding whose `occurrences` is 2, so equating them would be wrong on the
   * very first report. */
  count: number;
  /** The word this milestone's count counts, so a tile never implies the
   * three milestones are measuring the same thing. */
  countNoun: string;
  verdict: string;
  findings: NcscFinding[];
  /** What an EMPTY `findings` means for this milestone, given its own count.
   * `null` whenever there is a list to show. */
  emptyNote: string | null;
  /** True when the list is MISSING rather than empty: qryx counted something
   * for this milestone and carried no findings for it. Both render an empty
   * table and only one of them is a warning, so the flag is here rather than
   * left for a component to infer from the note's prose. */
  missingList: boolean;
}

function emptyNoteFor(count: number, countNoun: string, findings: NcscFinding[]): string | null {
  if (findings.length > 0) return null;
  if (count > 0) {
    // The case this whole module was written for. qryx says 1 system is in
    // scope for 2031 and carries no list of which one. "No findings" would
    // read as a milestone that is clear.
    return `qryx reported ${count} ${countNoun} for this milestone and carried no finding list with it. That is a missing list, not an empty result.`;
  }
  return `qryx reported nothing ${countNoun} for this milestone.`;
}

/**
 * The three NCSC milestones, each carrying the finding list that arrived with
 * it. `discovery2028.quantumVulnerableFindings` was the only one the console
 * ever rendered; `highestPriority2031.findings` and
 * `fullMigration2035.findings` are on the same wire (see
 * `crates/connectors/src/qryx.rs`, both `#[serde(default,
 * deserialize_with = "crate::null_default")] Vec<NcscFinding>`) and reached
 * no component.
 */
export function milestoneViews(report: NcscReport): MilestoneView[] {
  const d = report.discovery2028;
  const p = report.highestPriority2031;
  const f = report.fullMigration2035;
  return [
    {
      key: "discovery2028",
      label: "2028 · complete discovery",
      exportLabel: "2028 complete discovery",
      count: d.quantumVulnerableCount,
      countNoun: "quantum-vulnerable",
      verdict: d.verdict,
      findings: d.quantumVulnerableFindings,
      emptyNote: emptyNoteFor(d.quantumVulnerableCount, "quantum-vulnerable", d.quantumVulnerableFindings),
      missingList: d.quantumVulnerableFindings.length === 0 && d.quantumVulnerableCount > 0,
    },
    {
      key: "highestPriority2031",
      label: "2031 · highest-priority",
      exportLabel: "2031 highest-priority systems",
      count: p.count,
      countNoun: "in scope",
      verdict: p.verdict,
      findings: p.findings,
      emptyNote: emptyNoteFor(p.count, "in scope", p.findings),
      missingList: p.findings.length === 0 && p.count > 0,
    },
    {
      key: "fullMigration2035",
      label: "2035 · full migration",
      exportLabel: "2035 full migration",
      count: f.count,
      countNoun: "in scope",
      verdict: f.verdict,
      findings: f.findings,
      emptyNote: emptyNoteFor(f.count, "in scope", f.findings),
      missingList: f.findings.length === 0 && f.count > 0,
    },
  ];
}

// ============================================================================
// Report provenance: which standard, generated when, over what root
// ============================================================================

/** One "here is where this came from" line. `missing` is the honest half. */
export interface ProvenanceLine {
  label: string;
  value: string;
  missing: boolean;
}

/**
 * `NcscReport.standard`, `generatedAt` and `root`, none of which reached the
 * screen. `generatedAt` is the one that changes a decision: the panel's own
 * freshness badge times the CLICK, and a scan of a checkout that has not
 * moved in three weeks is a three-week-old posture presented as current.
 * `root` is the second: the input box holds what was TYPED, `root` is what
 * qryx resolved and actually walked.
 */
export function ncscProvenance(report: NcscReport): ProvenanceLine[] {
  return [
    line("standard", report.standard),
    line("generated at", report.generatedAt),
    line("scanned root", report.root),
  ];
}

/** The same three for an evidence bundle, plus which build of qryx made it -
 * the bundle is an attestation, and "which tool version signed off" is part
 * of what it attests. */
export function evidenceProvenance(report: EvidenceReport): ProvenanceLine[] {
  const tool = [blank(report.tool), blank(report.version)].filter((s): s is string => s !== null).join(" ");
  return [
    line("built by", tool),
    line("standard", report.standard),
    line("generated at", report.generatedAt),
    line("scanned root", report.root),
  ];
}

// ============================================================================
// Evidence: the summary's by-severity breakdown
// ============================================================================

export interface SeverityCount {
  severity: string;
  count: number;
}

/** Worst first, using the console's own severity ladder. A severity outside
 * that ladder is kept and sorted after the known ones rather than dropped -
 * the same tolerance `SeverityBadge` already keeps, since the ladder is never
 * closed on the wire. */
function severityRank(severity: string): number {
  const idx = (SEVERITIES as readonly string[]).indexOf(severity);
  return idx === -1 ? SEVERITIES.length : SEVERITIES.length - 1 - idx;
}

function bySeverityOf(summary: EvidenceSummary): Record<string, number> {
  const raw: unknown = summary.bySeverity;
  return typeof raw === "object" && raw !== null && !Array.isArray(raw) ? (raw as Record<string, number>) : {};
}

/** `EvidenceSummary.bySeverity`, which reached no component. It is the
 * triage order for the non-compliant assets, so without it "2 non-compliant"
 * says nothing about whether tomorrow is soon enough. */
export function evidenceSeverityRows(report: EvidenceReport): SeverityCount[] {
  return Object.entries(bySeverityOf(report.summary))
    .map(([severity, count]) => ({ severity, count }))
    .sort((a, b) => severityRank(a.severity) - severityRank(b.severity) || a.severity.localeCompare(b.severity));
}

/**
 * What an EMPTY breakdown means, which is the whole reason it is worth
 * rendering. An empty severity table beside "2 non-compliant" reads as
 * nothing severe, and that is a conclusion the report did not support.
 */
export function evidenceSeverityNote(report: EvidenceReport): string | null {
  if (Object.keys(bySeverityOf(report.summary)).length > 0) return null;
  const nonCompliant = report.summary.nonCompliant;
  if (nonCompliant > 0) {
    return `qryx carried no severity breakdown with this report, so its ${nonCompliant} non-compliant asset(s) are unattributed here. That is a missing breakdown, not a clean one.`;
  }
  return "No severity breakdown: this report records nothing non-compliant to break down.";
}

// ============================================================================
// Evidence: the per-asset CNSA rows
//
// `EvidenceReport.assets` reached no component at all, and the connector's own
// doc comment for the field says the opposite: "a large, display-only shape
// the panel renders as a table" (`crates/connectors/src/qryx.rs`). It did not.
// The rows are the actionable half of the bundle - the summary says 2 assets
// are non-compliant, these say WHICH two, by when, and what qryx says to do.
// ============================================================================

/** A tolerant, partial view of one `assets[]` row (qryx's `cnsaAssetJSON`,
 * `internal/report/cnsa.go:255-264`). Every field optional for the same
 * reason [`CbomComponentLike`]'s are: the row crosses the backend as raw
 * JSON (`EvidenceReport.assets` is `Vec<serde_json::Value>`), so nothing on
 * either side checked that any of these are present. */
export interface EvidenceAssetLike {
  algorithm?: string;
  /** Asset type, e.g. `public-key`, `certificate`. */
  type?: string;
  /** `compliant` | `non-compliant` | `issue` | `not-assessed`. */
  status?: string;
  /** The migration deadline qryx assigned: `immediate`, `2027`, `2030`,
   * `2035` or `n/a`. */
  deadline?: string;
  /** qryx's own remediation sentence for this asset. */
  action?: string;
  occurrences?: number;
  locations?: string[];
  tags?: Record<string, string>;
}

/** The bundle's per-asset rows, read tolerantly and in the order they
 * arrived. Same discipline as [`cbomComponents`], and for the same reason:
 * the rows cross the backend as raw JSON (`Vec<serde_json::Value>`), so an
 * entry that is not an object is skipped rather than thrown on, and the table
 * and the export read them through this one function. */
export function evidenceAssets(report: EvidenceReport): EvidenceAssetLike[] {
  const assets: unknown = report.assets;
  if (!Array.isArray(assets)) return [];
  return assets.filter(isRecord) as EvidenceAssetLike[];
}

/**
 * What an EMPTY asset list means, given what the summary claims to have
 * graded. A bundle reporting 127 graded assets beside an empty table reads as
 * an estate with nothing in it, and that is invariant 8's shape again: the
 * total is accurate about itself and false about what was asked.
 */
export function evidenceAssetsNote(report: EvidenceReport): string | null {
  if (evidenceAssets(report).length > 0) return null;
  const total = report.summary.total;
  if (total > 0) {
    return `This bundle grades ${total} asset(s) and carried no per-asset rows with them. That is a missing list, not an empty inventory.`;
  }
  return "This bundle graded no assets, so there are no per-asset rows to show.";
}

/**
 * The assets counted in `total` that are in none of the three counts this
 * console can see.
 *
 * qryx grades every asset into FOUR buckets, not three: `compliant`,
 * `non-compliant`, `issue` and `not-assessed`, and its own source says why the
 * fourth is not cosmetic - "60% compliant out of an inventory this tool graded
 * completely and 60% out of one where a third was never assessed are different
 * facts", and `notAssessed` is inside the score's denominator
 * (`internal/report/cnsa.go`'s `cnsaSummary`). This console's wire type has no
 * field for it (`crates/connectors/src/qryx.rs`'s `EvidenceSummary` carries
 * five counts, not six), so the count itself cannot be recovered here.
 *
 * What CAN be said honestly is the difference, reported as a difference and
 * never as qryx's own figure. A NEGATIVE difference is not folded away either:
 * it means the four numbers do not reconcile, which is a thing about the
 * report worth saying out loud rather than clamping to zero.
 */
export function evidenceUnaccounted(report: EvidenceReport): string | null {
  const s = report.summary;
  const shown = s.compliant + s.nonCompliant + s.issues;
  const residual = s.total - shown;
  if (residual === 0) return null;
  if (residual > 0) {
    return `${residual} of the ${s.total} asset(s) this bundle grades are in none of the three counts above. qryx grades those "not assessed" and keeps them in the score's denominator, but that count does not cross this console's wire type, so this is the difference rather than qryx's own figure.`;
  }
  return `These counts do not reconcile: compliant, non-compliant and issues add to ${shown}, which is more than the ${s.total} this bundle calls its total. Both figures are shown as they arrived; this console does not pick one.`;
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

/** Whether the document had a top-level `components[]` array at all - the
 * difference between "qryx found no crypto components" and "this console
 * looked in a place this document does not have". */
function hasComponentsArray(value: unknown): boolean {
  return isRecord(value) && Array.isArray(value.components);
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

/** Every milestone's findings in one table, each row saying which milestone
 * it came from. Three separate lists on screen, one file: a PQC migration
 * plan is written across all three deadlines, not one per tab. */
export function findingExportRows(report: NcscReport): FindingExportRow[] {
  return milestoneViews(report).flatMap((m) =>
    m.findings.map((f) => ({
      milestone: m.exportLabel,
      algorithm: f.algorithm,
      type: f.type,
      severity: f.severity,
      occurrences: f.occurrences,
      locations: f.locations.length > 0 ? f.locations.join("; ") : null,
      externally_facing: f.externallyFacing,
      long_lived_data: f.longLivedData,
      planned: f.planned,
    })),
  );
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

export function cbomExportRows(value: unknown): CbomExportRow[] {
  return cbomComponents(value).map((c) => ({
    name: blank(c.name),
    type: blank(c.type),
    version: blank(c.version),
    asset_type: blank(c.cryptoProperties?.assetType),
    primitive: blank(c.cryptoProperties?.algorithmProperties?.primitive),
    parameter_set: blank(c.cryptoProperties?.algorithmProperties?.parameterSetIdentifier),
  }));
}

export interface EvidenceAssetExportRow {
  algorithm: string | null;
  type: string | null;
  status: string | null;
  deadline: string | null;
  action: string | null;
  /** `null`, never `0`, for a row that carried no occurrence count: nobody
   * found this asset zero times. */
  occurrences: number | null;
  locations: string | null;
  tags: string | null;
}

export const EVIDENCE_ASSET_EXPORT_COLUMNS: { key: keyof EvidenceAssetExportRow & string; header: string }[] = [
  { key: "algorithm", header: "algorithm" },
  { key: "type", header: "type" },
  { key: "status", header: "cnsa_status" },
  { key: "deadline", header: "deadline" },
  { key: "occurrences", header: "occurrences" },
  { key: "locations", header: "locations" },
  { key: "tags", header: "tags" },
  { key: "action", header: "action" },
];

/** One row per graded asset, in the order the report carried them. This
 * console does not re-sort: qryx's own report is already ordered most urgent
 * first (`buildEntries` sorts by its `deadlineOrder`, then by occurrence
 * count), and a second opinion about urgency computed here would be this
 * console's claim rather than the attestation's. */
export function evidenceAssetExportRows(report: EvidenceReport): EvidenceAssetExportRow[] {
  return evidenceAssets(report).map((a) => ({
    algorithm: blank(a.algorithm),
    type: blank(a.type),
    status: blank(a.status),
    deadline: blank(a.deadline),
    action: blank(a.action),
    occurrences: typeof a.occurrences === "number" ? a.occurrences : null,
    locations: Array.isArray(a.locations) && a.locations.length > 0 ? a.locations.join("; ") : null,
    tags: tagsOf(a),
  }));
}

/** `k=v; k=v`, or `null` for a row with no tags. An empty string here would
 * read as an asset qryx tagged with nothing, which is a different claim. */
function tagsOf(asset: EvidenceAssetLike): string | null {
  const tags = asset.tags;
  if (!isRecord(tags)) return null;
  const pairs = Object.entries(tags).map(([k, v]) => `${k}=${String(v)}`);
  return pairs.length > 0 ? pairs.join("; ") : null;
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

export function qualityRunExportRows(runs: VerdryxRunSummary[]): QualityRunExportRow[] {
  return runs.map((s) => ({
    run_id: s.run.id,
    model: s.run.model,
    started_at: s.run.started_at,
    // Both nulls are carried through rather than filled. A finished_at
    // stamped from the console clock, or a mean_score of 0, would each be a
    // measurement nobody took.
    finished_at: s.run.finished_at,
    case_count: s.case_count,
    mean_score: s.mean_score,
    total_tokens: s.total_tokens,
    total_cost_usd: s.total_cost_usd,
  }));
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

export function baselineExportRows(
  baselines: VerdryxBaseline[],
  runs: VerdryxRunSummary[] | null,
): BaselineExportRow[] {
  return baselines.map((b) => ({
    // The screen shows "(unlabeled)". A file must not: that string is this
    // console's word, not verdryx's, and it would sort and filter as a real
    // label.
    label: blank(b.label),
    baseline_id: b.id,
    eval_run_id: b.eval_run_id,
    source_run_model: blank(runs?.find((r) => r.run.id === b.eval_run_id)?.run.model),
    mean_score: b.mean_score,
    created_at: b.created_at,
  }));
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

export function routineHistoryExportRows(records: RoutineRunDto[]): RoutineHistoryExportRow[] {
  return records.map((r) => ({
    routine: r.routine,
    // The raw recorded value, not `toUiStatus`'s classification. A fifth
    // status a future stack-up writes must survive the round trip out of
    // here; folding it to "unknown" would lose what the box actually wrote.
    status: r.status,
    started_at: r.started_at,
    finished_at: r.finished_at,
    exit_code: r.exit_code,
    reason: blank(r.reason),
    artifact: blank(r.artifact),
    summary: blank(r.summary),
    schema: r.schema,
  }));
}

// ============================================================================
// Provenance blocks
//
// `ExportMeta.caveats` is the part that earns the file. A table that merges
// two windows, is capped, or is a partial projection of a bigger document has
// to say so, or it reads as complete and is not.
// ============================================================================

/** The server's own default in `crates/api/src/routines/commands.rs`
 * (`DEFAULT_HISTORY_LIMIT`). `RoutinesView` passes no `limit`, so this is the
 * number that actually applies. */
const ROUTINE_HISTORY_DEFAULT_LIMIT = 200;

export function findingExportMeta(report: NcscReport, takenAt: string, environment: string): ExportMeta {
  const views = milestoneViews(report);
  const generated = recorded(report.generatedAt);
  const root = recorded(report.root);
  const standard = recorded(report.standard);
  const missingLists = views.filter((m) => m.count > 0 && m.findings.length === 0);

  return {
    subject: "Genaryx crypto findings, all three NCSC milestones",
    environment,
    takenAt,
    windows: [
      `qryx scan --format ncsc over ${root.value}, which qryx generated at ${generated.value}. Not a time window: a scan describes the tree as it stood when it ran.`,
      `standard: ${standard.value}`,
    ],
    caveats: [
      `Each milestone's count is qryx's own figure for that milestone and is NOT the row count here: ${views
        .map((m) => `${m.exportLabel} ${m.count} ${m.countNoun}`)
        .join(", ")}.`,
      ...missingLists.map(
        (m) =>
          `PARTIAL: ${m.exportLabel} reports ${m.count} ${m.countNoun} and carried no finding list, so nothing in this file represents it.`,
      ),
      "occurrences is qryx's own per-finding occurrence count. These rows are one per finding, not one per occurrence, so the rows do not sum to a milestone's count.",
      "An empty locations cell is a finding qryx recorded no location for, not a finding that has none.",
      ...(generated.missing
        ? ["PARTIAL: qryx recorded no generation time on this report, so how old these findings are cannot be read off this file."]
        : []),
    ],
  };
}

export function cbomExportMeta(
  value: unknown,
  scanTarget: string,
  takenAt: string,
  environment: string,
): ExportMeta {
  const target = recorded(scanTarget);
  return {
    subject: "Genaryx CBOM crypto-component inventory",
    environment,
    takenAt,
    windows: [
      `qryx scan --format cbom over ${target.value}, as this console read it at taken_at. Not a time window: a scan describes the tree as it stood when it ran.`,
    ],
    caveats: [
      "This file carries only the fields this console understands from each CycloneDX component: name, type, version, cryptoProperties.assetType, and cryptoProperties.algorithmProperties.primitive / .parameterSetIdentifier. The CBOM qryx produced is a larger CycloneDX document and the rest of it is not here.",
      "The CBOM crosses this console untyped end to end (crypto_scan_cbom returns raw JSON and nothing on either side is a typed contract), so neither the backend nor this file validated it against the CycloneDX schema.",
      "An empty cell is a field this console did not find on that component, not a recorded absence of one.",
      ...(hasComponentsArray(value)
        ? []
        : [
            "PARTIAL: this document had no top-level components[] array where this console looked, so this file has no rows. That is not qryx reporting an empty inventory.",
          ]),
    ],
  };
}

/**
 * The provenance block for the per-asset CNSA rows. This is the one export
 * here that is a piece of an ATTESTATION rather than a view of a store, so it
 * names the digest: without it, a reader cannot tie the file back to the
 * bundle whose signature they might check.
 */
export function evidenceAssetExportMeta(
  report: EvidenceReport,
  takenAt: string,
  environment: string,
): ExportMeta {
  const generated = recorded(report.generatedAt);
  const root = recorded(report.root);
  const digest = recorded(report.digest);
  const rows = evidenceAssets(report).length;
  const unaccounted = evidenceUnaccounted(report);

  return {
    subject: "Genaryx CNSA 2.0 evidence, per-asset rows",
    environment,
    takenAt,
    windows: [
      `qryx scan --format evidence over ${root.value}, which qryx generated at ${generated.value}. Not a time window: a scan describes the tree as it stood when it ran.`,
      `bundle digest ${digest.value}`,
    ],
    caveats: [
      `The bundle grades ${report.summary.total} asset(s) in total: ${report.summary.compliant} compliant, ${report.summary.nonCompliant} non-compliant, ${report.summary.issues} with issues.`,
      ...(unaccounted !== null ? [`PARTIAL: ${unaccounted}`] : []),
      ...(report.summary.total > 0 && rows === 0
        ? [
            `PARTIAL: this bundle grades ${report.summary.total} asset(s) and carried no per-asset rows, so nothing in this file represents them. That is a missing list, not an empty inventory.`,
          ]
        : []),
      "These rows are in the order the bundle carried them and this console does not re-sort them. qryx orders its own report most urgent first, by migration deadline and then by occurrence count.",
      "occurrences is how many places qryx found that asset. The rows are one per asset, not one per occurrence, so they do not sum to it.",
      "An empty cell is a field this console did not find on that row, not a recorded absence of one. The rows cross this console as raw JSON, so neither the backend nor this file checked their shape.",
      report.signature !== null
        ? `This bundle carries a ${report.signature.alg} signature over its digest, made with the public key ${report.signature.publicKey}. This file is a projection of the bundle and is NOT covered by that signature.`
        : "This bundle is unsigned, because this console asks qryx for an unsigned one. That is this console's own request, not something qryx could not do.",
    ],
  };
}

export function qualityRunExportMeta(
  runs: VerdryxRunSummary[],
  dbPath: string,
  takenAt: string,
  environment: string,
): ExportMeta {
  const db = recorded(dbPath);
  return {
    subject: "Genaryx eval runs (Verdryx)",
    environment,
    takenAt,
    windows: [
      `verdryx.db at ${db.value}: its whole eval_runs table as this console read it at taken_at, newest run first. Not a time window and not a cap.`,
    ],
    caveats: [
      "An empty mean_score is a run verdryx recorded no scores for. It is not a score of 0: verdryx computes it as AVG(value), which is NULL over no rows.",
      "total_tokens and total_cost_usd are COALESCE'd sums over the scores verdryx recorded. A run with no scores reports 0 for both, which is an empty sum rather than a measured zero cost.",
      "An empty finished_at is a run verdryx has not recorded as finished, not a zero-length run.",
      `verdryx.db is written by the operator's own verdryx eval runs; this console only reads it, and holds ${runs.length} run(s) at taken_at.`,
    ],
  };
}

export function baselineExportMeta(
  baselines: VerdryxBaseline[],
  runs: VerdryxRunSummary[] | null,
  dbPath: string,
  takenAt: string,
  environment: string,
): ExportMeta {
  const db = recorded(dbPath);
  return {
    subject: "Genaryx quality baselines (Verdryx)",
    environment,
    takenAt,
    windows: [
      `verdryx.db at ${db.value}: its whole baselines table as this console read it at taken_at, newest created first. ${baselines.length} baseline(s).`,
    ],
    caveats: [
      "source_run_model is filled by joining eval_run_id against the eval runs this console had loaded. An empty one means that run was not in the loaded list, not that the baseline has no source run.",
      "An empty label is a baseline verdryx stored without one. The console shows those as '(unlabeled)'; that word is this console's, not verdryx's, so it is not written here.",
      ...(runs === null
        ? [
            "PARTIAL: no eval runs were loaded when this file was taken, so source_run_model could not be resolved for any row and every one of them is empty for that reason.",
          ]
        : []),
    ],
  };
}

export function routineHistoryExportMeta(
  routine: string,
  history: RoutinesHistoryDto,
  takenAt: string,
  environment: string,
): ExportMeta {
  const count = history.records.length;
  const capReached = count >= ROUTINE_HISTORY_DEFAULT_LIMIT;
  return {
    subject: `Genaryx routine history: ${routine}`,
    environment,
    takenAt,
    windows: [
      `${history.routines_dir}/history.ndjson, filtered to ${routine}, newest first, as this console read it at taken_at. ${count} record(s).`,
    ],
    caveats: [
      `This file is the history of ONE routine, ${routine}. The other routines stack-up records are not in it.`,
      capReached
        ? `PARTIAL: routines_history returns at most ${ROUTINE_HISTORY_DEFAULT_LIMIT} records by default and this console asks for no more. That cap was reached, so runs of ${routine} older than these ${count} are not in this file.`
        : `routines_history caps at ${ROUTINE_HISTORY_DEFAULT_LIMIT} records by default; this file holds ${count}, under the cap.`,
      "reason is only recorded for skipped and error runs, so an empty reason on an ok run is normal rather than missing.",
      ...(history.skipped_lines > 0
        ? [
            `PARTIAL: ${history.skipped_lines} line(s) in history.ndjson could not be parsed and are in no row here. They were not dropped on the box, only unreadable to this console.`,
          ]
        : []),
      ...(history.history_file_exists
        ? []
        : [
            `history.ndjson does not exist in ${history.routines_dir}: no routine has ever run on this box. This file is empty for that reason, not because ${routine} has no runs.`,
          ]),
    ],
  };
}
