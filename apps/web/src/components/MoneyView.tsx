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
import { useSession } from "../lib/useSession";
import { formatUsd } from "../lib/format";
import { sevColor, sevRank } from "../lib/dashData";
import type { Incident, MoneyError, MutationOutcome, Run, Savings } from "../moneyTypes";
import { MoneyEmptyState } from "./MoneyEmptyState";
import { RunsBoard } from "./RunsBoard";
import { UpsellBanner } from "./UpsellBanner";
import { DashMain, Section, Composition, Feed } from "./dash";
import type { CompItem, FeedItem } from "./dash";
import { usePopover } from "../lib/popover";
import { AgentDetailCard } from "./AgentDetailCard";
import { SortBar, type SortDir } from "./SortBar";

const RUN_SORTS = [
  { key: "spend", label: "spend" },
  { key: "calls", label: "calls" },
  { key: "utilisation", label: "utilisation" },
  { key: "agent", label: "agent" },
  { key: "status", label: "status" },
];

function cmpRun(a: Run, b: Run, key: string): number {
  if (key === "calls") return a.calls - b.calls;
  if (key === "utilisation") {
    const ua = a.budget_usd ? a.spent_usd / a.budget_usd : 0;
    const ub = b.budget_usd ? b.spent_usd / b.budget_usd : 0;
    return ua - ub;
  }
  if (key === "agent") return a.agent_id.localeCompare(b.agent_id);
  if (key === "status") return Number(a.killed) - Number(b.killed);
  return a.spent_usd - b.spent_usd; // spend (default)
}

const REFRESH_INTERVAL_MS = 20_000;
const RUNS_SHOWN = 18;

export function MoneyView({
  onOpenAgent,
  onOpenReplay,
  onExplainIncident,
}: {
  onOpenAgent: (agentId: string) => void;
  onOpenReplay: (runId: string) => void;
  /** "Explain with Felyx" (C1, docs/PHASE6-C1.md): hands the incident id up
   * to `AppShell`, which switches to the Copilot pane and seeds it with a
   * `copilot_explain` round trip - this view never calls the copilot itself,
   * see `AppShell.tsx`'s `onExplainIncident` doc comment. */
  onExplainIncident: (incidentId: string) => void;
}) {
  const status = useMoneyStatus();
  const ready = status?.state === "ready";
  const session = useSession();
  const { open } = usePopover();
  const openAgent = useCallback(
    (agentId: string, rect: DOMRect) =>
      open(<AgentDetailCard agentId={agentId} onOpenFull={onOpenAgent} onReplay={onOpenReplay} />, { anchor: rect }),
    [open, onOpenAgent, onOpenReplay],
  );

  const [runs, setRuns] = useState<Run[] | null>(null);
  const [incidents, setIncidents] = useState<Incident[] | null>(null);
  const [savings, setSavings] = useState<Savings | null>(null);
  const [error, setError] = useState<MoneyError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [sort, setSort] = useState<{ key: string; dir: SortDir }>({ key: "spend", dir: "desc" });

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

  // Each of these three sets the SAME `error` state `refresh()` above uses,
  // so a rejection (most notably a 403 `role_required` - docs/CONSOLE-IDP.md)
  // renders through the existing banner rather than vanishing as an unhandled
  // rejection. Still rethrown after: `ConfirmButton`'s break-glass ceremony
  // and `BudgetEditor`'s dialog both branch on resolve-vs-reject (the latter
  // deliberately stays open on failure so the operator can retry without
  // re-entering the amount/reason) - swallowing the rejection here would
  // make both ceremonies close as if they had succeeded.
  const handleKill = useCallback(
    async (runId: string, reason: string) => {
      try {
        afterMutation(await killRun(runId, reason));
      } catch (err) {
        setError(err as MoneyError);
        throw err;
      }
    },
    [afterMutation],
  );
  const handleSetBudget = useCallback(
    async (runId: string, budgetUsd: number, reason: string) => {
      try {
        afterMutation(await setBudget(runId, budgetUsd, reason));
      } catch (err) {
        setError(err as MoneyError);
        throw err;
      }
    },
    [afterMutation],
  );
  const handleAck = useCallback(
    async (id: string) => {
      try {
        afterMutation(await ackIncident(id));
      } catch (err) {
        setError(err as MoneyError);
        throw err;
      }
    },
    [afterMutation],
  );

  const topRuns = useMemo(() => {
    const sign = sort.dir === "desc" ? -1 : 1;
    return (runs ?? [])
      .slice()
      .sort((a, b) => cmpRun(a, b, sort.key) * sign)
      .slice(0, RUNS_SHOWN);
  }, [runs, sort]);
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
  // Fleet-wide totals (spend, active runs, calls, saved, open incidents) are
  // deliberately NOT recomputed here: they are the same numbers Overview's
  // own KPI band already shows (docs task 2026-07-24, "slim the duplicated
  // headers") - re-printing them above the Runs table would just repeat
  // Overview's header a second time. `saved` survives below because the
  // retained Governed savings breakdown still needs it as the composition
  // total; nothing else from the old band is kept.
  const saved = savings?.total_saved_usd ?? 0;

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
    onClick: inc.agent_id ? (rect) => openAgent(inc.agent_id as string, rect) : undefined,
    // "Explain with Felyx" (C1, docs/PHASE6-C1.md) sits beside the existing
    // Ack control rather than replacing it - explaining and acknowledging
    // are independent operator actions, so both stay reachable per row.
    action: (
      <span className="flex items-center gap-1.5">
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 9px", fontSize: 11, color: "var(--iris)" }}
          title="Explain this incident with Felyx"
          onClick={() => onExplainIncident(inc.id)}
        >
          Explain
        </button>
        {inc.acknowledged ? (
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
        )}
      </span>
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
          {describeMoneyError(error, session?.role)}
        </div>
      )}

      {runs === null ? (
        <div className="mono" style={{ fontSize: 12, color: "var(--faint)" }}>
          loading money plane...
        </div>
      ) : (
        // No fleet-wide KPI band here (see the doc comment above `saved`):
        // Money leads straight with what is unique to it - the Runs table,
        // the savings breakdown, and Incidents. Overview's own KPI band is
        // one click away for the AI spend / active runs / model calls /
        // open incidents totals.
        <DashMain
          primary={
            <Section
              title="Runs"
              right={`top ${Math.min(RUNS_SHOWN, allRuns.length)} of ${allRuns.length} · full stream in Bus`}
            >
              <div style={{ paddingBottom: 8 }}>
                <SortBar options={RUN_SORTS} active={sort.key} dir={sort.dir} onChange={(key, dir) => setSort({ key, dir })} />
              </div>
              <RunsBoard
                runs={topRuns}
                onKill={handleKill}
                onSetBudget={handleSetBudget}
                onOpenAgentAt={openAgent}
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
      )}
    </div>
  );
}
