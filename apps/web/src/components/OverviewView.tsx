import { useCallback, useEffect, useMemo, useState } from "react";
import { cssVar } from "../lib/cssVars";
import {
  ackIncident,
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
import { HeroBand, Hero, KpiTile, DashMain, Section, Bars, Composition, Feed } from "./dash";
import type { BarItem, CompItem, FeedItem } from "./dash";
import { sevColor, sevRank, spendByAgent, spendSeries, usd0 } from "../lib/dashData";
import { agentBlockedStateFromRuns, StateBadge } from "../lib/lifecycle";
import { useConsoleStateVersion } from "../lib/consoleState";
import { usePopover } from "../lib/popover";
import { shortAgentLabel } from "../lib/graph";
import { prettyUnit, unitForTeam } from "../lib/views";
import { AgentDetailCard } from "./AgentDetailCard";
import { MetricDetailCard, type MetricRow } from "./MetricDetailCard";
import type { IdryxAlert } from "../identityTypes";
import { fetchAlerts } from "../lib/identity";
import { useIdentityStatus } from "../lib/useIdentityStatus";
import { aggregateIncidents, INCIDENT_SOURCE_LABEL, INCIDENT_SOURCE_VIEW, isQualityDriftEvent } from "../lib/incidents";
import { usePostureData } from "../lib/usePostureData";
import { fetchRecentEvents } from "../lib/recentEvents";
import { hasBackend, subscribeBackend } from "../lib/transport";
import type { UiEvent } from "../types";
import type { ViewId } from "../lib/views";
import { FreshBadge } from "./FreshBadge";

const REFRESH_INTERVAL_MS = 20_000;

/** Same cap `QualityDriftStream.tsx`/`DecisionStream.tsx` apply to their own
 * bus reads - the Incident Center only needs a recent window of quality
 * drift events, not the whole history. */
const BUS_FETCH_LIMIT = 500;

/** Rows shown on the Incident Center card (I2 spec's own number). */
const INCIDENT_CENTER_ROWS = 10;

export function OverviewView({
  onOpenAgent,
  onSelectView,
  onExplainIncident,
}: {
  onOpenAgent: (agentId: string) => void;
  /** Source-chip click on an Incident Center row - switches to that source's
   * own tab (`AppShell.tsx`'s existing view-switching mechanism), same
   * pattern as `Agent360.tsx`'s "Open <plane> panel" links. */
  onSelectView: (view: ViewId) => void;
  /** "Explain with Felyx" (C1, docs/PHASE6-C1.md) - identical wiring to
   * `MoneyView.tsx`'s own prop of the same name; only ever called for a
   * money-sourced Incident Center row (see `lib/incidents.ts`'s
   * `explainable` doc comment for why). */
  onExplainIncident: (incidentId: string) => void;
}) {
  const status = useMoneyStatus();
  const ready = status?.state === "ready";
  const { open } = usePopover();

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

  // Re-read the moment any lifecycle action lands anywhere (a Stop/Freeze/Kill
  // from the dock or a card), so the spend-by-agent bars and the KPI counts
  // reflect it within a beat rather than waiting out the 20s poll.
  const consoleVersion = useConsoleStateVersion();
  useEffect(() => {
    void refresh();
  }, [consoleVersion, refresh]);

  // I2 Incident Center's other three sources - each a fresh, independent
  // read this view owns itself (mirrors `PostureView.tsx`'s own "each view
  // owns its own reads" convention for the identical identity-alerts read).

  const identityStatus = useIdentityStatus();
  const [identityAlerts, setIdentityAlerts] = useState<IdryxAlert[]>([]);
  useEffect(() => {
    if (identityStatus?.state !== "ready") return;
    let cancelled = false;
    fetchAlerts()
      .then((a) => {
        if (!cancelled) setIdentityAlerts(a);
      })
      .catch(() => {
        if (!cancelled) setIdentityAlerts([]);
      });
    return () => {
      cancelled = true;
    };
  }, [identityStatus?.state]);

  // Quality drift: same source `QualityDriftStream.tsx` reads (the live bus,
  // filtered to `source === "verdryx" && type === "quality_drift"`), fetched
  // independently here rather than reaching into that component's state.
  const [qualityDriftEvents, setQualityDriftEvents] = useState<UiEvent[]>([]);
  useEffect(() => {
    let cancelled = false;
    void fetchRecentEvents(BUS_FETCH_LIMIT).then((res) => {
      if (!cancelled) setQualityDriftEvents(res.events.filter(isQualityDriftEvent));
    });
    return () => {
      cancelled = true;
    };
  }, []);
  useEffect(() => {
    if (!hasBackend()) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    subscribeBackend<UiEvent>("bus:event", (payload) => {
      if (!isQualityDriftEvent(payload)) return;
      setQualityDriftEvents((prev) => [payload, ...prev].slice(0, BUS_FETCH_LIMIT));
    })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch((err: unknown) => {
        // eslint-disable-next-line no-console
        console.error("subscribe(bus:event) failed (overview incident center):", err);
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // Posture findings: the SAME live hook `PostureView.tsx` renders from, so
  // a triggered zond shows up here automatically, no separate derivation.
  const posture = usePostureData();
  const postureFindings = useMemo(
    () => [...posture.stackFindings, ...posture.identityFindings, ...posture.connectionFindings],
    [posture.stackFindings, posture.identityFindings, posture.connectionFindings],
  );

  const unifiedIncidents = useMemo(
    () =>
      aggregateIncidents({
        moneyIncidents: incidents,
        identityAlerts,
        qualityDriftEvents,
        postureFindings,
      }).slice(0, INCIDENT_CENTER_ROWS),
    [incidents, identityAlerts, qualityDriftEvents, postureFindings],
  );

  const handleAckIncident = useCallback(
    async (id: string) => {
      try {
        await ackIncident(id);
        void refresh();
      } catch (err) {
        setError(err as MoneyError);
      }
    },
    [refresh],
  );

  const agents = useMemo(() => spendByAgent(runs), [runs]);
  // Per-agent blocked state (STOPPED/FROZEN/KILLED), so a halted agent's spend
  // bar carries the same badge it shows on every other panel.
  const blockedByAgent = useMemo(() => agentBlockedStateFromRuns(runs), [runs]);
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

  // One place that opens an agent's detail card beside whatever was clicked.
  const openAgent = (agentId: string, rect: DOMRect) =>
    open(<AgentDetailCard agentId={agentId} onOpenFull={onOpenAgent} />, { anchor: rect });

  const agentBars: BarItem[] = agents.slice(0, 20).map((a) => {
    const state = blockedByAgent.get(a.agent);
    return {
      key: a.agent,
      label: a.name,
      sub: prettyUnit(unitForTeam(a.team)),
      fraction: a.spent / maxAgent,
      tone: "amber",
      value: formatUsd(a.spent),
      badge: state ? <StateBadge state={state} /> : undefined,
      onClick: (rect) => openAgent(a.agent, rect),
    };
  });

  // Breakdown rows behind each headline number, so a clicked KPI opens the
  // agents/incidents/levers that make it up, each drillable in turn.
  const spendRows: MetricRow[] = agents.slice(0, 20).map((a) => ({
    key: a.agent,
    label: `${a.name} · ${prettyUnit(unitForTeam(a.team))}`,
    value: formatUsd(a.spent),
    agentId: a.agent,
  }));
  const callRows: MetricRow[] = [...runs]
    .sort((x, y) => y.calls - x.calls)
    .slice(0, 12)
    .map((r) => ({
      key: r.run_id,
      label: shortAgentLabel(r.agent_id),
      value: r.calls.toLocaleString("en-US"),
      agentId: r.agent_id,
    }));
  const incidentRows: MetricRow[] = topIncidents.map((inc) => ({
    key: inc.id,
    label: inc.kind.replace(/_/g, " "),
    value: inc.occurrences,
    valueColor: sevColor(inc.severity),
    agentId: inc.agent_id ?? undefined,
  }));
  const savingsRows: MetricRow[] = savings
    ? [
        { key: "blocked", label: "Runaway blocked", value: formatUsd(savings.blocked_spend_usd) },
        { key: "cache", label: "Semantic cache", value: formatUsd(savings.cache_saved_usd) },
        { key: "router", label: "Model router", value: formatUsd(savings.router_saved_usd) },
      ]
    : [];

  const compItems: CompItem[] = savings
    ? [
        { key: "blocked", label: "Runaway blocked", value: savings.blocked_spend_usd, total: saved, tone: "ember", valueText: formatUsd(savings.blocked_spend_usd) },
        { key: "cache", label: "Semantic cache", value: savings.cache_saved_usd, total: saved, tone: "mint", valueText: formatUsd(savings.cache_saved_usd) },
        { key: "router", label: "Model router", value: savings.router_saved_usd, total: saved, tone: "iris", valueText: formatUsd(savings.router_saved_usd) },
      ]
    : [];

  // I2 Incident Center: the top `unifiedIncidents` rows, rendered through
  // the same `Feed` primitive every other incidents list in this app uses.
  // The source chip is its OWN nested clickable button (mirrors
  // `Agent360.tsx`'s `AgentChip` - a bare `.chip` button, no dot, just
  // `cursor: pointer`), not the row's `onClick`: only money rows have a
  // natural "open agent" drill target, but every row's chip must navigate,
  // so the chip owns its own click and stops it from bubbling rather than
  // the row itself carrying one conditionally.
  const incidentCenterFeed: FeedItem[] = unifiedIncidents.map((row) => ({
    key: row.id,
    color: sevColor(row.severity),
    title: (
      <span className="flex items-center gap-2">
        <button
          type="button"
          className="chip"
          style={{ cursor: "pointer" }}
          title={`Open the ${INCIDENT_SOURCE_VIEW[row.source]} tab`}
          onClick={(e) => {
            e.stopPropagation();
            onSelectView(INCIDENT_SOURCE_VIEW[row.source]);
          }}
        >
          {INCIDENT_SOURCE_LABEL[row.source]}
        </button>
        <span className="truncate">{row.title}</span>
      </span>
    ),
    sub: row.detail,
    action:
      row.source === "money" ? (
        <span className="flex items-center gap-1.5">
          {row.explainable && (
            <button
              type="button"
              className="icon-btn"
              style={{ width: "auto", padding: "0 9px", fontSize: 11, color: "var(--iris)" }}
              title="Explain this incident with Felyx"
              onClick={() => onExplainIncident(row.raw.id)}
            >
              Explain
            </button>
          )}
          {row.ackable ? (
            <button
              type="button"
              className="icon-btn"
              style={{ width: "auto", padding: "0 9px", fontSize: 11 }}
              title="Acknowledge incident"
              onClick={() => void handleAckIncident(row.raw.id)}
            >
              Ack
            </button>
          ) : (
            <span className="mono" style={{ fontSize: 10, color: "var(--faint)" }}>
              ack'd
            </span>
          )}
        </span>
      ) : undefined,
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

      {error && (
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
                  onClick={(rect) =>
                    open(
                      <MetricDetailCard
                        kicker="Money"
                        title="Active runs"
                        value={overview.active_runs.toLocaleString("en-US")}
                        description={`Runs seen in the rolling window. ${overview.killed_runs} were killed by an operator; ${overview.total_runs.toLocaleString("en-US")} ran in total. Top spenders below.`}
                        rows={spendRows}
                        rowsTitle="by spend"
                        onOpenFullAgent={onOpenAgent}
                      />,
                      { anchor: rect },
                    )
                  }
                />
                <KpiTile
                  label="Governed saved"
                  value={formatUsd(saved)}
                  tone="var(--mint)"
                  sub="blocked · cache · router"
                  onClick={(rect) =>
                    open(
                      <MetricDetailCard
                        kicker="Money"
                        title="Governed saved"
                        value={formatUsd(saved)}
                        valueTone="var(--mint)"
                        description="Spend the governance layer prevented or recovered: budget breaks blocked before the provider was called, plus semantic cache and model-router savings."
                        rows={savingsRows}
                        rowsTitle="by lever"
                      />,
                      { anchor: rect },
                    )
                  }
                />
                <KpiTile
                  label="Open incidents"
                  value={overview.open_incidents}
                  tone={overview.open_incidents > 0 ? "var(--sev-high)" : undefined}
                  sub={`${overview.total_incidents} total detected`}
                  onClick={(rect) =>
                    open(
                      <MetricDetailCard
                        kicker="Incidents"
                        title="Open incidents"
                        value={overview.open_incidents}
                        valueTone={overview.open_incidents > 0 ? "var(--sev-high)" : undefined}
                        description={`Detector-raised incidents not yet acknowledged, out of ${overview.total_incidents} detected in this window. Worst first.`}
                        rows={incidentRows}
                        rowsTitle="worst first"
                        onOpenFullAgent={onOpenAgent}
                      />,
                      { anchor: rect },
                    )
                  }
                />
                <KpiTile
                  label="Model calls"
                  value={overview.total_calls.toLocaleString("en-US")}
                  sub="across the fleet"
                  onClick={(rect) =>
                    open(
                      <MetricDetailCard
                        kicker="Money"
                        title="Model calls"
                        value={overview.total_calls.toLocaleString("en-US")}
                        description="Metered calls the gateway forwarded or blocked across the fleet in this window, by agent."
                        rows={callRows}
                        rowsTitle="by calls"
                        onOpenFullAgent={onOpenAgent}
                      />,
                      { anchor: rect },
                    )
                  }
                />
              </>
            }
          />

          <DashMain
            primary={
              <Section title="Spend by agent" right={`top ${Math.min(20, agents.length)} of ${agents.length}`}>
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
                <Section
                  title="Incident center"
                  right={
                    <FreshBadge
                      variant="auto"
                      detail="20s"
                      title="Money incidents/runs poll every 20s; identity alerts + the posture identity snapshot load once and Refresh on their own panel; quality drift and the posture bus signals are live; the 9 plane-health checks each settle within seconds of connecting."
                    />
                  }
                >
                  <Feed items={incidentCenterFeed} empty="no incidents across money, identity, quality, or posture" />
                </Section>
              </>
            }
          />
        </>
      )}
    </div>
  );
}
