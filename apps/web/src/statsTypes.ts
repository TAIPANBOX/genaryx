/** Per-agent event counts, as the bus recorded them.
 *
 * Field-for-field mirror of `crates/api/src/stats/mod.rs`. Two shapes here are
 * load-bearing and easy to flatten by accident:
 *
 * - `measured: false` means the console could not look. The rows are empty and
 *   mean nothing, and a component must render `note` INSTEAD of the table. An
 *   empty table says "none of your agents was ever stopped", which is the one
 *   wrong answer that reads as good news.
 * - `worst_overshoot_microusd: null` means no event in this window carried the
 *   amounts, which is not `0`. Zero is "went over by nothing"; null is "nobody
 *   wrote down by how much". The Cloud's own `budget_exhausted` export carries
 *   no amounts at all, so null is the common case, not the exotic one.
 */

/** One agent's counts over the bus window. */
export interface AgentStats {
  agent_id: string;
  /** Every stop, whoever caused it. */
  blocked: number;
  /** The subset of `blocked` a human caused: a kill naming an actor, or an
   * approval a person denied. An operator FREEZE is invisible here, because it
   * is enforced as an ordinary policy and its refusals carry no mark; those
   * count on the system side rather than being guessed. See the Rust doc. */
  blocked_by_operator: number;
  anomalies: number;
  budget_events: number;
  /** The WORST single breach, in micro-USD, not the sum of them: one runaway
   * run trips its breaker on every call, and adding those up reports an
   * overspend that never happened. How often it happened is `budget_events`.
   * `null` when no event carried the amounts - see this module's doc. */
  worst_overshoot_microusd: number | null;
  /** Every event type seen for this agent under its own raw name, including
   * types this build does not recognize. */
  by_type: Record<string, number>;
  last_seen: string;
}

export interface StatsPanel {
  /** False when nothing could be read. Render `note`, not the table. */
  measured: boolean;
  note: string | null;
  /** How many bus lines were actually read, so a reader can tell a quiet
   * estate from a short window. */
  scanned: number;
  agents: AgentStats[];
}

/** Mirrors `lib/egress.ts`'s `EgressError`: the two ways asking can fail. */
export type StatsError =
  | { kind: "no_environment" }
  | { kind: "backend"; message: string };
