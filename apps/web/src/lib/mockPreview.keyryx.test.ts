import { describe, expect, it } from "vitest";

import { aggregateIncidents, busPlaneView, TAB_BANDS } from "./incidents";
import { mockInvoke } from "./mockPreview";
import type { UiEvent } from "../types";

/**
 * The published demo runs on this mock transport, so what it shows of the
 * Keyryx witness is what a visitor learns about it: a finding about a stream
 * on the bus becomes an incident that names the stream, with no code of its
 * own per type. The rows are synthetic, as every demo row is.
 */
describe("the demo bus carries the Keyryx witness's findings", () => {
  async function demoBus(): Promise<UiEvent[]> {
    // The Incidents tab asks for 500 and keeps the first 500, so a finding
    // placed after the seed would be cut there; the read here does the same.
    return (await mockInvoke<UiEvent[]>("recent_events", { limit: 500 })).slice(0, 500);
  }

  it("demo_bus_shows_a_high_keyryx_finding_as_an_incident_naming_its_stream", async () => {
    const events = await demoBus();
    const findings = events.filter((e) => e.source === "keyryx");
    expect(findings.length).toBeGreaterThan(0);
    for (const e of findings) {
      expect(e.agent_id).toBe("agent://keyryx.internal/witness");
      expect(e.schema).toBe("taipanbox.dev/agent-event/v1.0");
    }
    const rows = aggregateIncidents({ moneyIncidents: [], identityAlerts: [], busEvents: events, postureFindings: [] });
    const row = rows.find((r) => r.title === "witness stream rewritten");
    expect(row).toBeDefined();
    expect(row!.severity).toBe("high");
    expect(row!.detail).toContain("stream wardryx.ndjson");
    expect(busPlaneView(row!.raw as UiEvent)).toBe("bus");
  });

  it("demo_medium_keyryx_finding_is_on_the_incidents_tab_and_not_the_fire_card", async () => {
    const events = await demoBus();
    const card = aggregateIncidents({ moneyIncidents: [], identityAlerts: [], busEvents: events, postureFindings: [] });
    const tab = aggregateIncidents(
      { moneyIncidents: [], identityAlerts: [], busEvents: events, postureFindings: [] },
      { bands: TAB_BANDS },
    );
    expect(card.some((r) => r.title === "witness checkpoint unsigned")).toBe(false);
    expect(tab.some((r) => r.title === "witness checkpoint unsigned")).toBe(true);
  });
});
