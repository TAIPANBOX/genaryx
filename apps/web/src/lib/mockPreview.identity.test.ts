import { describe, expect, it } from "vitest";

import { aggregateIncidents, incidentSubject, TAB_BANDS } from "./incidents";
import { mockInvoke } from "./mockPreview";
import type { UiEvent } from "../types";

/**
 * The published demo runs on this mock transport, so what it shows of an
 * identity refusal is what a visitor learns about invariant 19: the refusal
 * filed under the key that made the call, and the incident naming both the
 * claimed agent and the key, never the claimed agent as its subject.
 */
describe("the demo bus carries an identity refusal filed under its key", () => {
  it("demo_bus_shows_an_identity_refusal_under_the_key_not_the_claimed_agent", async () => {
    // The Incidents tab asks for 500 and keeps the first 500; a refusal placed
    // after the seed was cut there, so the read here does the same.
    const events = (await mockInvoke<UiEvent[]>("recent_events", { limit: 500 })).slice(0, 500);
    const refusals = events.filter((e) => e.type === "identity_mismatch");
    expect(refusals.length).toBeGreaterThan(0);
    const claimed = refusals[0].agent_id;

    const rows = aggregateIncidents(
      { moneyIncidents: [], identityAlerts: [], busEvents: events, postureFindings: [] },
      { bands: TAB_BANDS },
    );
    const row = rows.find((r) => r.title === "identity mismatch");
    expect(row).toBeDefined();
    expect(incidentSubject(row!)).toBe("key:forge-imposter");
    expect(row!.detail).toContain(`claimed ${claimed} with key forge-imposter`);
    expect(row!.occurrences).toBe(refusals.length);
  });

  it("demo_statistics_count_the_refusal_under_the_key", async () => {
    const events = await mockInvoke<UiEvent[]>("recent_events", { limit: 60 });
    const claimed = events.find((e) => e.type === "identity_mismatch")?.agent_id;
    const stats = await mockInvoke<{ agents: { agent_id: string; by_type: Record<string, number> }[] }>(
      "stats_counts",
      { window_days: 30 },
    );
    const key = stats.agents.find((a) => a.agent_id === "key:forge-imposter");
    expect(key?.by_type.identity_mismatch).toBeGreaterThan(0);
    const victim = stats.agents.find((a) => a.agent_id === claimed);
    expect(victim?.by_type.identity_mismatch ?? 0).toBe(0);
  });
});

describe("the claimed agent's own feed in the demo", () => {
  it("demo_claimed_agents_feed_carries_the_refusal_as_a_claim_by_the_key", async () => {
    const { agentFeedLine } = await import("./attribution");
    const events = await mockInvoke<UiEvent[]>("recent_events", { limit: 60 });
    const claimed = events.find((e) => e.type === "identity_mismatch")!.agent_id;
    // Agent 360 reads `agent_events` by envelope, as the real backend does,
    // so the refusals are on the claimed agent's own feed, and must read as a
    // claim by the key there.
    const feed = await mockInvoke<UiEvent[]>("agent_events", { agent_id: claimed, limit: 50 });
    const refusals = feed.filter((e) => e.type === "identity_mismatch");
    expect(refusals.length).toBeGreaterThan(0);
    for (const e of refusals) {
      expect(agentFeedLine(e, claimed)).toEqual({
        text: "identity_mismatch: claimed by key forge-imposter, refused",
        own: false,
      });
    }
  });
});

describe("the demo shows an incident with no run id", () => {
  it("demo_bus_keeps_a_runless_incident_inside_the_incidents_tab_read", async () => {
    // The Incidents tab reads 500 and keeps the first 500. The demo's one
    // run-less bus incident (quality drift) used to sit after the seed and was
    // cut there, so the demo could not show Incident 360's "no run id" state.
    const events = (await mockInvoke<UiEvent[]>("recent_events", { limit: 500 })).slice(0, 500);
    const rows = aggregateIncidents(
      { moneyIncidents: [], identityAlerts: [], busEvents: events, postureFindings: [] },
      { bands: TAB_BANDS },
    );
    const runless = rows.filter((r) => (r.source === "bus" || r.source === "verdryx") && !r.raw.run_id);
    expect(runless.length).toBeGreaterThan(0);
  });
});
