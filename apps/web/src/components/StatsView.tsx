import { useCallback, useEffect, useMemo, useState } from "react";
import { fetchStats, groupRows, rowsFromOwners, sortRows } from "../lib/stats";
import type { GroupBy, SortKey, StatsRow } from "../lib/stats";
import { fetchOwners, fetchRuns } from "../lib/money";
import { fetchIdentities } from "../lib/identity";
import { downloadCsv, downloadJson, type ExportMeta } from "../lib/download";
import { formatUsd } from "../lib/format";
import { prettyUnit } from "../lib/views";
import { Hero, HeroBand, KpiTile } from "./dash";
import { FreshBadge } from "./FreshBadge";
import { useConsoleStateVersion } from "../lib/consoleState";
import type { AgentStats } from "../statsTypes";
import type { IdryxIdentity } from "../identityTypes";
import type { Owner, Run } from "../moneyTypes";

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

/** The windows offered, and why these four.
 *
 * Rolling rather than calendar: "the last 7 days" is what an operator means
 * when something looks wrong now, and a calendar week resets to almost nothing
 * every Monday. `0` is the honest option for a box whose history is younger
 * than any of them, and stays the default so the first render of a fresh
 * install shows what it has rather than an empty 24 hours. */
const WINDOWS: { days: number; label: string }[] = [
  { days: 0, label: "All held" },
  { days: 1, label: "24h" },
  { days: 7, label: "7d" },
  { days: 30, label: "30d" },
  { days: 365, label: "1y" },
];

const GROUPS: { id: GroupBy; label: string; title: string }[] = [
  { id: "agent", label: "Agent", title: "One row per agent" },
  {
    id: "owner",
    label: "Owner",
    title: "Who OWNS the agent, from the identity plane. Accountability.",
  },
  {
    id: "launcher",
    label: "Ran on behalf of",
    title:
      "Who STARTED the run, from the delegation chain the money plane recorded. Not the same question as Owner, and deliberately not merged with it.",
  },
  { id: "unit", label: "Business unit", title: "Derived from the agent id's team segment" },
];

interface Column {
  key: SortKey;
  header: string;
  /** Which window this column belongs to, which decides whether it renders as
   * a dash when that window could not be read. */
  band: "money" | "counts" | "key";
  align: "left" | "right";
  /** Overrides the band's own tooltip where the column needs to say something
   * more specific than which window it came from. */
  title?: string;
}

const COLUMNS: Column[] = [
  { key: "label", header: "Name", band: "key", align: "left" },
  {
    key: "agentCount",
    header: "Agents",
    band: "key",
    align: "right",
    // Says what it counts, because the detail card behind the row counts
    // something else. This is agents that appear in one of the two windows;
    // the unit and owner cards list the full roster, including agents that
    // have not run. Both numbers are right and they are not the same number.
    title: "Agents seen in these windows, not the full roster the detail card lists",
  },
  { key: "spentUsd", header: "Spend", band: "money", align: "right" },
  { key: "calls", header: "Calls", band: "money", align: "right" },
  {
    key: "blocked",
    header: "Blocked",
    band: "counts",
    align: "right",
    title:
      "Stops by our own services: a policy denial, a tripped breaker, a DLP or taint block, a refused fetch. An operator freeze is enforced as an ordinary policy, so its refusals land here too and are not attributed to the person.",
  },
  {
    key: "blockedByOperator",
    header: "By operator",
    band: "counts",
    align: "right",
    title:
      "Of those, the ones a person caused: a kill naming an actor, or a hold a human denied. Everything else was the services acting on their own.",
  },
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

export function StatsView({
  onOpenAgent,
  onOpenUser,
  onOpenUnit,
}: {
  /** Opens the full Agent 360 view, the same one the Overview bars open. */
  onOpenAgent: (agentId: string) => void;
  /** Opens the owner card: every agent they own and what those agents spend. */
  onOpenUser: (handle: string) => void;
  /** Opens the business-unit card. */
  onOpenUnit: (unitId: string) => void;
}) {
  const [runs, setRuns] = useState<Run[] | null>(null);
  const [owners, setOwners] = useState<Owner[] | null>(null);
  const [identities, setIdentities] = useState<IdryxIdentity[] | null>(null);
  const [counts, setCounts] = useState<AgentStats[] | null>(null);
  const [countsNote, setCountsNote] = useState<string | null>(null);
  const [countsMeasured, setCountsMeasured] = useState(false);
  const [scanned, setScanned] = useState(0);
  const [at, setAt] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);

  const [group, setGroup] = useState<GroupBy>("agent");
  const [windowDays, setWindowDays] = useState(0);
  // The window the backend ANSWERED for, which is not always the one asked
  // for: the preview holds one session's events and says so rather than
  // pretending it filtered a month it never had. Every label reads this, so
  // the caption can never describe a window the numbers did not come from.
  const [answeredWindow, setAnsweredWindow] = useState(0);
  const [historyFrom, setHistoryFrom] = useState<string | null>(null);
  const [undated, setUndated] = useState(0);
  const [sortKey, setSortKey] = useState<SortKey>("spentUsd");
  const [desc, setDesc] = useState(true);

  const load = useCallback(async () => {
    // Settled, not `all`: one plane being down must not blank the other two.
    const [r, i, s, o] = await Promise.allSettled([
      fetchRuns(),
      fetchIdentities(),
      fetchStats(undefined, windowDays),
      // A box whose money plane predates tokenfuse #192 has no `/v1/owners`,
      // and that is a missing grouping rather than a broken panel: settled,
      // like the other three.
      fetchOwners(),
    ]);
    setRuns(r.status === "fulfilled" ? r.value : null);
    setOwners(o.status === "fulfilled" ? o.value : null);
    setIdentities(i.status === "fulfilled" ? i.value : null);
    if (s.status === "fulfilled") {
      setCounts(s.value.agents);
      setCountsMeasured(s.value.measured);
      setCountsNote(s.value.note);
      setScanned(s.value.scanned);
      setAnsweredWindow(s.value.window_days ?? 0);
      setHistoryFrom(s.value.history_from ?? null);
      setUndated(s.value.undated ?? 0);
    } else {
      setCounts(null);
      setCountsMeasured(false);
      setCountsNote(
        "The event counts could not be read from this box, so the blocked, odd-behaviour " +
          "and budget columns are blank. This is not a report that your agents were never stopped.",
      );
      setScanned(0);
      setAnsweredWindow(0);
      setHistoryFrom(null);
      setUndated(0);
    }
    setAt(Date.now());
    setLoading(false);
  }, [windowDays]);

  // Re-read on any lifecycle action anywhere in the app, not just on the
  // timer. Freezing an agent from this table's own drill-down and watching its
  // row sit unchanged for half a minute reads as the freeze not having worked.
  const consoleVersion = useConsoleStateVersion();

  useEffect(() => {
    void load();
    const t = setInterval(() => void load(), REFRESH_INTERVAL_MS);
    return () => clearInterval(t);
  }, [load, consoleVersion]);

  const rows = useMemo(() => {
    // The money plane aggregated this one itself; folding it again console-side
    // would be a second number for the same question.
    if (group === "launcher") return owners ? rowsFromOwners(owners) : [];
    if (!runs && !counts) return [];
    return groupRows(runs ?? [], identities ?? [], counts ?? [], group);
  }, [runs, identities, counts, owners, group]);

  const sorted = useMemo(() => sortRows(rows, sortKey, desc), [rows, sortKey, desc]);

  const meta = useCallback(
    (): ExportMeta => ({
      subject: `Genaryx statistics by ${group === "launcher" ? "who ran it (delegation chain)" : group}`,
      environment: window.location.host || "unknown",
      takenAt: new Date().toISOString(),
      windows: [
        "spend and calls: the money plane's own window (TokenFuse Cloud)",
        countsMeasured
          ? (answeredWindow === 0
              ? `blocked, odd behaviour and budget events: ${scanned} event(s), every age this box still holds`
              : `blocked, odd behaviour and budget events: ${scanned} event(s) from the last ${answeredWindow} day(s), by each event's own timestamp`)
          : "blocked, odd behaviour and budget events: NOT MEASURED, the event store could not be read",
      ],
      caveats: [
        ...(historyFrom ? [`This box's event history starts at ${historyFrom}; a window longer than that is everything there is, not a quiet period.`] : []),
        ...(undated > 0 ? [`${undated} stored event(s) carry a timestamp this build cannot read and are in no window.`] : []),
        ...(group === "launcher"
          ? [
              "Owner here is the root of the delegation chain: who STARTED the run, as the money plane recorded it. This is NOT the same figure as the Owner grouping, which names who owns the agent.",
              "The blocked, odd-behaviour and budget columns are empty at this grouping: those counts are per agent, and this rollup has no agent list to join them to.",
            ]
          : []),
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
        "blocked counts every stop; blocked_by_operator is the subset a person caused. An operator freeze is enforced as an ordinary policy, so its refusals are counted as the system's.",
        "worst_breach_usd is the single worst breach, never a sum: one runaway run trips its breaker on every call.",
        "An empty worst_breach_usd means no event recorded the amounts, which is not the same as no overspend.",
      ],
    }),
    [group, countsMeasured, scanned, answeredWindow, historyFrom, undated],
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
        blocked: countsMeasured && r.countsApply ? r.blocked : null,
        blocked_by_operator: countsMeasured && r.countsApply ? r.blockedByOperator : null,
        odd_behaviour: countsMeasured && r.countsApply ? r.anomalies : null,
        budget_events: countsMeasured && r.countsApply ? r.budgetEvents : null,
        worst_breach_usd:
          countsMeasured && r.countsApply && r.worstOvershootMicrousd !== null
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
        { key: "blocked_by_operator" as const, header: "blocked_by_operator" },
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
  // Only sum the counts where they mean something. At a grouping with nothing
  // to join them to, the honest tile is a dash, not a confident zero in the
  // blocked column.
  const countsApply = sorted.length > 0 && sorted.every((r) => r.countsApply);
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
            sub={
              group === "agent"
                ? "agents"
                : group === "owner"
                  ? "owners"
                  : group === "launcher"
                    ? "people who ran something"
                    : "units"
            }
            noteLeft={
              at ? <FreshBadge variant="auto" detail="30s" title={countsNote ?? undefined} /> : null
            }
            noteRight={
              countsMeasured
                ? answeredWindow === 0
                  ? `${scanned.toLocaleString("en-US")} bus events, every age held`
                  : `${scanned.toLocaleString("en-US")} bus events in the last ${answeredWindow}d`
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
              value={countsMeasured && countsApply ? totalBlocked.toLocaleString("en-US") : "-"}
              sub={
                !countsMeasured
                  ? "not measured"
                  : countsApply
                    ? "bus window"
                    : "not countable at this grouping"
              }
            />
            <KpiTile
              label="Odd behaviour"
              value={countsMeasured && countsApply ? totalAnomalies.toLocaleString("en-US") : "-"}
              sub={
                !countsMeasured
                  ? "not measured"
                  : countsApply
                    ? "bus window"
                    : "not countable at this grouping"
              }
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
      {countsMeasured && windowDays !== answeredWindow && (
        <Banner tone="info">
          This box answered for {answeredWindow === 0 ? "every event it holds" : `the last ${answeredWindow} day(s)`}, not
          the {windowDays}-day window asked for. {countsNote}
        </Banner>
      )}
      {group === "owner" && !identities && (
        <Banner tone="warn">
          The identity plane did not answer, so no agent could be matched to an owner and every row
          is under "(no owner in idryx)". This is not a report that these agents are unowned.
        </Banner>
      )}
      {group === "launcher" && (
        <Banner tone="info">
          Who <strong>started</strong> the run, from the delegation chain the money plane recorded.
          The Owner grouping beside it answers a different question, who <strong>owns</strong> the
          agent, from the identity plane. An agent owned by one person and run on another's behalf
          appears under both names, and neither number is wrong. They are not added together
          anywhere.
        </Banner>
      )}
      {group === "launcher" && !owners && (
        <Banner tone="warn">
          The money plane did not answer for this grouping. A box running a Cloud older than the
          per-person rollup has no such record at all; this is not a report that nobody ran
          anything.
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
              title={g.title}
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
        <div className="flex items-center gap-1">
          {WINDOWS.map((w) => (
            <button
              key={w.days}
              type="button"
              onClick={() => setWindowDays(w.days)}
              className="mono text-[11.5px] px-3 py-1 rounded"
              title={
                w.days === 0
                  ? "Every event this box still holds, of any age"
                  : `Events from the last ${w.days} day(s), by each event's own timestamp`
              }
              style={{
                background: windowDays === w.days ? "var(--accent-dim)" : "var(--panel-2)",
                color: windowDays === w.days ? "var(--fg)" : "var(--dim)",
                border: "1px solid var(--line)",
              }}
            >
              {w.label}
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
                      c.title ??
                      (c.band === "money"
                        ? "money plane window"
                        : c.band === "counts"
                          ? "bus window: since this console started"
                          : undefined)
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
                  onOpen={
                    // The unattributed row has nothing to open: there is no
                    // owner and no unit behind it, only the agents that lacked
                    // one. A control that looked clickable and did nothing
                    // would suggest the console knows something it does not.
                    r.unattributed
                      ? undefined
                      : group === "agent"
                        ? () => onOpenAgent(r.key)
                        : group === "owner"
                          ? () => onOpenUser(r.key)
                          : group === "launcher"
                            // The owner card is keyed on the handle the console
                            // pins people under, which is the last path segment
                            // of the `user://` principal this rollup returns.
                            ? () => onOpenUser(r.key.split("/").filter(Boolean).pop() ?? r.key)
                            : () => onOpenUnit(r.key)
                  }
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
  onOpen,
}: {
  row: StatsRow;
  group: GroupBy;
  hasMoney: boolean;
  hasCounts: boolean;
  /** Opens whatever this row is: the agent, the owner, or the unit. Absent for
   * the unattributed row, which is not a thing that can be opened. */
  onOpen?: () => void;
}) {
  const num = (v: number, on: boolean) => (on ? v.toLocaleString("en-US") : "-");
  // A dash where a COUNT would be a claim nobody measured: either the bus could
  // not be read at all, or this grouping has nothing to join per-agent counts
  // to (see `StatsRow.countsApply`). Deliberately separate from `num`: `calls`
  // is a money-plane column and this must never blank it, which the first cut
  // of this did.
  const count = (v: number) => (hasCounts && row.countsApply ? v.toLocaleString("en-US") : "-");
  const shown = group === "unit" && !row.unattributed ? prettyUnit(row.label) : row.label;
  return (
    <tr style={{ borderTop: "1px solid var(--line)" }}>
      <td className="px-3 py-2" style={{ color: row.unattributed ? "var(--faint)" : "var(--fg)" }}>
        {onOpen ? (
          // A button rather than a click handler on the cell: this is a real
          // control, so it takes focus, answers Enter and Space, and reads as
          // one to a screen reader. The whole row is deliberately NOT the
          // target - a table whose every cell navigates makes selecting a
          // number to copy impossible.
          <button
            type="button"
            onClick={onOpen}
            className="mono text-[11.5px] statslink"
            title={
              group === "agent"
                ? `Open ${row.key}`
                : group === "owner"
                  ? `Open ${row.label}, and every agent they own`
                  : `Open the ${shown} unit`
            }
          >
            {shown}
          </button>
        ) : (
          <span className="mono text-[11.5px]">{shown}</span>
        )}
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
        {count(row.blocked)}
      </td>
      <td
        className="mono text-[11px] px-3 py-2 text-right"
        style={{ color: row.blockedByOperator > 0 ? "var(--fg)" : "var(--faint)" }}
      >
        {count(row.blockedByOperator)}
      </td>
      <td className="mono text-[11px] px-3 py-2 text-right" style={{ color: "var(--dim)" }}>
        {count(row.anomalies)}
      </td>
      <td className="mono text-[11px] px-3 py-2 text-right" style={{ color: "var(--dim)" }}>
        {count(row.budgetEvents)}
      </td>
      <td className="mono text-[11px] px-3 py-2 text-right" style={{ color: "var(--dim)" }}>
        {hasCounts && row.countsApply ? overshootCell(row.worstOvershootMicrousd) : "-"}
      </td>
    </tr>
  );
}
