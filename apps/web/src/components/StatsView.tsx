import { useCallback, useEffect, useMemo, useState } from "react";
import { fetchStats, groupRows, sortRows } from "../lib/stats";
import type { GroupBy, SortKey, StatsRow } from "../lib/stats";
import { fetchRuns } from "../lib/money";
import { fetchIdentities } from "../lib/identity";
import { downloadCsv, downloadJson, type ExportMeta } from "../lib/download";
import { formatUsd } from "../lib/format";
import { prettyUnit } from "../lib/views";
import { Hero, HeroBand, KpiTile } from "./dash";
import { FreshBadge } from "./FreshBadge";
import type { AgentStats } from "../statsTypes";
import type { IdryxIdentity } from "../identityTypes";
import type { Run } from "../moneyTypes";

/** Statistics: the same estate cut three ways.
 *
 * # WHY THIS IS NOT THE OVERVIEW
 *
 * Overview answers "is anything on fire right now" with a top-20 spend bar and
 * an incident feed. This answers "who, and how much" and expects to be sorted
 * and exported. Neither is a better version of the other, and the console had
 * no place at all where every owner or every unit could be listed.
 *
 * # THE TWO WINDOWS ARE NEVER ADDED TOGETHER
 *
 * The money columns come from the Cloud and the count columns come from this
 * console's bus store, which is fresh per launch. They are different periods
 * over different stores, so the table shows them as two labelled groups and
 * never totals across them. The header states both windows, and so does every
 * export.
 *
 * # PARTIAL IS NOT EMPTY
 *
 * The three reads fail independently, and this view degrades one column group
 * at a time rather than collapsing to a single error. When the bus could not be
 * read the money columns still render and the count cells show a dash with a
 * banner saying why. A dash is "not measured"; a 0 would be a measurement
 * nobody took, and in the blocked column that reads as good news.
 */

const REFRESH_INTERVAL_MS = 30_000;

const GROUPS: { id: GroupBy; label: string }[] = [
  { id: "agent", label: "Agent" },
  { id: "owner", label: "Owner" },
  { id: "unit", label: "Business unit" },
];

interface Column {
  key: SortKey;
  header: string;
  /** Which window this column belongs to, which decides whether it renders as
   * a dash when that window could not be read. */
  band: "money" | "counts" | "key";
  align: "left" | "right";
}

const COLUMNS: Column[] = [
  { key: "label", header: "Name", band: "key", align: "left" },
  { key: "agentCount", header: "Agents", band: "key", align: "right" },
  { key: "spentUsd", header: "Spend", band: "money", align: "right" },
  { key: "calls", header: "Calls", band: "money", align: "right" },
  { key: "blocked", header: "Blocked", band: "counts", align: "right" },
  { key: "anomalies", header: "Odd behaviour", band: "counts", align: "right" },
  { key: "budgetEvents", header: "Budget events", band: "counts", align: "right" },
  { key: "worstOvershootMicrousd", header: "Worst breach", band: "counts", align: "right" },
];

function Banner({ tone, children }: { tone: "warn" | "info"; children: React.ReactNode }) {
  return (
    <div
      className="mono text-[11.5px] px-4 py-2"
      style={{
        color: tone === "warn" ? "var(--fg)" : "var(--dim)",
        background: "var(--panel-2)",
        borderBottom: "1px solid var(--line)",
        lineHeight: 1.7,
      }}
    >
      {children}
    </div>
  );
}

function NothingWasRead({ note }: { note: string }) {
  return (
    <div className="flex-1 min-h-0 flex items-center justify-center px-6">
      <div
        className="panel px-5 py-4 flex flex-col gap-2"
        style={{ background: "var(--panel-2)", maxWidth: 560 }}
      >
        <span style={{ fontSize: 13, color: "var(--fg)" }}>Nothing was read</span>
        <span className="mono text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.7 }}>
          {note}
        </span>
      </div>
    </div>
  );
}

/** Micro-USD as money, or a dash when nothing recorded it.
 *
 * `null` is not `$0.00`. Nothing on the bus said by how much this agent went
 * over, which is a different fact from it going over by nothing, and the Cloud
 * exports `budget_exhausted` without amounts so `null` is the ordinary case. */
function overshootCell(micro: number | null): string {
  if (micro === null) return "-";
  return formatUsd(micro / 1_000_000);
}

/** Money for an export cell: a plain number a spreadsheet can sum, rounded to
 * the sub-cent precision this data actually has.
 *
 * Rounded because summing floats leaves `175.24000000000004` in the file, and a
 * reader who sees that in a column of tidy numbers stops trusting the whole
 * sheet before they work out it is binary floating point. */
function usd4(value: number): number {
  return Math.round(value * 10_000) / 10_000;
}

export function StatsView() {
  const [runs, setRuns] = useState<Run[] | null>(null);
  const [identities, setIdentities] = useState<IdryxIdentity[] | null>(null);
  const [counts, setCounts] = useState<AgentStats[] | null>(null);
  const [countsNote, setCountsNote] = useState<string | null>(null);
  const [countsMeasured, setCountsMeasured] = useState(false);
  const [scanned, setScanned] = useState(0);
  const [at, setAt] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);

  const [group, setGroup] = useState<GroupBy>("agent");
  const [sortKey, setSortKey] = useState<SortKey>("spentUsd");
  const [desc, setDesc] = useState(true);

  const load = useCallback(async () => {
    // Settled, not `all`: one plane being down must not blank the other two.
    const [r, i, s] = await Promise.allSettled([fetchRuns(), fetchIdentities(), fetchStats()]);
    setRuns(r.status === "fulfilled" ? r.value : null);
    setIdentities(i.status === "fulfilled" ? i.value : null);
    if (s.status === "fulfilled") {
      setCounts(s.value.agents);
      setCountsMeasured(s.value.measured);
      setCountsNote(s.value.note);
      setScanned(s.value.scanned);
    } else {
      setCounts(null);
      setCountsMeasured(false);
      setCountsNote(
        "The event counts could not be read from this box, so the blocked, odd-behaviour " +
          "and budget columns are blank. This is not a report that your agents were never stopped.",
      );
      setScanned(0);
    }
    setAt(Date.now());
    setLoading(false);
  }, []);

  useEffect(() => {
    void load();
    const t = setInterval(() => void load(), REFRESH_INTERVAL_MS);
    return () => clearInterval(t);
  }, [load]);

  const rows = useMemo(() => {
    if (!runs && !counts) return [];
    return groupRows(runs ?? [], identities ?? [], counts ?? [], group);
  }, [runs, identities, counts, group]);

  const sorted = useMemo(() => sortRows(rows, sortKey, desc), [rows, sortKey, desc]);

  const meta = useCallback(
    (): ExportMeta => ({
      subject: `Genaryx statistics by ${group}`,
      environment: window.location.host || "unknown",
      takenAt: new Date().toISOString(),
      windows: [
        "spend and calls: the money plane's own window (TokenFuse Cloud)",
        countsMeasured
          ? `blocked, odd behaviour and budget events: the ${scanned} most recent events this console has ingested since it started`
          : "blocked, odd behaviour and budget events: NOT MEASURED, the event store could not be read",
      ],
      caveats: [
        ...(group === "owner"
          ? [
              "Owner comes from idryx. An agent idryx has no owner for is grouped under '(no owner in idryx)' rather than dropped.",
            ]
          : []),
        ...(group === "unit"
          ? [
              "Unit is derived from the team segment of the agent id, not from a separate org chart.",
            ]
          : []),
        "worst_breach_usd is the single worst breach, never a sum: one runaway run trips its breaker on every call.",
        "An empty worst_breach_usd means no event recorded the amounts, which is not the same as no overspend.",
      ],
    }),
    [group, countsMeasured, scanned],
  );

  const exportRows = useMemo(
    () =>
      sorted.map((r) => ({
        // The label as SHOWN, not the internal key: a spreadsheet row reading
        // "finops" when the screen it came from said "FinOps" is one more
        // thing for a reader to reconcile.
        name: group === "unit" && !r.unattributed ? prettyUnit(r.label) : r.label,
        agents: r.agentCount,
        spend_usd: runs ? usd4(r.spentUsd) : null,
        calls: runs ? r.calls : null,
        runs: runs ? r.runs : null,
        blocked: countsMeasured ? r.blocked : null,
        odd_behaviour: countsMeasured ? r.anomalies : null,
        budget_events: countsMeasured ? r.budgetEvents : null,
        worst_breach_usd:
          countsMeasured && r.worstOvershootMicrousd !== null
            ? usd4(r.worstOvershootMicrousd / 1_000_000)
            : null,
        unattributed: r.unattributed,
      })),
    [sorted, runs, countsMeasured, group],
  );

  const EXPORT_COLUMNS = useMemo(
    () =>
      [
        { key: "name" as const, header: "name" },
        { key: "agents" as const, header: "agents" },
        { key: "spend_usd" as const, header: "spend_usd" },
        { key: "calls" as const, header: "calls" },
        { key: "runs" as const, header: "runs" },
        { key: "blocked" as const, header: "blocked" },
        { key: "odd_behaviour" as const, header: "odd_behaviour" },
        { key: "budget_events" as const, header: "budget_events" },
        { key: "worst_breach_usd" as const, header: "worst_breach_usd" },
        { key: "unattributed" as const, header: "unattributed" },
      ],
    [],
  );

  if (loading) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        loading...
      </div>
    );
  }

  // Both windows dark. There is no table to draw and no honest zero to show.
  if (!runs && !countsMeasured) {
    return (
      <NothingWasRead
        note={
          countsNote ??
          "Neither the money plane nor the event store answered on this box, so there are no " +
            "numbers to show. This is not a report that your agents did nothing."
        }
      />
    );
  }

  const totalSpend = sorted.reduce((a, r) => a + r.spentUsd, 0);
  const totalBlocked = sorted.reduce((a, r) => a + r.blocked, 0);
  const totalAnomalies = sorted.reduce((a, r) => a + r.anomalies, 0);
  const unattributed = sorted.find((r) => r.unattributed);

  const setSort = (key: SortKey) => {
    if (key === sortKey) {
      setDesc(!desc);
      return;
    }
    setSortKey(key);
    setDesc(key !== "label");
  };

  return (
    <div className="flex-1 min-h-0 flex flex-col overflow-auto">
      <HeroBand
        hero={
          <Hero
            cap={`Statistics · by ${GROUPS.find((g) => g.id === group)?.label.toLowerCase()}`}
            value={sorted.length.toLocaleString("en-US")}
            sub={group === "agent" ? "agents" : group === "owner" ? "owners" : "units"}
            noteLeft={
              at ? <FreshBadge variant="auto" detail="30s" title={countsNote ?? undefined} /> : null
            }
            noteRight={
              countsMeasured
                ? `counts from the ${scanned.toLocaleString("en-US")} most recent bus events`
                : "counts not measured"
            }
          />
        }
        tiles={
          <>
            <KpiTile
              label="Spend"
              value={runs ? formatUsd(totalSpend) : "-"}
              sub={runs ? "money plane window" : "money plane did not answer"}
            />
            <KpiTile
              label="Blocked"
              value={countsMeasured ? totalBlocked.toLocaleString("en-US") : "-"}
              sub={countsMeasured ? "bus window" : "not measured"}
            />
            <KpiTile
              label="Odd behaviour"
              value={countsMeasured ? totalAnomalies.toLocaleString("en-US") : "-"}
              sub={countsMeasured ? "bus window" : "not measured"}
            />
            <KpiTile
              label="Rows"
              value={sorted.length.toLocaleString("en-US")}
              sub={unattributed ? "1 unattributed" : "all attributed"}
            />
          </>
        }
      />

      {!runs && (
        <Banner tone="warn">
          The money plane did not answer, so spend and calls are blank. The event counts below are
          unaffected.
        </Banner>
      )}
      {!countsMeasured && <Banner tone="warn">{countsNote}</Banner>}
      {group === "owner" && !identities && (
        <Banner tone="warn">
          The identity plane did not answer, so no agent could be matched to an owner and every row
          is under "(no owner in idryx)". This is not a report that these agents are unowned.
        </Banner>
      )}
      {group === "unit" && (
        <Banner tone="info">
          Unit is read from the team segment of each agent id, not from a separate org chart.
        </Banner>
      )}

      <div className="px-4 py-3 flex items-center gap-3 flex-wrap">
        <div className="flex items-center gap-1">
          {GROUPS.map((g) => (
            <button
              key={g.id}
              type="button"
              onClick={() => setGroup(g.id)}
              className="mono text-[11.5px] px-3 py-1 rounded"
              style={{
                background: group === g.id ? "var(--accent-dim)" : "var(--panel-2)",
                color: group === g.id ? "var(--fg)" : "var(--dim)",
                border: "1px solid var(--line)",
              }}
            >
              {g.label}
            </button>
          ))}
        </div>
        <span className="flex-1" />
        <button
          type="button"
          className="mono text-[11.5px] px-3 py-1 rounded"
          style={{ background: "var(--panel-2)", color: "var(--dim)", border: "1px solid var(--line)" }}
          onClick={() =>
            downloadCsv(`genaryx-statistics-by-${group}.csv`, EXPORT_COLUMNS, exportRows, meta())
          }
        >
          Export CSV
        </button>
        <button
          type="button"
          className="mono text-[11.5px] px-3 py-1 rounded"
          style={{ background: "var(--panel-2)", color: "var(--dim)", border: "1px solid var(--line)" }}
          onClick={() =>
            downloadJson(`genaryx-statistics-by-${group}.json`, exportRows, meta())
          }
        >
          Export JSON
        </button>
      </div>

      <div className="px-4 pb-6">
        <div className="panel overflow-x-auto" style={{ background: "var(--panel)" }}>
          <table className="w-full" style={{ borderCollapse: "collapse" }}>
            <thead>
              <tr>
                {COLUMNS.map((c) => (
                  <th
                    key={c.key}
                    onClick={() => setSort(c.key)}
                    className="mono text-[10.5px] uppercase px-3 py-2 select-none"
                    style={{
                      color: sortKey === c.key ? "var(--fg)" : "var(--faint)",
                      textAlign: c.align,
                      cursor: "pointer",
                      whiteSpace: "nowrap",
                      borderBottom: "1px solid var(--line)",
                    }}
                    title={
                      c.band === "money"
                        ? "money plane window"
                        : c.band === "counts"
                          ? "bus window: since this console started"
                          : undefined
                    }
                  >
                    {c.header}
                    {sortKey === c.key ? (desc ? " v" : " ^") : ""}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {sorted.map((r) => (
                <StatsTableRow
                  key={r.key}
                  row={r}
                  group={group}
                  hasMoney={!!runs}
                  hasCounts={countsMeasured}
                />
              ))}
            </tbody>
          </table>
          {sorted.length === 0 && (
            <div className="mono text-[11.5px] px-4 py-6" style={{ color: "var(--faint)" }}>
              no rows in either window
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function StatsTableRow({
  row,
  group,
  hasMoney,
  hasCounts,
}: {
  row: StatsRow;
  group: GroupBy;
  hasMoney: boolean;
  hasCounts: boolean;
}) {
  const num = (v: number, on: boolean) => (on ? v.toLocaleString("en-US") : "-");
  return (
    <tr style={{ borderTop: "1px solid var(--line)" }}>
      <td className="px-3 py-2" style={{ color: row.unattributed ? "var(--faint)" : "var(--fg)" }}>
        <span className="mono text-[11.5px]">
          {group === "unit" && !row.unattributed ? prettyUnit(row.label) : row.label}
        </span>
      </td>
      <td className="mono text-[11px] px-3 py-2 text-right" style={{ color: "var(--dim)" }}>
        {row.agentCount.toLocaleString("en-US")}
      </td>
      <td className="mono text-[11px] px-3 py-2 text-right" style={{ color: "var(--fg)" }}>
        {hasMoney ? formatUsd(row.spentUsd) : "-"}
      </td>
      <td className="mono text-[11px] px-3 py-2 text-right" style={{ color: "var(--dim)" }}>
        {num(row.calls, hasMoney)}
      </td>
      <td className="mono text-[11px] px-3 py-2 text-right" style={{ color: "var(--dim)" }}>
        {num(row.blocked, hasCounts)}
      </td>
      <td className="mono text-[11px] px-3 py-2 text-right" style={{ color: "var(--dim)" }}>
        {num(row.anomalies, hasCounts)}
      </td>
      <td className="mono text-[11px] px-3 py-2 text-right" style={{ color: "var(--dim)" }}>
        {num(row.budgetEvents, hasCounts)}
      </td>
      <td className="mono text-[11px] px-3 py-2 text-right" style={{ color: "var(--dim)" }}>
        {hasCounts ? overshootCell(row.worstOvershootMicrousd) : "-"}
      </td>
    </tr>
  );
}
