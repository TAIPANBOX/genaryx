/**
 * I2 "Unified incident center": a pure aggregator over four ALREADY-FETCHED
 * sources - money incidents, idryx identity alerts, verdryx quality-drift
 * bus events, and triggered posture findings - into one `UnifiedIncident`
 * shape the Overview panel's Incident Center card renders. Kept
 * framework-free (no React/Tauri) like `lib/dashData.ts`/`lib/posture.ts`,
 * so `OverviewView.tsx` owns every fetch and just calls
 * [`aggregateIncidents`] with what it currently has.
 *
 * Client-side aggregation only: no new genaryx-api command, no Rust change -
 * every input here is a read some other view in this app already performs
 * (`money_incidents` via `lib/money.ts`, `identity_list_alerts` via
 * `lib/identity.ts`, the live bus via `lib/recentEvents.ts` filtered the
 * same way `QualityDriftStream.tsx` already does, and `lib/posture.ts`'s own
 * findings via `lib/usePostureData.ts`).
 */
import type { IdryxAlert } from "../identityTypes";
import type { Incident } from "../moneyTypes";
import type { PostureFinding } from "./posture";
import type { UiEvent } from "../types";
import type { ViewId } from "./views";
import { sevRank } from "./dashData";
import type { Severity } from "../types";

export type IncidentSource = "money" | "idryx" | "verdryx" | "posture";

/** Chip label per source (I2 spec's own exact wording): idryx/verdryx are
 * OTHER services' detections being surfaced here ("borrowed"), so they carry
 * a "via" prefix (the same "via idryx" precedent named in the spec); money
 * and posture are this console's own native planes and carry no prefix. */
export const INCIDENT_SOURCE_LABEL: Record<IncidentSource, string> = {
  money: "money",
  idryx: "via idryx",
  verdryx: "via verdryx",
  posture: "posture",
};

/** Which tab a source chip's click navigates to (`AppShell.tsx`'s existing
 * view-switching mechanism, threaded down as `onSelectView`). */
export const INCIDENT_SOURCE_VIEW: Record<IncidentSource, ViewId> = {
  money: "money",
  idryx: "identity",
  verdryx: "quality",
  posture: "posture",
};

interface UnifiedIncidentBase {
  id: string;
  severity: Severity;
  title: string;
  detail: string;
  /** UTC ISO-8601 when the source carries a real one - posture findings
   * never do (they describe a "right now" computed state, not a discrete
   * historical event), so this stays `undefined` there rather than a
   * fabricated timestamp. */
  ts?: string;
  occurrences?: number;
  /** True only for an unacknowledged money incident (`money_ack_incident`
   * exists for no other source). */
  ackable: boolean;
  /** True only for a money incident: `copilot_explain`'s DTO is
   * money-incident-specific, not a free-form context
   * (`CopilotService::explain_incident`'s fixed prompt only ever searches
   * the `incidents` tool, itself backed by `CloudClient`/money data - traced
   * down to `crates/copilot/src/tools/cloud.rs`), so a non-money row cannot
   * be explained without a Rust-side change - named as a follow-up rather
   * than silently attempted against an id it could never resolve. */
  explainable: boolean;
}

export type UnifiedIncident =
  | (UnifiedIncidentBase & { source: "money"; raw: Incident })
  | (UnifiedIncidentBase & { source: "idryx"; raw: IdryxAlert })
  | (UnifiedIncidentBase & { source: "verdryx"; raw: UiEvent })
  | (UnifiedIncidentBase & { source: "posture"; raw: PostureFinding });

const KNOWN_SEVERITIES: ReadonlySet<string> = new Set(["critical", "high", "medium", "low", "info"]);

/** Normalize a raw, untrusted severity string into the closed `Severity`
 * union, falling back to "info" for anything unrecognized - mirrors
 * `SeverityBadge`/`sevColor`'s own "never look more assured than the data
 * actually is" tolerance rather than throwing on a surprising value. */
function normalizeSeverity(raw: string | null | undefined): Severity {
  return (raw && KNOWN_SEVERITIES.has(raw) ? raw : "info") as Severity;
}

function fromMoney(incidents: readonly Incident[]): UnifiedIncident[] {
  return incidents.map((inc) => ({
    id: `money:${inc.id}`,
    source: "money",
    severity: normalizeSeverity(inc.severity),
    title: inc.kind.replace(/_/g, " "),
    detail: `${inc.run_id ?? inc.agent_id ?? "fleet"} · ${inc.occurrences}× · ${inc.acknowledged ? "acknowledged" : "open"}`,
    ts: inc.last_seen,
    occurrences: inc.occurrences,
    ackable: !inc.acknowledged,
    explainable: true,
    raw: inc,
  }));
}

function fromIdentity(alerts: readonly IdryxAlert[]): UnifiedIncident[] {
  return alerts.map((a, idx) => ({
    // idryx alerts carry no id of their own (docs/PHASE3.md's grounded
    // contract) - detector+identity+time is unique in practice, `idx`
    // breaks a same-millisecond tie so React's `key` is always distinct.
    id: `idryx:${a.detector}:${a.identity}:${a.time}:${idx}`,
    source: "idryx",
    severity: normalizeSeverity(a.severity),
    title: a.detector.replace(/_/g, " "),
    detail: `${a.identity} · ${a.summary}`,
    ts: a.time,
    ackable: false,
    explainable: false,
    raw: a,
  }));
}

/** Mirrors `QualityDriftStream.tsx`'s own, independently-defined
 * `isQualityDrift`/`dataNumber`/`dataString` helpers exactly (same
 * `source === "verdryx" && type === "quality_drift"` filter, same
 * best-effort untyped-`data` field reads) - duplicated rather than imported
 * from that component on purpose: a `lib/*` module must not depend on a
 * `components/*` one, and this is the same "two independent literals, not
 * worth a shared dependency" call `lib/posture.ts` already makes for its own
 * `SCHEMA_V0_1`/`SCHEMA_V0_2` constants. Exported so `OverviewView.tsx` (this
 * module's caller) can filter its OWN independent bus read with the exact
 * same predicate before handing the result to [`aggregateIncidents`], rather
 * than re-deriving it a third time. */
export const VERDRYX_SOURCE = "verdryx";
export const DRIFT_TYPE = "quality_drift";

export function isQualityDriftEvent(e: UiEvent): boolean {
  return e.source === VERDRYX_SOURCE && e.type === DRIFT_TYPE;
}

function dataNumber(data: unknown, key: string): number | null {
  if (data && typeof data === "object" && key in (data as Record<string, unknown>)) {
    const value = (data as Record<string, unknown>)[key];
    if (typeof value === "number") return value;
  }
  return null;
}

function dataString(data: unknown, key: string): string | null {
  if (data && typeof data === "object" && key in (data as Record<string, unknown>)) {
    const value = (data as Record<string, unknown>)[key];
    if (typeof value === "string") return value;
  }
  return null;
}

/** Docs/PHASE4.md's own grounded contract: a `quality_drift` bus event fires
 * ONLY on a regression, always at severity "high" - so in practice `verdict`
 * is always `"regressed"` on the real bus. This still maps explicitly
 * (rather than assuming) so a differently-verdicted event - a future
 * addition, a test fixture - degrades to the event's own reported severity
 * instead of silently mislabeling. */
function fromQualityDrift(events: readonly UiEvent[]): UnifiedIncident[] {
  return events.filter(isQualityDriftEvent).map((e) => {
    const verdict = dataString(e.data, "verdict");
    const delta = dataNumber(e.data, "delta");
    const baselineId = dataString(e.data, "baseline_id");
    const meanScore = dataNumber(e.data, "mean_score");
    const severity: Severity = verdict === "regressed" ? "high" : normalizeSeverity(e.severity);
    return {
      id: `verdryx:${e.id}`,
      source: "verdryx",
      severity,
      title: verdict ? `quality drift: ${verdict}` : "quality drift",
      detail:
        `${e.agent_id} · baseline ${baselineId ?? "n/a"}` +
        ` · delta ${delta !== null ? delta.toFixed(3) : "n/a"}` +
        (meanScore !== null ? ` · mean ${meanScore.toFixed(3)}` : ""),
      ts: e.ts,
      ackable: false,
      explainable: false,
      raw: e,
    };
  });
}

/** `PostureFinding.severity` is always set (never optional - see
 * `posture.ts`'s own doc comment on the field), so this is a straight
 * pass-through, not a fallback chain: it already yields "high" for
 * `devkey`/`governance_fail_open`/`idryx_exposed`/`wardryx_keyless_admin`/
 * `policy_plane_health` (this codebase's fail-open/devkey-class zonds) and
 * "medium"/"info" for the rest, with no special-casing needed here. */
function fromPosture(findings: readonly PostureFinding[]): UnifiedIncident[] {
  return findings
    .filter((f) => f.state === "triggered")
    .map((f) => ({
      id: `posture:${f.id}`,
      source: "posture",
      severity: f.severity,
      title: f.title,
      detail: f.whyItMatters,
      ackable: false,
      explainable: false,
      raw: f,
    }));
}

/** `ts` sorts as "oldest possible" when absent or unparseable, so a
 * same-severity/same-occurrences tie always prefers a row that DOES carry a
 * real timestamp over one that does not, rather than an arbitrary array
 * order. */
function tsRank(ts: string | undefined): number {
  if (!ts) return -Infinity;
  const ms = Date.parse(ts);
  return Number.isFinite(ms) ? ms : -Infinity;
}

export interface AggregateIncidentsInput {
  moneyIncidents: readonly Incident[];
  identityAlerts: readonly IdryxAlert[];
  qualityDriftEvents: readonly UiEvent[];
  postureFindings: readonly PostureFinding[];
}

/** Aggregate the four sources into one worst-first list. Sorting: severity
 * rank desc (reuses `lib/dashData.ts`'s existing `sevRank` - already shared
 * by `MoneyView.tsx`/`OverviewView.tsx`, nothing to extract here), then
 * occurrences desc, then timestamp desc. Pure and total: never throws,
 * never mutates its input, always returns every row (callers slice to a
 * top-N themselves, e.g. the Incident Center card's top 10). */
export function aggregateIncidents(input: AggregateIncidentsInput): UnifiedIncident[] {
  const rows: UnifiedIncident[] = [
    ...fromMoney(input.moneyIncidents),
    ...fromIdentity(input.identityAlerts),
    ...fromQualityDrift(input.qualityDriftEvents),
    ...fromPosture(input.postureFindings),
  ];
  return rows.sort(
    (a, b) =>
      sevRank(b.severity) - sevRank(a.severity) ||
      (b.occurrences ?? 0) - (a.occurrences ?? 0) ||
      tsRank(b.ts) - tsRank(a.ts),
  );
}
