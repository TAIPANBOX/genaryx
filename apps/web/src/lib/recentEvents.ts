import { hasBackend, invokeBackend } from "./transport";
import { MOCK_EVENTS } from "../mockData";
import type { UiEvent } from "../types";

export type EventsSource = "backend" | "mock";

export interface RecentEventsResult {
  events: UiEvent[];
  /** "backend" when `recent_events` actually answered; "mock" when there is
   * no backend (plain `vite build` / browser preview) or the call failed.
   * Surfaced in the header so it is never ambiguous which one is on screen. */
  source: EventsSource;
}

/**
 * Load recent events for the Bus Explorer.
 *
 * Calls the Rust `recent_events` command when a backend is configured;
 * otherwise (and on any call failure) falls back to the same-shaped mock
 * data in `mockData.ts`, so a plain browser preview always renders.
 */
export async function fetchRecentEvents(limit: number): Promise<RecentEventsResult> {
  if (hasBackend()) {
    try {
      const events = await invokeBackend<UiEvent[]>("recent_events", { limit });
      return { events, source: "backend" };
    } catch (err) {
      // eslint-disable-next-line no-console
      console.error("recent_events invoke failed, falling back to mock data:", err);
    }
  }
  return { events: MOCK_EVENTS.slice(0, limit), source: "mock" };
}
