import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ackIncident,
  describeMoneyError,
  fetchIncidents,
  fetchRuns,
  fetchSavings,
  killRun,
  setBudget,
} from "../lib/money";
import { useMoneyStatus } from "../lib/useMoneyStatus";
import { formatUsd } from "../lib/format";
import { sevColor, sevRank, spendSeries, usd0 } from "../lib/dashData";
import type { Incident, MoneyError, MutationOutcome, Run, Savings } from "../moneyTypes";
import { MoneyEmptyState } from "./MoneyEmptyState";
import { RunsBoard } from "./RunsBoard";
import { UpsellBanner } from "./UpsellBanner";
import { HeroBand, Hero, KpiTile, DashMain, Section, Composition, Feed } from "./dash";
import type { CompItem, FeedItem } from "./dash";

const REFRESH_INTERVAL_MS = 20_000;
const RUNS_SHOWN = 18;

export function MoneyView({
  onOpenAgent,
  onOpenReplay,
}: {
  onOpenAgent: (agentId: string) => void;
  onOpenReplay: (runId: string) => void;
}) {
  const status = useMoneyStatus();
  const ready = status?.state === "ready";

  const [runs, setRuns] = useState<Run[] | null>(null);
  const [incidents, setIncidents] = useState<Incident[] | null>(null);
  const [savings, setSavings] = useState<Savings | null>(null);
  const [error, setError] = useState<MoneyError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!ready) return;
    try {
      const [r, i, s] = await Promise.all([fetchRuns(), fetchIncidents(), fetchSavings()]);
      setRuns(r);
      setIncidents(i);
      setSavings(s);
      setError(null);
    } catch (err) {
      setError(err as MoneyError);
    }
  }, [ready]);

  useEffect(() => {
    void refresh();
    const id = window.setInterval(() => void refresh(), REFRESH_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [refresh]);

  const afterMutation = useCallback(
    (outcome: MutationOutcome) => {
      setNotice(
        outcome.bus_recorded
          ? `${outcome.summary} - signed console_command recorded, visible in the Bus tab.`
          : `${outcome.summary} (bus journal not recorded: ${outcome.bus_error ?? "unknown reason"})`,
      );
      void refresh();
    },
    [refresh],
  );

  const handleKill = useCallback(
    async (runId: string, reason: string) => {
      afterMutation(await killRun(runId, reason));
    },
    [afterMutation],
  );
  const handleSetBudget = useCallback(
    async (runId: string, budgetUsd: number, reason: string) => {
      afterMutation(await setBudget(runId, budgetUsd, reason));
    },
    [afterMutation],
  );
  const handleAck = useCallback(
    async (id: string) => {
      afterMutation(await ackIncident(id));
    },
    [afterMutation],
  );

  const topRuns = useMemo(
    () =>
      (runs ?? [])
        .slice()
        .sort((a, b) => Number(a.killed) - Number(b.killed) || b.spent_usd - a.spent_usd)
        .slice(0, RUNS_SHOWN),
    [runs],
  );
  const series = useMemo(() => spendSeries(runs ?? []), [runs]);
  const topIncidents = useMemo(
    () =>
      (incidents ?? [])
        .slice()
        .sort((a, b) => sevRank(b.severity) - sevRank(a.severity) || b.occurrences - a.occurrences)
        .slice(0, 7),
    [incidents],
  );

  if (!ready) {
    return <MoneyEmptyState status={status} />;
  }

  const allRuns = runs ?? [];
  const totalSpent = allRuns.reduce((s, r) => s + r.spent_usd, 0);
  const totalCalls = allRuns.reduce((s, r) => s + r.calls, 0);
  const activeRuns = allRuns.filter((r) => !r.killed).length;
  const saved = savings?.total_saved_usd ?? 0;
  const gross = totalSpent + saved;
  const savePct = gross > 0 ? Math.round((saved / gross) * 100) : 0;
  const blocked = savings?.blocked_spend_usd ?? 0;
  const openIncidents = (incidents ?? []).filter((i) => !i.acknowledged).length;

  const compItems: CompItem[] = savings
    ? [
        { key: "blocked", label: "Runaway blocked", value: savings.blocked_spend_usd, total: saved, tone: "ember", valueText: formatUsd(savings.blocked_spend_usd) },
        { key: "cache", label: "Semantic cache", value: savings.cache_saved_usd, total: saved, tone: "mint", valueText: formatUsd(savings.cache_saved_usd) },
        { key: "router", label: "Model router", value: savings.router_saved_usd, total: saved, tone: "iris", valueText: formatUsd(savings.router_saved_usd) },
      ]
    : [];

  const incidentFeed: FeedItem[] = topIncidents.map((inc) => ({
    key: inc.id,
    color: sevColor(inc.severity),
    title: inc.kind.replace(/_/g, " "),
    sub: `${inc.run_id ?? inc.agent_id ?? "fleet"} · ${inc.occurrences}×`,
    onClick: inc.agent_id ? () => onOpenAgent(inc.agent_id as string) : undefined,
    action: inc.acknowledged ? (
      <span className="mono" style={{ fontSize: 10, color: "var(--faint)" }}>ack'd</span>
    ) : (
      <button
        type="button"
        className="icon-btn"
        style={{ width: "auto", padding: "0 9px", fontSize: 11 }}
        title="Acknowledge incident"
        onClick={() => void handleAck(inc.id)}
      >
        Ack
      </button>
    ),
  }));

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-4">
      {notice && (
        <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--mint)" }}>
          {notice}
        </div>
      )}
      {error && error.kind === "plan_required" && <UpsellBanner error={error} />}
      {error && error.kind !== "plan_required" && (
        <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
          {describeMoneyError(error)}
        </div>
      )}

      {runs === null ? (
        <div className="mono" style={{ fontSize: 12, color: "var(--faint)" }}>
          loading money plane...
        </div>
      ) : (
        <>
          <HeroBand
            hero={
              <Hero
                cap="AI spend · live fleet"
                value={usd0(totalSpent)}
                sub={
                  <>
                    governed savings <b>{formatUsd(saved)}</b>
                  </>
                }
                series={series}
                fuseFraction={gross > 0 ? saved / gross : 0}
                fuseTone="iris"
                noteLeft={
                  <>
                    prevented <b>{formatUsd(blocked)}</b> runaway spend
                  </>
                }
                noteRight={
                  <>
                    recovered <b>{savePct}%</b> of gross draw
                  </>
                }
              />
            }
            tiles={
              <>
                <KpiTile
                  label="Active runs"
                  value={activeRuns.toLocaleString("en-US")}
                  sub={`${allRuns.length.toLocaleString("en-US")} total in window`}
                />
                <KpiTile label="Model calls" value={totalCalls.toLocaleString("en-US")} sub="metered through gateway" />
                <KpiTile label="Governed saved" value={formatUsd(saved)} tone="var(--mint)" sub={`${savings?.budget_breaks ?? 0} budget breaks`} />
                <KpiTile
                  label="Open incidents"
                  value={openIncidents}
                  tone={openIncidents > 0 ? "var(--sev-high)" : undefined}
                  sub={`${(incidents ?? []).length} detected`}
                />
              </>
            }
          />

          <DashMain
            primary={
              <Section
                title="Runs"
                right={`top ${Math.min(RUNS_SHOWN, allRuns.length)} by spend · full stream in Bus`}
              >
                <RunsBoard
                  runs={topRuns}
                  onKill={handleKill}
                  onSetBudget={handleSetBudget}
                  onOpenAgent={onOpenAgent}
                  onReplayRun={onOpenReplay}
                />
              </Section>
            }
            rail={
              <>
                {savings && (
                  <Section title="Governed savings" right="prevented + recovered">
                    <Composition items={compItems} />
                  </Section>
                )}
                <Section title="Incidents" right="worst first">
                  <Feed items={incidentFeed} empty="no incidents" />
                </Section>
              </>
            }
          />
        </>
      )}
    </div>
  );
}
