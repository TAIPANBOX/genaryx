import { useCallback, useEffect, useState } from "react";
import { fetchEgress, navigationOnlyShare } from "../lib/egress";
import type { EgressError, EgressPanel, EgressRow } from "../egressTypes";
import { Hero, HeroBand, KpiTile, Section } from "./dash";
import { FreshBadge } from "./FreshBadge";
import { downloadCsv, downloadJson, type ExportMeta } from "../lib/download";

/** Web egress: what agents reached, and what was refused before they could.
 *
 * # WHY THIS IS NOT A BUS EXPLORER FILTER
 *
 * Every line here also reaches the Bus Explorer, which shows an event's data as
 * JSON and is right for "what happened at 14:32". This panel answers a
 * different question: is the fidelity of what my agents read actually what I
 * think it is. That needs four fields compared across rows, and a JSON blob
 * shows all four and compares none.
 *
 * # THE STATE THIS VIEW EXISTS TO RENDER HONESTLY
 *
 * `panel.measured === false` means nothing could be read. The rows are empty
 * and mean nothing, and this component renders the note INSTEAD of the table.
 * An empty table would say "your agents fetched nothing", which is the one
 * wrong answer that reads as good news.
 */

const REFRESH_INTERVAL_MS = 30_000;

/** Rows asked for. The backend reads a wider slice of the bus and keeps the
 * egress lines out of it, and it stops the moment it has this many, so this
 * number bounds the totals as well as the table. Named once because the
 * export has to state it. */
const ROW_LIMIT = 200;

function shortOrigin(origin: string): string {
  return origin.replace(/^https?:\/\//, "") || "(unrecorded)";
}

/** The refusal vocabulary, rendered as English where we know it and passed
 * through where we do not.
 *
 * Unknown verdicts are shown as they arrived rather than as "other". The
 * vocabulary belongs to the egress plane; a console that folded an unfamiliar
 * one into a bucket would hide the exact case somebody needs to see, which is a
 * refusal this build has never met. */
const VERDICT_ENGLISH: Record<string, string> = {
  deny_policy: "policy said no",
  deny_policy_unreachable: "policy could not be asked",
  deny_address: "the address is not somewhere an agent may reach",
  deny_host: "the host is refused",
  deny_scheme: "not http or https",
  deny_redirect_depth: "too many redirects",
  deny_rate: "the per-hour cap was spent",
};

function CouldNotLook({ note }: { note: string | null }) {
  return (
    <div className="flex-1 min-h-0 flex items-center justify-center px-6">
      <div className="panel px-5 py-4 flex flex-col gap-2" style={{ background: "var(--panel-2)", maxWidth: 560 }}>
        <span style={{ fontSize: 13, color: "var(--fg)" }}>Nothing was read</span>
        <span className="mono text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.7 }}>
          {note ??
            "The egress record could not be read, and this panel will not show an empty table in its place."}
        </span>
      </div>
    </div>
  );
}

/** One line of the table. Exported for its own test: this component is the
 * whole of what a reader is told about a fetch, and two of the fields on it
 * are ones where an empty cell would be read as a fact. */
export function EgressRowView({ r }: { r: EgressRow }) {
  const blocked = r.outcome === "blocked";
  return (
    <tr style={{ borderTop: "1px solid var(--line)" }}>
      <td className="mono text-[11px] px-3 py-2" style={{ color: "var(--faint)", whiteSpace: "nowrap" }}>
        {r.ts.replace("T", " ").replace(/\.\d+Z$/, "")}
      </td>
      <td className="px-3 py-2">
        <span
          className="mono text-[10.5px] px-1.5 py-0.5"
          style={{
            color: blocked ? "var(--bad)" : "var(--ok)",
            border: `1px solid ${blocked ? "var(--bad)" : "var(--ok)"}`,
            borderRadius: 3,
          }}
        >
          {blocked ? "refused" : "fetched"}
        </span>
      </td>
      {/* The digest rides on the origin cell rather than taking a column of
          its own: 96 hex characters is not something a person reads, it is
          something two records are compared by. Absent is said in words,
          because a blank here would read as "no url", and the record having
          no digest is a different fact from the fetch having no address. */}
      <td
        className="mono text-[11.5px] px-3 py-2"
        style={{ color: "var(--fg)" }}
        title={r.url_sha384 ? `url hash (sha384): ${r.url_sha384}` : "url hash: not recorded for this line"}
      >
        {shortOrigin(r.origin)}
      </td>
      <td className="mono text-[11px] px-3 py-2" style={{ color: "var(--dim)" }}>
        {r.agent_id}
      </td>
      {/* Which run reached for this, so a burst of refusals can be traced to
          the run that caused it rather than only to the agent. */}
      <td className="mono text-[11px] px-3 py-2" style={{ color: "var(--dim)" }}>
        {r.run_id ?? <span style={{ color: "var(--faint)" }}>no run recorded</span>}
      </td>
      <td className="mono text-[11px] px-3 py-2" style={{ color: "var(--dim)" }}>
        {blocked ? (
          <span title={r.reason ?? undefined}>
            {VERDICT_ENGLISH[r.verdict ?? ""] ?? r.verdict ?? "unrecorded"}
          </span>
        ) : (
          <>
            {r.backend ?? "unrecorded"}
            {r.enforcement === "navigation_only" && (
              <span
                className="ml-2"
                style={{ color: "var(--warn)" }}
                title="This backend fetched the page and handed it back. The navigation was decided; the requests the page then made were not."
              >
                navigation only
              </span>
            )}
          </>
        )}
      </td>
      <td className="mono text-[11px] px-3 py-2 text-right" style={{ color: "var(--faint)" }}>
        {r.content_bytes === null ? "" : r.content_bytes.toLocaleString()}
      </td>
    </tr>
  );
}

/** The columns of the saved file. Wider than the table on purpose: the table
 * is read by a person and drops what a person cannot use, while a file is
 * joined by a machine, and `url_sha384` is the field that makes this joinable
 * with any other record of the same fetch. */
export const EGRESS_EXPORT_COLUMNS: { key: keyof EgressRow & string; header: string }[] = [
  { key: "ts", header: "when" },
  { key: "outcome", header: "outcome" },
  { key: "origin", header: "origin" },
  { key: "agent_id", header: "agent" },
  { key: "run_id", header: "run" },
  { key: "backend", header: "backend" },
  { key: "enforcement", header: "enforcement" },
  { key: "content_bytes", header: "content_bytes" },
  { key: "verdict", header: "verdict" },
  { key: "reason", header: "reason" },
  { key: "url_sha384", header: "url_sha384" },
];

/**
 * Provenance for a saved egress table.
 *
 * The caveats are the part that matters, and the first one is not obvious
 * from the file: `egress_recent` breaks out of its loop the moment it has
 * `limit` rows, and it accumulates the totals inside that same loop. So the
 * header figures on the panel are the aggregate of exactly the rows in this
 * file, and nothing older was counted or listed. A reader who assumed the
 * totals were an estate-wide count and the rows a sample of it would have
 * both halves wrong.
 */
export function egressExportMeta(panel: EgressPanel, limit: number): ExportMeta {
  const capped = panel.rows.length >= limit;
  return {
    subject: "Web egress: what agents reached, and what was refused",
    environment: typeof window === "undefined" ? "unknown" : window.location.host || "unknown",
    takenAt: new Date().toISOString(),
    windows: [
      `rows: the ${panel.rows.length.toLocaleString("en-US")} most recent egress line(s) the backend returned`,
      panel.note ? `slice read: ${panel.note}` : "slice read: the backend did not say what it read",
    ],
    caveats: [
      "The panel's totals are the aggregate of exactly these rows. The backend stops as soon as it has enough rows and counts nothing after that, so these are not totals for all egress on this box.",
      ...(capped
        ? [
            `The row cap of ${limit} was reached, so there is older egress this file does not carry and those totals never counted.`,
          ]
        : []),
      "origin is scheme and host. The path and query string of a fetched URL were never written to the record, so no export can carry them.",
      "An empty run or url_sha384 cell means the record carried none. That is not the same as there being none.",
      ...(panel.totals.subresources_unknown > 0
        ? [
            `${panel.totals.subresources_unknown.toLocaleString("en-US")} fetch(es) had a backend that could not say what the page then requested. Those requests are in no row here.`,
          ]
        : []),
      ...(panel.totals.navigation_only > 0
        ? [
            `${panel.totals.navigation_only.toLocaleString("en-US")} fetch(es) went through a backend that governs the navigation only. What those pages then loaded was decided by nothing and is in no row here.`,
          ]
        : []),
    ],
  };
}

function ExportButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      type="button"
      className="mono text-[11.5px] px-3 py-1 rounded"
      style={{ background: "var(--panel-2)", color: "var(--dim)", border: "1px solid var(--line)" }}
      onClick={onClick}
    >
      {label}
    </button>
  );
}

export function EgressView() {
  const [panel, setPanel] = useState<EgressPanel | null>(null);
  const [error, setError] = useState<EgressError | null>(null);
  const [at, setAt] = useState<number | null>(null);
  // Explicit, rather than inferred from `panel === null`. Those are two
  // different states, and conflating them is what held this view on
  // "loading..." for ever the first time the backend answered something it
  // could not read.
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    try {
      const p = await fetchEgress(ROW_LIMIT);
      setPanel(p);
      setError(null);
      setAt(Date.now());
    } catch (e) {
      setError(e as EgressError);
      setPanel(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
    const t = setInterval(() => void load(), REFRESH_INTERVAL_MS);
    return () => clearInterval(t);
  }, [load]);

  if (error) {
    return (
      <CouldNotLook
        note={
          error.kind === "no_environment"
            ? "There is no backend to ask. This panel has no demo data, deliberately: a plausible list of web requests that never happened is the one thing it must never show."
            : `The backend refused the request: ${error.message}`
        }
      />
    );
  }
  if (loading) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        loading...
      </div>
    );
  }
  if (!panel) {
    return <CouldNotLook note={null} />;
  }
  if (!panel.measured) {
    return <CouldNotLook note={panel.note} />;
  }

  const share = navigationOnlyShare(panel);
  const verdicts = Object.entries(panel.totals.by_verdict).sort((a, b) => b[1] - a[1]);

  return (
    <div className="flex-1 min-h-0 flex flex-col overflow-auto">
      <HeroBand
        hero={
          <Hero
            cap="Web egress · what agents reached"
            value={(panel.totals.fetched + panel.totals.blocked).toLocaleString("en-US")}
            sub={
              <>
                {panel.totals.fetched.toLocaleString("en-US")} fetched,{" "}
                {panel.totals.blocked.toLocaleString("en-US")} refused
              </>
            }
            noteLeft={at ? <FreshBadge variant="auto" detail="30s" title={panel.note ?? undefined} /> : null}
          />
        }
        tiles={
          <>
            <KpiTile label="Fetched" value={panel.totals.fetched.toLocaleString("en-US")} />
            <KpiTile label="Refused" value={panel.totals.blocked.toLocaleString("en-US")} />
            {/* "n/a", not 0%, when nothing was fetched. A dial reading 0%
                beside "0 fetches" is remembered as "everything was fully
                enforced", which is a conclusion drawn from an absent number. */}
            <KpiTile
              label="Governed at navigation only"
              value={share === null ? "n/a" : `${Math.round(share * 100)}%`}
              tone={share !== null && share > 0 ? "var(--warn)" : undefined}
              sub={share === null ? "no fetches to measure" : undefined}
            />
            <KpiTile
              label="Subresources unknown"
              value={panel.totals.subresources_unknown.toLocaleString("en-US")}
              tone={panel.totals.subresources_unknown > 0 ? "var(--warn)" : undefined}
              sub="the backend could not say"
            />
          </>
        }
      />

      {panel.totals.navigation_only > 0 && (
        <Section title="What “navigation only” means here">
          <p className="mono text-[11.5px] px-4 pb-3" style={{ color: "var(--dim)", lineHeight: 1.8 }}>
            {panel.totals.navigation_only} of {panel.totals.fetched} fetches went through a backend that
            reports only the page it was asked for. The destination was decided before the request left, so
            the navigation was governed. What that page then loaded, its images, fonts, scripts and
            background requests, was not: they were fetched by that service, with nothing in between. This is
            not a fault, it is what wrapping a tool you already run buys and does not buy, and it is stated
            here rather than in a footnote because the alternative is believing every request was policed.
          </p>
        </Section>
      )}

      {verdicts.length > 0 && (
        <Section title="Why fetches were refused">
          <div className="flex flex-wrap gap-2 px-4 pb-3">
            {verdicts.map(([v, n]) => (
              <span
                key={v}
                className="mono text-[11px] px-2 py-1"
                style={{ background: "var(--panel-2)", color: "var(--dim)", borderRadius: 3 }}
              >
                {VERDICT_ENGLISH[v] ?? v}
                <span style={{ color: "var(--fg)" }}> {n}</span>
              </span>
            ))}
          </div>
        </Section>
      )}

      <Section
        title="Recent"
        right={
          <span className="flex items-center gap-2">
            <ExportButton
              label="Export CSV"
              onClick={() =>
                downloadCsv("genaryx-web-egress.csv", EGRESS_EXPORT_COLUMNS, panel.rows, egressExportMeta(panel, ROW_LIMIT))
              }
            />
            <ExportButton
              label="Export JSON"
              onClick={() => downloadJson("genaryx-web-egress.json", panel.rows, egressExportMeta(panel, ROW_LIMIT))}
            />
          </span>
        }
      >
        <table className="w-full" style={{ borderCollapse: "collapse" }}>
          <thead>
            <tr className="mono text-[10.5px]" style={{ color: "var(--faint)", textAlign: "left" }}>
              <th className="px-3 py-2">when</th>
              <th className="px-3 py-2">outcome</th>
              <th className="px-3 py-2">origin</th>
              <th className="px-3 py-2">agent</th>
              <th className="px-3 py-2">run</th>
              <th className="px-3 py-2">backend / why</th>
              <th className="px-3 py-2 text-right">bytes</th>
            </tr>
          </thead>
          <tbody>
            {panel.rows.map((r, i) => (
              <EgressRowView key={`${r.ts}-${i}`} r={r} />
            ))}
          </tbody>
        </table>
        {panel.rows.length === 0 && (
          <p className="mono text-[11.5px] px-4 py-4" style={{ color: "var(--dim)", lineHeight: 1.7 }}>
            No egress events in the window read. {panel.note}
          </p>
        )}
        {/* The origin column is scheme and host, and that is all the record
            holds. The path and the query string were never written: a URL is
            personal data, and the plane that recorded this deliberately never
            assembled them into the event. Said here so nobody looks for a
            "show full URL" control that would have to invent one. */}
        <p className="mono text-[11px] px-4 py-3" style={{ color: "var(--faint)", lineHeight: 1.7 }}>
          Origins only. The path and query string of a fetched URL are never written to the record: they are
          where an identifier or a session token lives, and the event is the part designed to be kept.
        </p>
      </Section>
    </div>
  );
}
