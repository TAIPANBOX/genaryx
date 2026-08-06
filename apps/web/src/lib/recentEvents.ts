import { hasBackend, invokeBackend } from "./transport";
import { MOCK_EVENTS } from "../mockData";
import type { UiEvent } from "../types";

export type EventsSource = "backend" | "mock" | "error";

export interface RecentEventsResult {
  events: UiEvent[];
  /** Where these rows came from.
   *
   * - `backend`: `recent_events` answered. The rows are the operator's own,
   *   and an empty list means an empty bus, which is information.
   * - `mock`: there is NO backend to ask (a plain `vite build` / browser
   *   preview). Fixtures are honest here because nothing real is being
   *   misrepresented, and it is labelled as mock everywhere it surfaces.
   * - `error`: there IS a backend and it did not answer. No rows, ever.
   */
  source: EventsSource;
  /** Why the read failed, when `source` is `error`, so a panel can say what
   * happened instead of rendering an unexplained empty list. */
  error?: string;
}

/**
 * Load recent events for the Bus Explorer.
 *
 * A backend failure returns NO events and says so. It used to fall through to
 * `mockData.ts` on any thrown error, which meant a console pointed at a real
 * box that had stopped answering filled its panels with a fixture stream:
 * plausible agents, plausible severities, plausible timestamps, none of them
 * things that happened. CLAUDE.md invariant 4 names that as the single worst
 * thing this product can do, because the entire proposition is that what you
 * see is what happened, and a status-bar label is not enough when the ROWS
 * are the claim.
 *
 * The fixtures stay for the one case where they mislead nobody: no backend is
 * configured at all, so there is no real answer being displaced. That is the
 * same line `lib/graph.ts` already draws, except graph has no fixtures to
 * fall back to and returns empty on both paths.
 */
export async function fetchRecentEvents(limit: number): Promise<RecentEventsResult> {
  if (!hasBackend()) {
    return { events: MOCK_EVENTS.slice(0, limit), source: "mock" };
  }
  try {
    const events = await invokeBackend<UiEvent[]>("recent_events", { limit });
    return { events, source: "backend" };
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("recent_events invoke failed, reporting no events:", err);
    return {
      events: [],
      source: "error",
      error: err instanceof Error ? err.message : String(err),
    };
  }
}
