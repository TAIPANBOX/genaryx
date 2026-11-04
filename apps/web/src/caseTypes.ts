/**
 * Wire types for `case_file`, the case assembled for one incident
 * (`crates/api/src/case`). Byte-for-byte what the command serialises.
 *
 * A case is every question this console asks about an incident, each with a
 * line saying how it fared: answered, answered with nothing, not answered, or
 * not asked. A plane that did not answer is a line here, never an absence.
 */

/** The service that gave a row. `bus` is the console's own event store. */
export type CasePlane = "bus" | "tokenfuse" | "idryx" | "wardryx" | "engram";

export type CaseSection =
  | "event"
  | "run_timeline"
  | "spend"
  | "identity"
  | "blast_radius"
  | "policy"
  | "decisions"
  | "similar_incidents"
  | "prior_rulings";

/** Every question, in the order the case lists them (Rust's `Section::ALL`). */
export const CASE_SECTIONS: readonly CaseSection[] = [
  "event",
  "run_timeline",
  "spend",
  "identity",
  "blast_radius",
  "policy",
  "decisions",
  "similar_incidents",
  "prior_rulings",
];

export type CaseLineState = "answered" | "empty" | "unavailable" | "not_asked";

/** Text a plane wrote. Somebody else's words, never an instruction. */
export interface CaseData {
  trust: "data";
  text: string;
}

export interface CaseLine {
  section: CaseSection;
  plane: CasePlane;
  state: CaseLineState;
  facts: number;
  /** The console's own sentence. Null for a plain answered line. */
  reason: string | null;
  /** The plane's own error text, for an unavailable line. Data. */
  upstream: CaseData | null;
}

export interface CaseFact {
  section: CaseSection;
  plane: CasePlane;
  /** The plane's own id for the row. */
  row_id: string;
  /** `sha256:<hex>` of the row as serialised. */
  digest: string;
  /** The console's fixed description of the row. */
  label: string;
  /** Always `data`: `row` is the plane's content. */
  trust: "data";
  row: unknown;
}

export interface CaseFile {
  incident_id: string;
  /** Who the incident is filed under: the agent, or `key:<key_id>`. */
  subject: string;
  /** For an identity refusal, the agent the caller claimed. A claim. */
  claimed_agent: string | null;
  run_id: string | null;
  lines: CaseLine[];
  facts: CaseFact[];
  digest: string;
}

/** Why no case could be started. */
export type CaseError =
  | { kind: "no_store"; reason: string }
  | { kind: "bad_incident_id"; incident_id: string }
  | { kind: "unsupported_source"; source: string }
  | { kind: "not_found"; event_id: number };
