/** Shared derivations for the dashboard panels: money grouping, severity
 * ranking/colours, and the client-side spend-over-window series (there is no
 * burn-rate Cloud endpoint yet - see lib/money.ts). Kept framework-free so
 * every panel computes the same way. */
import type { Run } from "../moneyTypes";

/** Big money: thousands-separated, no cents (hero headline). */
export const usd0 = (v: number): string => "$" + Math.round(v).toLocaleString("en-US");

const SEV_ORDER = ["info", "low", "medium", "high", "critical"] as const;

/** Rank a lowercase severity so worst sorts first. Unknown -> 0. */
export function sevRank(s: string): number {
  const i = SEV_ORDER.indexOf(s as (typeof SEV_ORDER)[number]);
  return i < 0 ? 0 : i;
}

/** Map a severity to its theme colour variable. */
export function sevColor(s: string): string {
  return `var(--sev-${SEV_ORDER.includes(s as (typeof SEV_ORDER)[number]) ? s : "info"})`;
}

/** Last path segment of an `agent://org/team/name` id. */
export function agentShortName(agentId: string): string {
  const parts = agentId.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? agentId;
}

/** Second-to-last segment (the team) of an agent id. */
export function agentTeam(agentId: string): string {
  const parts = agentId.split("/").filter(Boolean);
  return parts[parts.length - 2] ?? "";
}

export interface AgentSpend {
  agent: string;
  name: string;
  team: string;
  spent: number;
  calls: number;
  runs: number;
}

/** Group runs into per-agent spend totals, high-to-low. */
export function spendByAgent(runs: Run[]): AgentSpend[] {
  const m = new Map<string, AgentSpend>();
  for (const r of runs) {
    const e =
      m.get(r.agent_id) ??
      { agent: r.agent_id, name: agentShortName(r.agent_id), team: agentTeam(r.agent_id), spent: 0, calls: 0, runs: 0 };
    e.spent += r.spent_usd;
    e.calls += r.calls;
    e.runs += 1;
    m.set(r.agent_id, e);
  }
  return [...m.values()].sort((a, b) => b.spent - a.spent);
}

/** Bucket per-run spend by `last_seen` into a spend-over-window curve for the
 * hero sparkline. */
export function spendSeries(runs: Run[], buckets = 32): number[] {
  const times = runs.map((r) => new Date(r.last_seen).getTime()).filter((t) => !Number.isNaN(t));
  if (times.length < 2) return [];
  const min = Math.min(...times);
  const max = Math.max(...times);
  const span = Math.max(1, max - min);
  const out = new Array<number>(buckets).fill(0);
  for (const r of runs) {
    const t = new Date(r.last_seen).getTime();
    if (Number.isNaN(t)) continue;
    const idx = Math.min(buckets - 1, Math.floor(((t - min) / span) * buckets));
    out[idx] += r.spent_usd;
  }
  return out;
}
