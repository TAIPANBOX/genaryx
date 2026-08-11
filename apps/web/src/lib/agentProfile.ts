import { hasBackend, invokeBackend } from "./transport";

/**
 * What ONE agent's normal looks like, and how its latest complete day sits in
 * it. The shape of `crates/api/src/stats/profile.rs`.
 *
 * # WHY THIS IS NOT ANOTHER COUNT
 *
 * The Statistics tab answers "how many". That ranks a fleet and does not
 * describe an agent: twenty-six stops in an hour and twenty-six across a month
 * are the same number and different situations, and the number cannot tell them
 * apart. This compares an agent to ITSELF over time, which is the only
 * comparison that means anything here. Nothing in it compares one agent to
 * another, because a busy agent is not an abnormal agent.
 */

/** Whether the agent has been watched long enough for "unusual" to mean
 * anything. `too_new` still carries real counts and refuses only the
 * comparison, which is the useful half of a refusal. */
export type Confidence = "no_data" | "too_new" | "normal";

/** The last 7 days against the 7 before them. */
export type Direction = "rising" | "falling" | "steady" | "unknown";

export interface AgentProfile {
  agent_id: string;
  confidence: Confidence;
  /** Days from the agent's first stored event to now, capped at the window.
   * Reported even when the comparison is refused. */
  days_held: number;
  total: number;
  /** The agent's median day over EVERY day held, including the empty ones. A
   * median over only its busy days is the number that makes a quiet agent's
   * first bad day look ordinary. */
  median_day: number;
  /** The most recent COMPLETE UTC day. Today is excluded on purpose: a partial
   * day against full days reads as a quiet agent every morning. */
  latest_full_day: number;
  /** `latest_full_day` as a multiple of `median_day`, or null when there is no
   * usable median (too new, or a median of zero where every non-zero day would
   * be "infinitely above normal"). */
  times_median: number | null;
  /** Share of the window's events on its single busiest day: "a bad afternoon"
   * against "a bad month". Raw, with no threshold: 0.49 and 0.51 are not
   * different situations, and a cutoff here would be a verdict the counter
   * invented. */
  busiest_day_share: number;
  direction: Direction;
  /** The type that fired most, and its share. "The same thing over and over"
   * and "a different thing every time" are the same count. */
  top_type: string | null;
  top_type_share: number;
  /** Daily totals oldest-first, zero-filled, so every number above can be
   * checked against the days it came from. */
  daily: number[];
}

/** The quarter the card is designed around, matching the backend default and
 * the retention default, so the card never asks for history the box dropped. */
export const PROFILE_WINDOW_DAYS = 90;

/**
 * Ask the box for one agent's profile.
 *
 * Returns `null` with no backend and on any failure, so a caller renders no
 * claim rather than a wrong one. A profile is a statement about an agent's
 * behaviour, and the empty shape (`no_data`, zeroes) is a real answer the box
 * gives on purpose; a failed call must not be dressed up as one.
 */
export async function fetchAgentProfile(
  agentId: string,
  windowDays = PROFILE_WINDOW_DAYS,
): Promise<AgentProfile | null> {
  if (!hasBackend()) return null;
  try {
    const raw = await invokeBackend<AgentProfile | null>("agent_profile", {
      agent_id: agentId,
      window_days: windowDays,
    });
    if (!raw || typeof raw !== "object" || !("confidence" in raw)) return null;
    return raw;
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("agent_profile invoke failed:", err);
    return null;
  }
}

/** The one sentence the card leads with.
 *
 * Written here rather than in the component so it can be read on its own and
 * tested. Every branch names what it is comparing, because "3.2x" with no
 * denominator is a number a reader cannot check. */
export function profileSentence(p: AgentProfile): string {
  if (p.confidence === "no_data") {
    return "No events stored for this agent, so there is nothing to compare a day against.";
  }
  if (p.confidence === "too_new") {
    return `Watched for ${p.days_held} day(s), which is too short to call anything unusual. ${p.total} event(s) so far.`;
  }
  const base = `Its median day over ${p.days_held} days is ${fmt(p.median_day)} event(s); yesterday was ${p.latest_full_day}`;
  if (p.times_median === null) {
    // A zero median: every non-zero day is "infinitely more", which is true and
    // says less than the count already did.
    return `${base}. Most days are empty, so a multiple would say nothing a count does not.`;
  }
  return `${base}, which is ${fmt(p.times_median)}x its median day.`;
}

/** Trim a float to at most two decimals without printing "3.00". */
function fmt(n: number): string {
  return Number.isInteger(n) ? String(n) : String(Math.round(n * 100) / 100);
}
