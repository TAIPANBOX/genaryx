import { hasBackend, invokeBackend } from "./transport";

/**
 * When, why and by whom one agent was stopped.
 *
 * # WHY THE FOLD IS IN RUST AND THIS IS ONLY A READER
 *
 * Two reasons, and the second is the one that matters.
 *
 * The classification is pinned in `crates/api/src/stats/mod.rs`: which event
 * types count as a stop is that build's reading of SPEC 6.2, and which of them
 * name a person is a second rule beside it. A copy of either here would drift,
 * and it would drift silently, because both copies keep returning plausible
 * lists.
 *
 * And a FREEZE is not on the agent's own feed at all. It is journaled as a
 * `console_command` whose `agent_id` is the CONSOLE, naming this agent in
 * `data.members`. A view that filtered `agent_events` would see the refusals a
 * freeze causes and never the freeze itself, so "who stopped this agent" would
 * answer "nobody" for the clearest case there is.
 */

export interface StopEntry {
  /** The producer's own timestamp, unchanged. */
  ts: string;
  /** The wire type, so an operator reads `dlp_block` rather than this build's
   * paraphrase of it. The vocabulary belongs to the products. */
  type_: string;
  source: string;
  /** Who, when the event names somebody. `null` is the ordinary case and means
   * the services stopped it on their own, never "we did not look". */
  actor: string | null;
  /** The producer's own reason where it wrote one. */
  reason: string | null;
  by_operator: boolean;
}

export interface StopsPanel {
  /** False when nothing could be read. Render `note`, never an empty list:
   * an empty list reads as "never stopped". */
  measured: boolean;
  note: string | null;
  total: number;
  by_operator: number;
  entries: StopEntry[];
}

/** Ask the box for one agent's stop history.
 *
 * Returns `null` with no backend and on any failure, so a caller renders no
 * claim rather than a wrong one. The box's own "nothing stored" answer is a
 * real panel with `total: 0`, and the two must not render the same. */
export async function fetchAgentStops(agentId: string): Promise<StopsPanel | null> {
  if (!hasBackend()) return null;
  try {
    const raw = await invokeBackend<StopsPanel>("agent_stops", { agent_id: agentId });
    if (!raw || typeof raw !== "object" || !("measured" in raw)) return null;
    return raw;
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("agent_stops invoke failed:", err);
    return null;
  }
}

/** The one-line description of a stop, in the producer's own words where it
 * wrote any.
 *
 * Never invents a reason. An event with no reason field says so, because
 * "blocked, no reason recorded" is a fact an operator can act on (somebody's
 * producer is not writing one) and a plausible-sounding invention is not. */
export function stopSummary(e: StopEntry): string {
  const who = e.actor ? `by ${e.actor}` : "by the services";
  const why = e.reason ? `: ${e.reason}` : ", no reason recorded";
  return `${who}${why}`;
}
