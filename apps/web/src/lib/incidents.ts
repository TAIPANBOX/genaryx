/**
 * I2 "Unified incident center": a pure aggregator over four ALREADY-FETCHED
 * sources - money incidents, idryx identity alerts, the shared event bus, and
 * triggered posture findings - into one `UnifiedIncident` shape the Overview
 * panel's Incident Center card renders. Kept framework-free (no React/DOM)
 * like `lib/dashData.ts`/`lib/posture.ts`, so `OverviewView.tsx` owns every
 * fetch and just calls [`aggregateIncidents`] with what it currently has.
 *
 * Client-side aggregation only: no new genaryx-api command, no Rust change.
 * Every input is a read some other view in this app already performs
 * (`money_incidents` via `lib/money.ts`, `identity_list_alerts` via
 * `lib/identity.ts`, the live bus via `lib/recentEvents.ts`, and
 * `lib/posture.ts`'s own findings via `lib/usePostureData.ts`).
 *
 * # The bus read kept one type out of forty-two, and nothing said so
 *
 * Until 2026-08-26 the caller filtered its bus read down to `quality_drift`
 * before handing it over, so this module never saw that anything else existed.
 * Five hundred events were fetched on every refresh and all but one type were
 * discarded: qryx's crypto drift, wardryx's refusals and unanswered approvals,
 * scopyx's blocked fetches, mockryx's findings, heraldyx's dispatches, and,
 * the day they shipped, tokenfuse's `dependency_failed` and verdryx's
 * `slo_burn`, both at `high` and both about the box's own health.
 *
 * The panel was not wrong about any number it printed. It was named the
 * incident centre while being blind to six of the ten planes, and an operator
 * looking at a quiet centre had no way to tell that from a healthy estate.
 * That is invariant 8 one level out from a count: a figure is about the
 * question it was asked, or it says which part of the question it could not
 * reach. [`busCoverage`] is the saying-so.
 *
 * The filter is now a SEVERITY BAND rather than a list of types, so a producer
 * that ships a `high` event ships it into this panel the same day and nobody
 * has to come back here. See [`INCIDENT_BANDS`].
 */
import type { IdryxAlert } from "../identityTypes";
import type { Incident } from "../moneyTypes";
import type { PostureFinding } from "./posture";
import type { UiEvent } from "../types";
import type { ViewId } from "./views";
import { sevRank } from "./dashData";
import type { Severity } from "../types";

export type IncidentSource = "money" | "idryx" | "verdryx" | "posture" | "bus";

/** Chip label per source (I2 spec's own exact wording): idryx/verdryx are
 * OTHER services' detections being surfaced here ("borrowed"), so they carry
 * a "via" prefix (the same "via idryx" precedent named in the spec); money
 * and posture are this console's own native planes and carry no prefix. */
export const INCIDENT_SOURCE_LABEL: Record<IncidentSource, string> = {
  money: "money",
  idryx: "via idryx",
  verdryx: "via verdryx",
  posture: "posture",
  // A row that arrived on the shared bus and belongs to a plane with no panel
  // of its own here. The chip carries the producer's own `source` at render
  // time (see `busPlaneLabel`), so this fallback is only ever seen for an
  // event whose source string is empty, which the envelope forbids.
  bus: "via the bus",
};

/** Which tab a source chip's click navigates to (`AppShell.tsx`'s existing
 * view-switching mechanism, threaded down as `onSelectView`). */
export const INCIDENT_SOURCE_VIEW: Record<IncidentSource, ViewId> = {
  money: "money",
  idryx: "identity",
  verdryx: "quality",
  posture: "posture",
  bus: "bus",
};

/** Which panel owns a bus event, keyed by the producer's `source`.
 *
 * Per SOURCE and never per TYPE, and that is the whole reason this is a small
 * map rather than a large one. SPEC 6.2 registers forty-two type strings
 * across ten sources and grows a type whenever a producer ships one; it grows
 * a SOURCE roughly never. A per-type table here would be a third copy of that
 * registry, after the spec itself and heraldyx's render catalogue, and the two
 * event types that shipped on 2026-08-26 would have been invisible in this
 * console until somebody remembered to come and add them.
 *
 * A source with no panel falls through to the Bus Explorer, which is the
 * honest answer rather than a missing one: the event is real, this console has
 * nowhere better to send you, and the raw line is there. */
export const BUS_PLANE_VIEW: Readonly<Record<string, ViewId>> = {
  tokenfuse: "money",
  wardryx: "policy",
  idryx: "identity",
  verdryx: "quality",
  qryx: "crypto",
  engram: "memory",
  mockryx: "drills",
  scopyx: "egress",
  console: "routines",
  heraldyx: "bus",
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
  | (UnifiedIncidentBase & { source: "posture"; raw: PostureFinding })
  | (UnifiedIncidentBase & { source: "bus"; raw: UiEvent });

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

/** The bands worth an operator's attention on the incident centre.
 *
 * A RULE rather than a list of types, and the difference is the point. This
 * console read five hundred events off the shared bus and kept exactly one
 * type of them, `quality_drift`, from the moment the Incident Center shipped.
 * Every other line was fetched and discarded: qryx's crypto drift, wardryx's
 * refusals and unanswered approvals, scopyx's blocked fetches, mockryx's
 * findings, and on 2026-08-26 the two types that shipped that day,
 * tokenfuse's `dependency_failed` and verdryx's `slo_burn`, both at `high`.
 *
 * A per-type allowlist would have had the same shape and the same fate, one
 * type further along. Severity is fixed per type at the producer, by design
 * across this estate, which makes it the one field a consumer can route on
 * without keeping its own copy of somebody else's vocabulary. So a producer
 * that ships a `high` event ships it into this panel the same day, and this
 * file does not have to be told. */
export const INCIDENT_BANDS: ReadonlySet<string> = new Set(["critical", "high"]);

export function isIncidentEvent(e: UiEvent): boolean {
  return e.severity !== null && INCIDENT_BANDS.has(e.severity);
}

/** The producer's own source string, for the chip. Falls back to the union
 * member only for an event whose source is empty, which the envelope forbids
 * and which therefore only arrives from a fixture. */
export function busPlaneLabel(e: UiEvent): string {
  return e.source ? `via ${e.source}` : INCIDENT_SOURCE_LABEL.bus;
}

/** Where a bus row's chip navigates. The Bus Explorer for a plane with no
 * panel here, which is a real destination and not a dead chip. */
export function busPlaneView(e: UiEvent): ViewId {
  return BUS_PLANE_VIEW[e.source] ?? INCIDENT_SOURCE_VIEW.bus;
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
/** Every bus event in an incident band that `fromQualityDrift` does not
 * already render richly.
 *
 * The title is the producer's own type with its underscores opened out, the
 * same shape `fromMoney` gives a money incident's `kind`, because the type
 * string IS the name of the thing that happened and inventing a friendlier one
 * here would be a fourth copy of somebody else's vocabulary.
 *
 * The detail names the subject and, where the event carries them, the two
 * members that turn a type into a fact: this reads `data` best-effort and
 * shows nothing rather than guessing. No member is required, none is parsed
 * into a typed field, and an event whose `data` is absent still renders with
 * its subject and its time.
 */
function fromBus(events: readonly UiEvent[]): UnifiedIncident[] {
  const groups = new Map<string, { first: UiEvent; newest: UiEvent; count: number }>();
  for (const e of events) {
    if (!isIncidentEvent(e) || isQualityDriftEvent(e)) continue;
    const key = busGroupKey(e);
    const g = groups.get(key);
    if (!g) {
      groups.set(key, { first: e, newest: e, count: 1 });
      continue;
    }
    g.count += 1;
    if (tsRank(e.ts) > tsRank(g.newest.ts)) g.newest = e;
  }
  return [...groups.values()].map(({ newest, count }) => {
    const parts: string[] = [newest.agent_id || "fleet"];
    if (newest.run_id) parts.push(`run ${newest.run_id}`);
    for (const key of BUS_DETAIL_KEYS) {
      const value = dataString(newest.data, key);
      if (value !== null) parts.push(`${key} ${value}`);
    }
    return {
      // The NEWEST event's id, so the row's React key changes when the group
      // grows and the count on screen cannot go stale against its own row.
      id: `bus:${newest.id}`,
      source: "bus" as const,
      severity: normalizeSeverity(newest.severity),
      title: newest.type.replace(/_/g, " "),
      detail: parts.join(" · "),
      ts: newest.ts,
      occurrences: count,
      ackable: false,
      explainable: false,
      raw: newest,
    };
  });
}

/** What counts as "the same thing happening again" on the bus.
 *
 * **Found by looking at the panel rather than at a test.** With the bus read
 * widened, the card filled with nine rows of `budget exceeded` from one run of
 * one agent, and the estate's loudest run pushed every other plane off a
 * ten-row card. Every unit test still passed, because each of those rows was
 * a correct rendering of a real event.
 *
 * Money incidents never had this problem: they arrive already aggregated with
 * an `occurrences` count, which is why the card has a column for one. Bus
 * events arrive raw, so the grouping happens here and reuses that same column.
 *
 * The key is producer, type, subject and run. Run is IN it on purpose: two
 * budget refusals in one run are one situation, and the same refusal in two
 * different runs is two, which is the distinction an operator is actually
 * making when they look at this card. A run-less event groups per agent, which
 * is the right fallback for a fleet-wide signal that has no run to belong to. */
function busGroupKey(e: UiEvent): string {
  return `${e.source}|${e.type}|${e.agent_id}|${e.run_id ?? ""}`;
}

/** `data` members worth putting in a one-line detail, in the order they read.
 *
 * Chosen because each one changes what an operator should DO about the row,
 * and every one of them is a member some producer already documents in SPEC
 * 6.2: which of the box's own dependencies failed and what the gateway did
 * about it, which objective a burn is against, which detector fired, which
 * tool or verdict a refusal names. A member no event carries costs nothing;
 * `dataString` returns null and the part is skipped.
 *
 * This IS a small copy of other people's field names, and it is the one place
 * here that could go stale. It goes stale SAFELY: a renamed member drops a
 * clause from a sentence and never hides the row, which is why it is a detail
 * line and not a filter. */
const BUS_DETAIL_KEYS: readonly string[] = [
  "dependency",
  "effect",
  "sli",
  "trigger",
  "detector",
  "verdict",
  "reason",
  "tool",
];

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
  /** The bus read, WHOLE. It was `qualityDriftEvents` until 2026-08-26 and the
   * caller filtered before handing it over, which is why every other plane's
   * incidents never arrived: the narrowing happened before this module could
   * see there was anything else. The filtering now happens here, where the
   * rule is written down and tested. */
  busEvents: readonly UiEvent[];
  postureFindings: readonly PostureFinding[];
}

/** What the bus read could and could not account for.
 *
 * Invariant 8's shape applied to a panel rather than to a count: a figure is
 * about the question it was asked, or it names the part of the question it
 * could not reach. A quiet incident centre and a blind one look identical, and
 * this is the difference between them.
 *
 * `planes` is what actually contributed, so an operator can see at a glance
 * that four planes are represented and six are silent. Silence there is not a
 * finding on its own: a plane with nothing to report is exactly as silent as
 * one that is not running, and this console cannot tell those apart from the
 * bus alone. Saying which planes spoke is honest; claiming the rest are
 * healthy would not be.
 *
 * `truncated` is the one that would otherwise mislead. The read is capped, and
 * a cap reached is a window shorter than the one the operator thinks they are
 * looking at. */
export interface BusCoverage {
  read: number;
  limit: number;
  truncated: boolean;
  planes: readonly string[];
  incidentRows: number;
}

export function busCoverage(events: readonly UiEvent[], limit: number): BusCoverage {
  const planes = new Set<string>();
  let incidentRows = 0;
  for (const e of events) {
    if (e.source) planes.add(e.source);
    if (isIncidentEvent(e)) incidentRows += 1;
  }
  return {
    read: events.length,
    limit,
    // `>=` and not `===`: a caller that raised the cap without telling this
    // function would otherwise report a full window as a partial one, and a
    // backend answering with more than was asked for is still a read this
    // console cannot claim is complete.
    truncated: events.length >= limit,
    planes: [...planes].sort(),
    incidentRows,
  };
}

/** Aggregate the sources into one worst-first list. Sorting: severity
 * rank desc (reuses `lib/dashData.ts`'s existing `sevRank` - already shared
 * by `MoneyView.tsx`/`OverviewView.tsx`, nothing to extract here), then
 * occurrences desc, then timestamp desc. Pure and total: never throws,
 * never mutates its input, always returns every row (callers slice to a
 * top-N themselves, e.g. the Incident Center card's top 10). */
export function aggregateIncidents(input: AggregateIncidentsInput): UnifiedIncident[] {
  const rows: UnifiedIncident[] = [
    ...fromMoney(input.moneyIncidents),
    ...fromIdentity(input.identityAlerts),
    ...fromQualityDrift(input.busEvents),
    ...fromBus(input.busEvents),
    ...fromPosture(input.postureFindings),
  ];
  return rows.sort(
    (a, b) =>
      sevRank(b.severity) - sevRank(a.severity) ||
      (b.occurrences ?? 0) - (a.occurrences ?? 0) ||
      tsRank(b.ts) - tsRank(a.ts),
  );
}

/** The plane a row belongs to, as an operator would name it.
 *
 * `source` is this module's own four-plus-one union and says how a row REACHED
 * the console; a filter wants the producer. A bus row answers with the
 * producer's own string, everything else with the union member, which is
 * already the plane's name for those.
 */
export function incidentPlane(row: UnifiedIncident): string {
  return row.source === "bus" ? row.raw.source || "bus" : row.source;
}

export interface IncidentFilter {
  /** Empty means every plane. */
  planes?: readonly string[];
  /** Empty means every band. */
  severities?: readonly string[];
  /** Case-insensitive substring over the subject and the detail. Empty means
   * every row. Substring rather than exact match on purpose: an operator types
   * the part of an agent id they remember, not the whole `agent://` URI. */
  query?: string;
}

/** Filter without reordering. `aggregateIncidents` has already sorted
 * worst-first and a filter that re-sorted would quietly answer a different
 * question than the card above it. */
export function filterIncidents(
  rows: readonly UnifiedIncident[],
  filter: IncidentFilter,
): UnifiedIncident[] {
  const planes = new Set(filter.planes ?? []);
  const severities = new Set(filter.severities ?? []);
  const q = (filter.query ?? "").trim().toLowerCase();
  return rows.filter((row) => {
    if (planes.size > 0 && !planes.has(incidentPlane(row))) return false;
    if (severities.size > 0 && !severities.has(row.severity)) return false;
    if (q && !`${row.title} ${row.detail}`.toLowerCase().includes(q)) return false;
    return true;
  });
}

/** Every plane present in a row set, sorted, for building a filter control
 * from what is actually there rather than from a list this file would have to
 * keep true. A plane with no rows today gets no chip, which is the honest
 * shape: a chip that filters to nothing tells an operator the plane is quiet
 * when this console may simply never have heard of it. */
export function planesPresent(rows: readonly UnifiedIncident[]): string[] {
  return [...new Set(rows.map(incidentPlane))].sort();
}

/** The agent an incident is about, or "" when it is about no single one.
 *
 * A fleet-wide signal genuinely has no subject, and "" says so. A placeholder
 * would put a name on something that did not do the thing, which is the rule
 * the whole estate keeps: SPEC 6.1 forbids inventing an `agent_id` at the
 * producer, and a consumer inventing one downstream is the same error one
 * plane later. */
export function incidentSubject(row: UnifiedIncident): string {
  switch (row.source) {
    case "bus":
    case "verdryx":
      return row.raw.agent_id ?? "";
    case "money":
      return row.raw.agent_id ?? "";
    case "idryx":
      return row.raw.identity ?? "";
    default:
      return "";
  }
}

/** Who asked for the work, root first.
 *
 * Only the envelope carries this: `on_behalf_of` is a delegation chain whose
 * root is a `user://` when a person started the run, which is the one place
 * this console can answer "who set this off" rather than "which agent did it".
 * Money incidents, identity alerts and posture findings carry no chain, and an
 * empty array is the honest answer for them rather than a guess from the
 * agent's owner: the owner is who ANSWERS for an agent, not who asked it to do
 * this particular thing, and conflating the two would name the wrong person in
 * the one card built to name the right one. */
export function incidentDelegation(row: UnifiedIncident): readonly string[] {
  if (row.source === "bus" || row.source === "verdryx") return row.raw.on_behalf_of ?? [];
  return [];
}

/** The producer's own `data`, as a plain object, or null.
 *
 * Never parsed into typed fields. This console reports what a producer wrote;
 * reading a free-form member into a claim is the thing trailryx's mapper
 * refuses to do one plane over, for the same reason. */
export function incidentData(row: UnifiedIncident): Record<string, unknown> | null {
  const raw = row.source === "bus" || row.source === "verdryx" ? row.raw.data : null;
  return raw && typeof raw === "object" ? (raw as Record<string, unknown>) : null;
}

/** The run an incident belongs to, where it has one. */
export function incidentRunId(row: UnifiedIncident): string | null {
  if (row.source === "bus" || row.source === "verdryx") return row.raw.run_id;
  if (row.source === "money") return row.raw.run_id;
  return null;
}
