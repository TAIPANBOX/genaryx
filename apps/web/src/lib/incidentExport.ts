/**
 * Taking an anomaly out of the console.
 *
 * # Why this is not "download the row"
 *
 * Incident 360 does not show a row. It shows a row plus what the console went
 * and found about it: the run it belongs to, the agent's own record, the bus
 * events around it, the delegation chain it acted under, and the firewall's
 * verdict where there is one. An export of the ROW would be a file that
 * disagrees with the screen it was taken from, which is worse than no export:
 * somebody would send it on, and the thing they were looking at would not be
 * in it.
 *
 * So this exports what the CARD assembled. The card passes in what it already
 * fetched rather than this module fetching again, because a second fetch would
 * be a second answer and the file must be the picture the operator saw.
 *
 * # The caveats are the point
 *
 * `lib/download.ts` requires a provenance block and says why. Here it carries
 * more weight than in a statistics export, because an incident file is
 * evidence: it names an agent, and it will be read by somebody who was not
 * looking at the screen.
 *
 * A section the console could not fill is therefore stated, never omitted. "No
 * run is recorded for this incident" and a file with no `run` key read very
 * differently, and only the first is honest about the console having asked.
 */

import type { ExportMeta } from "./download";
import type { UnifiedIncident } from "./incidents";

/** What the card assembled, handed over rather than re-fetched. */
export interface IncidentPicture {
  row: UnifiedIncident;
  /** The agent this is about, or null when the source names none. */
  subject: string | null;
  /** `on_behalf_of`, root first. Empty when the incident carried none. */
  chain: readonly string[];
  run: unknown | null;
  record: unknown | null;
  /** The events the CARD showed: this incident's run, not the whole fetch.
   * null means "not read", which is not the same as an empty list. */
  busEvents: unknown[] | null;
  /** How many events the console had read altogether, so the block can say
   * "n of m" rather than implying the run's events were all there was. */
  busRead?: number | null;
}

/** The record written to the file. Field names are snake_case to match the
 * event envelope a reader of this file is most likely to have beside it. */
export interface IncidentExport {
  incident: UnifiedIncident;
  subject: string | null;
  delegation: readonly string[];
  run: unknown | null;
  agent_record: unknown | null;
  bus_events: unknown[] | null;
}

export function incidentExport(p: IncidentPicture): IncidentExport {
  return {
    incident: p.row,
    subject: p.subject,
    delegation: p.chain,
    run: p.run,
    agent_record: p.record,
    bus_events: p.busEvents,
  };
}

/**
 * Provenance for one incident export, including what was NOT found.
 *
 * `takenAt` and `environment` are parameters rather than read here, so this
 * function is pure and a test can assert the file without a clock or a DOM.
 */
export function incidentExportMeta(
  p: IncidentPicture & { environment: string; takenAt: string },
): ExportMeta {
  const caveats: string[] = [];
  if (p.run === null) {
    caveats.push("no run is recorded for this incident, so no run was exported");
  }
  if (p.record === null) {
    caveats.push("no agent record was found, so the agent's own declaration is absent");
  }
  if (p.busEvents === null) {
    caveats.push("the bus was not read, so no surrounding events are included");
  } else if (p.busEvents.length === 0) {
    caveats.push("the bus was read and carried no event for this incident's run");
  }
  if (p.chain.length === 0) {
    caveats.push(
      "this incident carries no delegation chain, which means it declared none, " +
        "not that the chain was lost",
    );
  }

  return {
    subject: `anomaly: ${p.row.title}`,
    environment: p.environment,
    takenAt: p.takenAt,
    windows: [
      `incident: as the console held it at ${p.takenAt}`,
      p.busEvents === null
        ? "bus: not read"
        : `bus: ${p.busEvents.length} event(s) for this run, out of ${
            p.busRead ?? p.busEvents.length
          } the console held, since it started`,
    ],
    caveats: caveats.length > 0 ? caveats : undefined,
  };
}

/**
 * A filename a person can find again in a folder a week later.
 *
 * `@yurii 2026-08-26`: it names the AGENT and the MOMENT of the save. The
 * first version used the console's row id and a date, and a row id
 * (`money:inc-1`) says nothing to anybody outside this console, while a date
 * alone collides with every other save made that day.
 *
 * The agent keeps its whole path, not just its last segment:
 * `meridian.io-sre-rca-copilot` rather than `rca-copilot`. Two teams running a
 * bot of the same name is the ordinary case, and a file that cannot tell them
 * apart is the file somebody sends to the wrong person.
 *
 * Seconds, not just the day, because two saves of the same anomaly minutes
 * apart is exactly what happens while an incident is live.
 *
 * Colons are stripped rather than left: they are legal in an `agent://` id and
 * in ISO 8601, and illegal in a Windows filename, so a browser would rewrite
 * the name silently and the file would not be the one this function promised.
 */
export function incidentExportName(
  row: UnifiedIncident,
  takenAt: string,
  subject: string | null,
): string {
  const who = subject
    ? subject.replace(/^agent:\/\//, "").replace(/^user:\/\//, "")
    : row.id;
  const slug = who.replace(/[^A-Za-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "");
  // `2026-08-26T15-39-37`: the T is kept because it is what makes the string
  // read as a moment rather than as two numbers that happen to be adjacent.
  const when = takenAt.slice(0, 19).replace(/:/g, "-");
  return `genaryx-anomaly-${slug || "unknown"}-${when}.json`;
}

/** The columns a saved anomaly list carries. `raw` is deliberately absent: it
 * is a different shape per source and a CSV cell is the wrong home for it.
 * Somebody who needs it opens the incident and saves that. */
export const ANOMALY_COLUMNS: { key: keyof AnomalyRow & string; header: string }[] = [
  { key: "severity", header: "severity" },
  { key: "plane", header: "plane" },
  { key: "title", header: "title" },
  { key: "detail", header: "detail" },
  { key: "agent", header: "agent" },
  { key: "ts", header: "ts" },
  { key: "occurrences", header: "occurrences" },
  { key: "id", header: "id" },
];

export interface AnomalyRow {
  severity: string;
  plane: string;
  title: string;
  detail: string;
  agent: string | null;
  ts: string | null;
  occurrences: number | null;
  id: string;
}

/**
 * Provenance for a saved anomaly LIST, and the filters are the whole reason it
 * is a separate function from [`incidentExportMeta`].
 *
 * The failure it prevents: somebody filters to `critical`, saves, mails the
 * file, and the reader counts two anomalies in an estate that had forty. The
 * count and every active filter therefore go in the block, and a filter that
 * was NOT applied is not mentioned, so the absence of a line means the absence
 * of a filter rather than a line somebody forgot to write.
 */
export function anomalyListMeta(p: {
  shown: number;
  total: number;
  planes: readonly string[];
  severities: readonly string[];
  query: string;
  environment: string;
  takenAt: string;
  busRead: number;
  busTruncated: boolean;
}): ExportMeta {
  const filters: string[] = [];
  if (p.planes.length > 0) filters.push(`plane: ${p.planes.join(", ")}`);
  if (p.severities.length > 0) filters.push(`severity: ${p.severities.join(", ")}`);
  if (p.query.trim() !== "") filters.push(`text: ${p.query.trim()}`);

  const caveats: string[] = [];
  if (p.shown < p.total) {
    caveats.push(
      `filtered: ${p.shown} of ${p.total} anomalies this console held are in this file`,
    );
  }
  if (p.busTruncated) {
    caveats.push(
      `the bus read was capped at ${p.busRead} events, so this is what the console had, not what the estate produced`,
    );
  }

  return {
    subject: `anomalies: ${p.shown} of ${p.total}`,
    environment: p.environment,
    takenAt: p.takenAt,
    windows: [
      `anomalies: as the console held them at ${p.takenAt}`,
      `bus: ${p.busRead} event(s) in this console's memory, since it started`,
      ...(filters.length > 0
        ? [`filters: ${filters.join("; ")}`]
        : ["filters: none, so this is everything the console held"]),
    ],
    caveats: caveats.length > 0 ? caveats : undefined,
  };
}

/**
 * The `{type}:{subject}` a deep link needs, or null when this anomaly has no
 * address.
 *
 * # Why this is not `row.id`
 *
 * `row.id` is this console's own bookkeeping: `bus:104076`, `money:inc-1`. The
 * link scheme in `lib/mailLink.ts` reads `{type}:{subject}` where the type is
 * what a PLANE emitted, which is how `VIEW_BY_TYPE` knows which panel shows
 * it. A link built from the row id parses cleanly, resolves to no view, and
 * lands the reader on the overview saying it could not place the id.
 *
 * That is not a guess. It is what the button did, live, on 2026-08-26, and it
 * was found by pressing it and opening the result rather than by reading the
 * code, which is the only way that class of mistake shows up: the button says
 * "copied", the string looks like a URL, and it goes nowhere.
 *
 * # Null is an answer
 *
 * A posture finding is a computed state rather than a stored event. It has no
 * id in any store and nothing to re-open, so an address for it would resolve
 * to a different answer tomorrow. The same goes for an event that names no run
 * and no agent: there is nothing to point at, and pointing anyway would be the
 * defect above with a different spelling. The caller offers no link at all,
 * which is honest, rather than a link that disappoints.
 */
export function incidentLinkTarget(row: UnifiedIncident): string | null {
  let type: string | undefined;
  let subject: string | null | undefined;

  switch (row.source) {
    case "money": {
      const raw = row.raw as { kind?: string; run_id?: string | null; agent_id?: string | null };
      type = raw.kind;
      subject = raw.run_id ?? raw.agent_id;
      break;
    }
    case "idryx": {
      const raw = row.raw as { detector?: string; identity?: string };
      type = raw.detector;
      subject = raw.identity;
      break;
    }
    default: {
      // bus and verdryx both carry the agent-event envelope.
      const raw = row.raw as { type?: string; run_id?: string | null; agent_id?: string | null };
      type = raw.type;
      subject = raw.run_id ?? raw.agent_id;
      break;
    }
  }

  if (row.source === "posture") return null;
  if (!type || !subject) return null;
  return `${type}:${subject}`;
}
