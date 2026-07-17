import { formatUsd } from "../lib/format";
import type { Savings } from "../moneyTypes";
import { StatTile } from "./StatTile";

export function SavingsBreakdown({ savings }: { savings: Savings }) {
  return (
    <div className="grid gap-3" style={{ gridTemplateColumns: "repeat(4, minmax(0, 1fr))" }}>
      <StatTile label="blocked spend" value={formatUsd(savings.blocked_spend_usd)} />
      <StatTile label="cache saved" value={formatUsd(savings.cache_saved_usd)} />
      <StatTile label="router saved" value={formatUsd(savings.router_saved_usd)} />
      <StatTile label="budget breaks" value={String(savings.budget_breaks)} />
    </div>
  );
}
