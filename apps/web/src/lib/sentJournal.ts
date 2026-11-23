import { hasBackend, invokeBackend } from "./transport";
import type { ChainVerdict, NotificationsPanel } from "../notificationsTypes";

/**
 * The Notifications panel's read (invariant 51). `null` only when there is no
 * backend to ask; a box that names no journal, or whose journal cannot be
 * read, answers that, and a call that fails says so too, so "nothing sent"
 * and "nobody could look" never render the same.
 */
export type NotificationsRead = { ok: true; panel: NotificationsPanel } | { ok: false; reason: string };

export async function fetchNotifications(): Promise<NotificationsRead | null> {
  if (!hasBackend()) return null;
  try {
    return { ok: true, panel: await invokeBackend<NotificationsPanel>("notifications_sent") };
  } catch (err) {
    const msg =
      err && typeof err === "object" && typeof (err as { error?: unknown }).error === "string"
        ? (err as { error: string }).error
        : typeof err === "string"
          ? err
          : "the box did not answer";
    return { ok: false, reason: msg };
  }
}

/** One sentence for the chain, in the words `heraldyx --journal` uses. */
export function chainSentence(chain: ChainVerdict, windowTruncated: boolean): string {
  const tail = windowTruncated ? " (judged over the newest part of the journal only)" : "";
  switch (chain.state) {
    case "verifies": {
      const unchecked = chain.unverifiable > 0 ? `, ${chain.unverifiable} could not be checked` : "";
      return `chain verifies: ${chain.chained} chained, ${chain.heads} head(s)${unchecked}${tail}`;
    }
    case "broken":
      return `chain BROKEN at line ${chain.first_line}: ${chain.breaks} record(s) name a predecessor that is not the line before them, so the journal was edited after it was written${tail}`;
    case "nothing_bound":
      return "chain: one record or none, which binds nothing, so it is not called verified";
  }
}
