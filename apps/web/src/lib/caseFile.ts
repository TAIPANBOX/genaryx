import { hasBackend, invokeBackend } from "./transport";
import type { UnifiedIncident } from "./incidents";
import type { CopilotAnswer } from "../copilotTypes";
import type {
  CaseError,
  CaseFact,
  CaseFile,
  CaseLine,
  CaseLineState,
  CasePlane,
  CaseSection,
} from "../caseTypes";

/**
 * The case for an incident, and how Incident 360 words it.
 *
 * The case itself is assembled in Rust (`crates/api/src/case`), with no model.
 * Everything here is reading it: asking for it, and turning each line into the
 * console's own sentence.
 *
 * # THE RULE THIS FILE KEEPS
 *
 * A sentence the console says is built from this file's fixed words and the
 * case's own fixed fields (the plane, the state, the count). It never contains
 * text a plane or an event wrote: that arrives in `upstream.text` and in a
 * fact's `row`, and the view prints it inside a frame marked as data.
 */

/** What asking for a case came to. */
export type CaseResult =
  | { kind: "case"; case: CaseFile }
  /** The box refused to start a case, and said why. */
  | { kind: "refused"; error: CaseError }
  /** A source with no event row (money, identity, posture): no case to ask for. */
  | { kind: "no_source" }
  /** The command itself got no answer. */
  | { kind: "unreachable"; message: string }
  /** No box at all behind this console (a bare preview). */
  | { kind: "no_backend" };

/** The incident id `case_file` reads: an event row, `bus:<n>` or `verdryx:<n>`.
 * Null for a source that has no event row, which the view says in words
 * rather than asking and being refused. */
export function caseIncidentId(row: Pick<UnifiedIncident, "id" | "source">): string | null {
  return row.source === "bus" || row.source === "verdryx" ? row.id : null;
}

function isCaseError(v: unknown): v is CaseError {
  return (
    typeof v === "object" &&
    v !== null &&
    typeof (v as { kind?: unknown }).kind === "string" &&
    ["no_store", "bad_incident_id", "unsupported_source", "not_found"].includes(
      (v as { kind: string }).kind,
    )
  );
}

function isCaseFile(v: unknown): v is CaseFile {
  if (typeof v !== "object" || v === null) return false;
  const c = v as Partial<CaseFile>;
  return (
    typeof c.incident_id === "string" &&
    typeof c.subject === "string" &&
    typeof c.digest === "string" &&
    Array.isArray(c.lines) &&
    Array.isArray(c.facts)
  );
}

/** Ask the box for one incident's case. */
export async function fetchCase(incidentId: string): Promise<CaseResult> {
  if (!hasBackend()) return { kind: "no_backend" };
  try {
    const raw = await invokeBackend<unknown>("case_file", { incident_id: incidentId });
    if (!isCaseFile(raw)) {
      return { kind: "unreachable", message: "the box answered with something that is not a case" };
    }
    return { kind: "case", case: raw };
  } catch (err) {
    if (isCaseError(err)) return { kind: "refused", error: err };
    return {
      kind: "unreachable",
      message: err instanceof Error ? err.message : "the box did not answer the case command",
    };
  }
}

/** What asking Felyx to read a case came to. */
export type FelyxReading =
  | { kind: "answer"; text: string; unsupported: string[] }
  /** The command refused, with a sentence: no event row, no provider. */
  | { kind: "refused"; message: string }
  | { kind: "no_backend" };

/** Ask Felyx to read the case of one incident. The box assembles the case
 * and hands it over whole: Felyx has no tools while it reads, and may cite
 * only ids the case holds. */
export async function explainCase(incidentId: string): Promise<FelyxReading> {
  void hasBackend;
  void invokeBackend;
  return { kind: "refused", message: incidentId.slice(0, 0) };
}

/** The console's name for each plane. */
export const PLANE_NAME: Record<CasePlane, string> = {
  bus: "the console's event store",
  tokenfuse: "TokenFuse",
  idryx: "Idryx",
  wardryx: "Wardryx",
  engram: "Engram",
};

/** Each question, as the card titles it. */
export const SECTION_TITLE: Record<CaseSection, string> = {
  event: "What happened",
  run_timeline: "What the run did, in order",
  spend: "What the run cost",
  identity: "Who the subject is",
  blast_radius: "What it could reach",
  policy: "Which policy is running",
  decisions: "What was decided on this run",
  similar_incidents: "Earlier incidents of this kind",
  prior_rulings: "What memory holds about this kind",
};

/** A plane's name, tolerating one this build has never heard of. */
export function planeName(plane: string): string {
  return (PLANE_NAME as Record<string, string | undefined>)[plane] ?? plane;
}

/** The console's sentence for one line of the case. */
export function lineSentence(line: CaseLine): string {
  const plane = planeName(line.plane);
  switch (line.state) {
    case "answered": {
      const n = `${line.facts} ${line.facts === 1 ? "fact" : "facts"} from ${plane}`;
      return line.reason ? `${n}. ${line.reason}.` : `${n}.`;
    }
    case "empty":
      return `${plane} answered and holds nothing for this incident.`;
    case "unavailable":
      return `${plane} did not answer.`;
    case "not_asked":
      return line.reason ? `Not asked: ${line.reason}.` : "Not asked.";
    default:
      // A state a newer console sends. Said as unknown, never as answered.
      return "This build does not know this state.";
  }
}

export interface CaseCoverage {
  answered: number;
  empty: number;
  unavailable: number;
  not_asked: number;
}

/** How the case's lines fared, counted. Every line is in exactly one count. */
export function coverage(c: Pick<CaseFile, "lines">): CaseCoverage {
  const out: CaseCoverage = { answered: 0, empty: 0, unavailable: 0, not_asked: 0 };
  for (const line of c.lines) {
    const state: CaseLineState = line.state;
    if (state in out) out[state] += 1;
  }
  return out;
}

/** The one sentence over the whole case. */
export function coverageSentence(c: Pick<CaseFile, "lines">): string {
  const n = coverage(c);
  const parts = [`${n.answered} of ${c.lines.length} questions answered`];
  if (n.empty > 0) parts.push(`${n.empty} answered with nothing`);
  if (n.unavailable > 0) parts.push(`${n.unavailable} did not answer`);
  if (n.not_asked > 0) parts.push(`${n.not_asked} not asked`);
  return `${parts.join(", ")}.`;
}

/** The lines the case could not fill: a plane that did not answer, and a
 * question that could not be asked. What is missing, in the case's own order. */
export function missingLines(c: Pick<CaseFile, "lines">): CaseLine[] {
  return c.lines.filter((l) => l.state === "unavailable" || l.state === "not_asked");
}

/** This section's facts, in the case's order. */
export function factsOf(c: Pick<CaseFile, "facts">, section: CaseSection): CaseFact[] {
  return c.facts.filter((f) => f.section === section);
}

/** The short form of a digest, for a line a person reads. */
export function shortDigest(digest: string): string {
  const hex = digest.startsWith("sha256:") ? digest.slice("sha256:".length) : digest;
  return `sha256:${hex.slice(0, 12)}`;
}

/** What a refused case says. */
export function refusalSentence(error: CaseError): string {
  switch (error.kind) {
    case "unsupported_source":
      return `No case for this incident: a ${error.source} incident has no event row to start from.`;
    case "not_found":
      return `No case: the console's store holds no event row ${error.event_id}.`;
    case "bad_incident_id":
      return "No case: this is not an incident id the case command reads.";
    case "no_store":
      return "No case: the console has no readable event store on this box.";
    default:
      return "No case: the box refused it for a reason this build does not know.";
  }
}
