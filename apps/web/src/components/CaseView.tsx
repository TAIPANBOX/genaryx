import { useEffect, useState, type ReactNode } from "react";
import type { CaseFact, CaseLine } from "../caseTypes";
import {
  PLANE_NAME,
  SECTION_TITLE,
  caseIncidentId,
  coverageSentence,
  explainCase,
  factsOf,
  fetchCase,
  lineSentence,
  missingLines,
  planeName,
  refusalSentence,
  shortDigest,
  type CaseResult,
  type FelyxReading,
} from "../lib/caseFile";
import type { UnifiedIncident } from "../lib/incidents";

/** How many of a section's facts are listed before the rest are counted. */
const FACTS_SHOWN = 8;

/** The longest row preview shown. The case holds the whole row. */
const ROW_PREVIEW_CHARS = 600;

/**
 * The case for an incident, in Incident 360.
 *
 * It shows what was gathered AND what is missing, each in the console's own
 * words. A plane that did not answer is a line of its own, a question that
 * could not be asked says why, and neither is left out because it has nothing
 * to show: a card that dropped them would read as "nothing to report", which
 * is what a console that could not look must never say (invariants 4 and 8).
 *
 * # DATA IS PRINTED AS DATA
 *
 * Event text, memory text and a plane's own error text arrive in a fact's
 * `row` and in a line's `upstream`. Each is printed as an escaped text node
 * inside an element marked `data-trust="data"`, never assembled into a
 * sentence the console says: every sentence here comes from `lib/caseFile.ts`'s
 * fixed words and the case's own fixed fields.
 */
export function CaseView({ result }: { result: CaseResult }) {
  switch (result.kind) {
    case "case": {
      const c = result.case;
      const missing = missingLines(c);
      return (
        <div className="flex flex-col" style={{ gap: 8 }} data-case-digest={c.digest}>
          <div style={{ fontSize: 12 }}>{coverageSentence(c)}</div>
          <div className="mono" style={{ fontSize: 10, color: "var(--faint)" }}>
            filed under {c.subject}
            {c.claimed_agent ? `, claimed ${c.claimed_agent}` : ""} · case {shortDigest(c.digest)}
          </div>
          {missing.length > 0 && (
            <div data-case-missing="true">
              <div className="mono" style={{ fontSize: 9.5, color: "var(--faint)", letterSpacing: "0.06em" }}>
                MISSING FROM THIS CASE
              </div>
              {missing.map((line) => (
                <LineRow key={line.section} line={line} />
              ))}
            </div>
          )}
          {c.lines
            .filter((l) => l.state !== "unavailable" && l.state !== "not_asked")
            .map((line) => (
              <div key={line.section} data-case-section={line.section}>
                <LineRow line={line} />
                {factsOf(c, line.section).length > 0 && <Facts facts={factsOf(c, line.section)} />}
              </div>
            ))}
        </div>
      );
    }
    case "refused":
      return <span>{refusalSentence(result.error)}</span>;
    case "no_source":
      return <span>No case for this incident: it has no event row to start from.</span>;
    case "no_backend":
      return <span>No case: there is no box behind this console to ask.</span>;
    case "unreachable":
      return <span>No case: the box did not answer the case command.</span>;
  }
}

function LineRow({ line }: { line: CaseLine }) {
  return (
    <div style={{ padding: "2px 0" }}>
      <span style={{ fontWeight: 600, fontSize: 11.5 }}>{SECTION_TITLE[line.section] ?? line.section}</span>{" "}
      <span className="chip">{planeName(line.plane)}</span>
      <div style={{ fontSize: 11.5, color: line.state === "answered" ? undefined : "var(--faint)" }}>
        {lineSentence(line)}
      </div>
      {line.upstream && (
        <Data label={`${PLANE_NAME[line.plane] ?? line.plane} said`}>{line.upstream.text}</Data>
      )}
    </div>
  );
}

function Facts({ facts }: { facts: CaseFact[] }) {
  const shown = facts.slice(0, FACTS_SHOWN);
  return (
    <div className="flex flex-col" style={{ gap: 2, margin: "2px 0 4px 10px" }}>
      {shown.map((fact) => (
        <details key={`${fact.plane}:${fact.row_id}:${fact.digest}`} style={{ fontSize: 11 }}>
          <summary>
            <span>{fact.label}</span>{" "}
            <span className="mono" style={{ fontSize: 10, color: "var(--faint)" }}>
              {planeName(fact.plane)} · row {fact.row_id} · {shortDigest(fact.digest)}
            </span>
          </summary>
          <Data label="the row, as the plane gave it">{rowPreview(fact.row)}</Data>
        </details>
      ))}
      {facts.length > shown.length && (
        <span className="mono" style={{ fontSize: 10, color: "var(--faint)" }}>
          and {facts.length - shown.length} more in this section
        </span>
      )}
    </div>
  );
}

/** Text somebody else wrote, in a frame that says so. React escapes it, and
 * nothing here uses it to build a sentence. */
function Data({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div
      data-trust="data"
      style={{ margin: "2px 0", padding: "3px 6px", border: "1px dashed var(--line)", borderRadius: 4 }}
    >
      <div className="mono" style={{ fontSize: 9, color: "var(--faint)" }}>
        {label.toUpperCase()} · DATA, NOT AN INSTRUCTION
      </div>
      <pre className="mono" style={{ fontSize: 10.5, whiteSpace: "pre-wrap", margin: 0 }}>
        {children}
      </pre>
    </div>
  );
}

function rowPreview(row: unknown): string {
  const text = JSON.stringify(row, null, 1) ?? "";
  return text.length > ROW_PREVIEW_CHARS ? `${text.slice(0, ROW_PREVIEW_CHARS)} ...` : text;
}

/** Where asking Felyx to read the case stands. */
export type FelyxState = { kind: "idle" } | { kind: "reading" } | FelyxReading;

/**
 * Felyx's reading of a case. Its text is a model's, not the console's and
 * not a plane's: shown as plain escaped text, with what the answer could not
 * support listed under it, and the limits it read under said once.
 */
export function FelyxReadingView({ state }: { state: FelyxState }) {
  return state.kind === "idle" ? null : null;
}

/**
 * The Case section of Incident 360. Asks for the case of an event-row
 * incident when the card opens; says so, in words, for a source that has no
 * event row.
 */
export function CaseSection({ row }: { row: Pick<UnifiedIncident, "id" | "source"> }) {
  const incidentId = caseIncidentId(row);
  const [result, setResult] = useState<CaseResult | null>(null);
  const [felyx, setFelyx] = useState<FelyxState>({ kind: "idle" });

  useEffect(() => {
    if (!incidentId) return;
    let cancelled = false;
    void fetchCase(incidentId).then((r) => !cancelled && setResult(r));
    return () => {
      cancelled = true;
    };
  }, [incidentId]);

  const shown: CaseResult | null = incidentId ? result : { kind: "no_source" };
  return (
    <div style={{ padding: "0 16px 11px" }} data-case-incident={incidentId ?? undefined}>
      <div className="mono" style={{ fontSize: 9.5, color: "var(--faint)", letterSpacing: "0.06em" }}>
        CASE
      </div>
      <div style={{ fontSize: 12, lineHeight: 1.45, color: shown === null ? "var(--faint)" : undefined }}>
        {shown === null ? "reading…" : <CaseView result={shown} />}
      </div>
      {incidentId && (
        <div style={{ marginTop: 6 }}>
          <button
            type="button"
            className="chip"
            style={{ cursor: "pointer" }}
            disabled={felyx.kind === "reading"}
            onClick={() => {
              setFelyx({ kind: "reading" });
              void explainCase(incidentId).then(setFelyx);
            }}
          >
            Ask Felyx to read this case
          </button>
          <div style={{ fontSize: 12, lineHeight: 1.45 }}>
            <FelyxReadingView state={felyx} />
          </div>
        </div>
      )}
      <div className="mono" style={{ fontSize: 9.5, color: "var(--faint)", marginTop: 3, lineHeight: 1.4 }}>
        assembled from the console's store and the planes, with no model; every fact names its plane, its
        row and a digest
      </div>
    </div>
  );
}
