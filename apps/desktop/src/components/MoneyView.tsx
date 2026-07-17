import { useCallback, useEffect, useState } from "react";
import { ackIncident, describeMoneyError, fetchIncidents, fetchRuns, fetchSavings, killRun, setBudget } from "../lib/money";
import { useMoneyStatus } from "../lib/useMoneyStatus";
import type { Incident, MoneyError, MutationOutcome, Run, Savings } from "../moneyTypes";
import { IncidentsList } from "./IncidentsList";
import { MoneyEmptyState } from "./MoneyEmptyState";
import { RunsTable } from "./RunsTable";
import { SavingsBreakdown } from "./SavingsBreakdown";
import { UpsellBanner } from "./UpsellBanner";

/** Feels-alive refresh cadence, matching Overview - not a live SSE push (out
 * of scope for this wave), just a periodic re-fetch plus an always-on
 * refetch right after any mutation settles. */
const REFRESH_INTERVAL_MS = 20_000;

function SectionHeader({ title }: { title: string }) {
  return (
    <span className="mono" style={{ fontSize: 11, letterSpacing: "0.1em", textTransform: "uppercase", color: "var(--faint)" }}>
      {title}
    </span>
  );
}

function Loading() {
  return (
    <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
      loading...
    </div>
  );
}

export function MoneyView() {
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

  if (!ready) {
    return <MoneyEmptyState status={status} />;
  }

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-6">
      {notice && (
        <div
          className="panel px-3 py-2 mono text-[11.5px]"
          style={{ background: "var(--panel-2)", color: "var(--sev-low)" }}
        >
          {notice}
        </div>
      )}

      {error && error.kind === "plan_required" && <UpsellBanner error={error} />}
      {error && error.kind !== "plan_required" && (
        <div
          className="panel px-3 py-2 mono text-[11.5px]"
          style={{ background: "var(--panel-2)", color: "var(--sev-high)" }}
        >
          {describeMoneyError(error)}
        </div>
      )}

      <section className="flex flex-col gap-2">
        <SectionHeader title="Runs" />
        {runs === null ? <Loading /> : <RunsTable runs={runs} onKill={handleKill} onSetBudget={handleSetBudget} />}
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Incidents" />
        {incidents === null ? <Loading /> : <IncidentsList incidents={incidents} onAck={handleAck} />}
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Savings" />
        {savings === null ? <Loading /> : <SavingsBreakdown savings={savings} />}
      </section>
    </div>
  );
}
