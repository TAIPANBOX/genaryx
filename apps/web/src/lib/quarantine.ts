import { hasBackend, invokeBackend } from "./transport";

/**
 * What the bus REFUSED, and why.
 *
 * # WHY THIS IS A PANEL AND NOT A LOG LINE
 *
 * Lines that fail the envelope have always been kept by the store, with their
 * file, offset, raw bytes and the validator's own reason. Nothing read them
 * back until 2026-08-11, so the only report was one line on stderr at startup.
 * A producer that broke its envelope after boot was invisible for as long as
 * the console stayed up.
 *
 * That matters because the console does not go blank when it happens. It keeps
 * showing the rest of the bus, correctly, and the broken producer's agents just
 * look idle. The real instance is `aws-comparable-176`: twelve events, every
 * one refused for an `agent_id` with no `agent://` prefix, and the console's
 * honest answer for that agent was nothing at all.
 */
export interface QuarantineReason {
  /** The validator's own words, not a paraphrase: an operator fixing a
   * producer needs the message the check produced. */
  reason: string;
  count: number;
  last_ts: string | null;
  /** Which file and byte offset one of these came from, so the producer is
   * findable on disk rather than guessable. */
  example_file: string | null;
  example_offset: number | null;
  /** The head of one refused line, capped by the backend. */
  raw_excerpt: string | null;
}

export interface QuarantinePanel {
  /** False when the store could not be read. Render `note`, never "nothing was
   * refused": that is the same wrong answer that reads as good news. */
  measured: boolean;
  note: string | null;
  total: number;
  reasons: QuarantineReason[];
}

/**
 * Ask the box what it refused.
 *
 * Returns `null` with no backend and on any failure, so a caller renders no
 * claim at all rather than a wrong one. This is the same rule `busStatus.ts`
 * follows, and it matters more here: a silent `{total: 0}` on a failed call
 * would say every line was accepted.
 */
export async function fetchQuarantine(): Promise<QuarantinePanel | null> {
  if (!hasBackend()) return null;
  try {
    const raw = await invokeBackend<QuarantinePanel>("bus_quarantine");
    if (!raw || typeof raw !== "object" || !("measured" in raw)) return null;
    return raw;
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("bus_quarantine invoke failed:", err);
    return null;
  }
}
