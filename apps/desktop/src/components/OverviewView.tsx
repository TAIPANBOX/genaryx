import { useCallback, useEffect, useMemo, useState } from "react";
import { cssVar } from "../lib/cssVars";
import {
  describeMoneyError,
  fetchIncidents,
  fetchOverview,
  fetchRuns,
  fetchSavings,
} from "../lib/money";
import { useMoneyStatus } from "../lib/useMoneyStatus";
import { formatUsd } from "../lib/format";
import type { Incident, MoneyError, Overview, Run, Savings } from "../moneyTypes";
import { MoneyEmptyState } from "./MoneyEmptyState";
import { UpsellBanner } from "./UpsellBanner";
import { HeroBand, Hero, KpiTile, DashMain, Section, Bars, Composition, Feed } from "./dash";
import type { BarItem, CompItem, FeedItem } from "./dash";
import { sevColor, sevRank, spendByAgent, spendSeries, usd0 } from "../lib/dashData";

const REFRESH_INTERVAL_MS = 20_000;

export function OverviewView({ onOpenAgent }: { onOpenAgent: (agentId: string) => void }) {
  const status = useMoneyStatus();
  const ready = status?.state === "ready";

  const [overview, setOverview] = useState<Overview | null>(null);
  const [runs, setRuns] = useState<Run[]>([]);
  const [savings, setSavings] = useState<Savings | null>(null);
  const [incidents, setIncidents] = useState<Incident[]>([]);
  const [error, setError] = useState<MoneyError | null>(null);

  const refresh = useCallback(async () => {
    if (!ready) return;
    try {
      const [ov, rs, sv, inc] = await Promise.all([
        fetchOverview(),
        fetchRuns(),
        fetchSavings().catch(() => null),
        fetchIncidents().catch(() => [] as Incident[]),
      ]);
      setOverview(ov);
      setRuns(rs);
      setSavings(sv);
      setIncidents(inc);
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

  const agents = useMemo(() => spendByAgent(runs), [runs]);
  const series = useMemo(() => spendSeries(runs), [runs]);
  const topIncidents = useMemo(
    () =>
      [...incidents]
        .sort((a, b) => sevRank(b.severity) - sevRank(a.severity) || b.occurrences - a.occurrences)
        .slice(0, 6),
    [incidents],
  );

  if (!ready) {
    return <MoneyEmptyState status={status} />;
  }

  const saved = savings?.total_saved_usd ?? overview?.total_saved_usd ?? 0;
  const spent = overview?.total_spent_usd ?? 0;
  const gross = spent + saved;
  const savePct = gross > 0 ? Math.round((saved / gross) * 100) : 0;
  const blocked = savings?.blocked_spend_usd ?? 0;
  const maxAgent = Math.max(1, ...agents.map((a) => a.spent));
  const org = status.state === "ready" ? status.org_domain : "";

  const agentBars: BarItem[] = agents.slice(0, 8).map((a) => ({
    key: a.agent,
    label: a.name,
    sub: a.team,
    fraction: a.spent / maxAgent,
    tone: "amber",
    value: formatUsd(a.spent),
    onClick: () => onOpenAgent(a.agent),
  }));

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
    sub: `${inc.run_id ?? inc.agent_id ?? "fleet"} · ${inc.severity}`,
    value: inc.occurrences,
    valueColor: sevColor(inc.severity),
    onClick: inc.agent_id ? () => onOpenAgent(inc.agent_id as string) : undefined,
  }));

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-4">
      <div className="flex items-center gap-2 flex-wrap">
        <span className="chip" style={cssVar("dot", "var(--sev-low)")}>
          <span className="dot" aria-hidden="true" />
          {status.source.source === "taipan" ? `taipan up · ${status.source.name}` : "env fallback"}
        </span>
        <span className="mono" style={{ fontSize: 11, color: "var(--faint)" }}>
          {status.cloud_url}
          {org ? ` · ${org}` : ""}
        </span>
      </div>

      {error && error.kind === "plan_required" && <UpsellBanner error={error} />}
      {error && error.kind !== "plan_required" && (
        <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
          {describeMoneyError(error)}
        </div>
      )}

      {overview === null ? (
        <div className="mono" style={{ fontSize: 12, color: "var(--faint)" }}>
          loading control room...
        </div>
      ) : (
        <>
          <HeroBand
            hero={
              <Hero
                cap={`AI spend · rolling window${org ? ` · ${org}` : ""}`}
                value={usd0(spent)}
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
                    governance recovered <b>{savePct}%</b> of gross draw
                  </>
                }
              />
            }
            tiles={
              <>
                <KpiTile
                  label="Active runs"
                  value={overview.active_runs.toLocaleString("en-US")}
                  sub={`${overview.killed_runs} killed of ${overview.total_runs.toLocaleString("en-US")}`}
                />
                <KpiTile label="Governed saved" value={formatUsd(saved)} tone="var(--mint)" sub="blocked · cache · router" />
                <KpiTile
                  label="Open incidents"
                  value={overview.open_incidents}
                  tone={overview.open_incidents > 0 ? "var(--sev-high)" : undefined}
                  sub={`${overview.total_incidents} total detected`}
                />
                <KpiTile label="Model calls" value={overview.total_calls.toLocaleString("en-US")} sub="across the fleet" />
              </>
            }
          />

          <DashMain
            primary={
              <Section title="Spend by agent" right={`top ${Math.min(8, agents.length)} of ${agents.length}`}>
                <Bars items={agentBars} empty="no agent spend yet" />
              </Section>
            }
            rail={
              <>
                {savings && (
                  <Section title="Governed savings" right={`${savings.budget_breaks} budget breaks`}>
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
