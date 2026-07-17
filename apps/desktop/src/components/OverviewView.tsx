import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { describeMoneyError, fetchOverview } from "../lib/money";
import { useMoneyStatus } from "../lib/useMoneyStatus";
import { formatUsd } from "../lib/format";
import type { MoneyError, Overview } from "../moneyTypes";
import { MoneyEmptyState } from "./MoneyEmptyState";
import { StatTile } from "./StatTile";
import { UpsellBanner } from "./UpsellBanner";

const REFRESH_INTERVAL_MS = 20_000;

export function OverviewView() {
  const status = useMoneyStatus();
  const ready = status?.state === "ready";

  const [overview, setOverview] = useState<Overview | null>(null);
  const [error, setError] = useState<MoneyError | null>(null);

  const refresh = useCallback(async () => {
    if (!ready) return;
    try {
      setOverview(await fetchOverview());
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

  if (!ready) {
    return <MoneyEmptyState status={status} />;
  }

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-4">
      <span className="chip" style={cssVar("dot", "var(--sev-low)")}>
        <span className="dot" aria-hidden="true" />
        {status.source.source === "taipan" ? `taipan up · ${status.source.name}` : "env fallback"} &middot;{" "}
        {status.cloud_url}
      </span>

      {error && error.kind === "plan_required" && <UpsellBanner error={error} />}
      {error && error.kind !== "plan_required" && (
        <div
          className="panel px-3 py-2 mono text-[11.5px]"
          style={{ background: "var(--panel-2)", color: "var(--sev-high)" }}
        >
          {describeMoneyError(error)}
        </div>
      )}

      {overview === null ? (
        <div className="mono text-[12px]" style={{ color: "var(--faint)" }}>
          loading overview...
        </div>
      ) : (
        <div className="grid gap-3" style={{ gridTemplateColumns: "repeat(4, minmax(0, 1fr))" }}>
          <StatTile
            label="total spent"
            value={formatUsd(overview.total_spent_usd)}
            sub={`${overview.total_calls} calls`}
          />
          <StatTile
            label="active runs"
            value={String(overview.active_runs)}
            sub={`${overview.killed_runs} killed of ${overview.total_runs}`}
          />
          <StatTile
            label="open incidents"
            value={String(overview.open_incidents)}
            sub={`${overview.total_incidents} total`}
            tone={overview.open_incidents > 0 ? "var(--sev-high)" : undefined}
          />
          <StatTile label="total saved" value={formatUsd(overview.total_saved_usd)} tone="var(--sev-low)" />
        </div>
      )}
    </div>
  );
}
