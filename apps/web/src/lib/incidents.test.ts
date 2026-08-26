/**
 * Tests for the incident aggregator.
 *
 * The one this file exists for is `a_high_event_from_any_plane_becomes_an_incident`.
 * Until 2026-08-26 the caller filtered its bus read to `quality_drift` before
 * this module saw it, so six of the estate's ten planes could not raise an
 * incident in this console at all. Nothing here was wrong; nothing here was
 * asked either, because the narrowing happened one file up.
 *
 * Every test below was run against the pre-change module and the ones that
 * could go red there did, with the failure recorded in the pull request.
 */
import { describe, expect, it } from "vitest";
import {
  aggregateIncidents,
  busCoverage,
  filterIncidents,
  incidentPlane,
  planesPresent,
  busPlaneLabel,
  busPlaneView,
  isIncidentEvent,
  INCIDENT_BANDS,
  TAB_BANDS,
} from "./incidents";
import type { UiEvent } from "../types";

function ev(over: Partial<UiEvent> = {}): UiEvent {
  return {
    id: 1,
    env: "prod",
    ts: "2026-08-26T10:00:00Z",
    source: "tokenfuse",
    type: "dependency_failed",
    agent_id: "agent://acme.example/support/bot",
    run_id: "run-1",
    severity: "high",
    data: {},
    ...over,
  } as UiEvent;
}

function agg(events: readonly UiEvent[]) {
  return aggregateIncidents({
    moneyIncidents: [],
    identityAlerts: [],
    busEvents: events,
    postureFindings: [],
  });
}

describe("what reaches the incident centre", () => {
  it("a_high_event_from_any_plane_becomes_an_incident", () => {
    // One per plane that could raise something and had no way in before.
    const planes: [string, string][] = [
      ["tokenfuse", "dependency_failed"],
      ["verdryx", "slo_burn"],
      ["qryx", "crypto_drift"],
      ["wardryx", "approval_unanswered"],
      ["scopyx", "web_blocked"],
      ["mockryx", "sim_finding"],
    ];
    const rows = agg(planes.map(([source, type], i) => ev({ id: i + 1, source, type })));
    expect(rows).toHaveLength(planes.length);
    for (const [source] of planes) {
      expect(rows.some((r) => r.source === "bus" && (r.raw as UiEvent).source === source)).toBe(true);
    }
  });

  it("the_two_types_that_shipped_on_the_day_this_was_written_are_visible", () => {
    // Named rather than folded into the case above, because these two are the
    // reason it was found: both shipped at `high` on 2026-08-26 and neither
    // could appear in this console on the day it shipped.
    const rows = agg([
      ev({ id: 1, source: "tokenfuse", type: "dependency_failed", data: { dependency: "policy_plane", effect: "allowed_ungoverned" } }),
      ev({ id: 2, source: "verdryx", type: "slo_burn", data: { sli: "containment", trigger: "exhausted" } }),
    ]);
    expect(rows.map((r) => r.title)).toEqual(
      expect.arrayContaining(["dependency failed", "slo burn"]),
    );
    // The member that changes what an operator should do reaches the detail.
    const dep = rows.find((r) => r.title === "dependency failed");
    expect(dep?.detail).toContain("allowed_ungoverned");
  });

  it("a_low_or_info_event_is_not_an_incident", () => {
    // The other half of the rule. Without it the card fills with per-action
    // audit rows and an operator stops reading it, which is the failure the
    // severity bands exist to prevent one plane over.
    expect(agg([ev({ severity: "low", type: "tool_call" })])).toHaveLength(0);
    expect(agg([ev({ severity: "info", type: "policy_allow" })])).toHaveLength(0);
    expect(INCIDENT_BANDS.has("medium")).toBe(false);
  });

  it("an_event_with_no_severity_is_not_guessed_at", () => {
    // The envelope makes severity optional, so a producer may omit it. A
    // consumer that assumed a band would be inventing the one field this
    // whole routing rests on.
    expect(isIncidentEvent(ev({ severity: null }))).toBe(false);
    expect(agg([ev({ severity: null })])).toHaveLength(0);
  });

  it("a_quality_drift_event_keeps_its_own_richer_row_and_is_not_doubled", () => {
    // `fromQualityDrift` reads the verdict, the delta and the baseline; the
    // generic mapper cannot. Both run over the same input, so the risk is two
    // rows for one event rather than a worse one.
    const rows = agg([
      ev({ source: "verdryx", type: "quality_drift", data: { verdict: "regressed", delta: -0.12 } }),
    ]);
    expect(rows).toHaveLength(1);
    expect(rows[0].source).toBe("verdryx");
    expect(rows[0].title).toBe("quality drift: regressed");
  });
});

describe("where a bus row sends you", () => {
  it("the_chip_names_the_producer_and_not_the_bus", () => {
    expect(busPlaneLabel(ev({ source: "qryx" }))).toBe("via qryx");
  });

  it("a_plane_with_a_panel_opens_that_panel", () => {
    expect(busPlaneView(ev({ source: "tokenfuse" }))).toBe("money");
    expect(busPlaneView(ev({ source: "wardryx" }))).toBe("policy");
    expect(busPlaneView(ev({ source: "scopyx" }))).toBe("egress");
  });

  it("a_plane_with_no_panel_opens_the_bus_explorer_rather_than_nothing", () => {
    // A source this console has never heard of is a real event with a real
    // raw line, so the chip must go somewhere. A dead chip would teach an
    // operator that the newest plane's rows are not clickable.
    expect(busPlaneView(ev({ source: "a-plane-shipped-next-year" }))).toBe("bus");
  });
});

describe("saying what the read could not account for", () => {
  it("coverage_names_the_planes_it_heard_from", () => {
    const c = busCoverage(
      [ev({ id: 1, source: "qryx" }), ev({ id: 2, source: "tokenfuse" }), ev({ id: 3, source: "qryx" })],
      500,
    );
    expect(c.planes).toEqual(["qryx", "tokenfuse"]);
    expect(c.read).toBe(3);
    expect(c.incidentRows).toBe(3);
    expect(c.truncated).toBe(false);
  });

  it("a_read_that_filled_its_cap_says_it_is_partial", () => {
    // The one that would otherwise mislead: a cap reached is a window shorter
    // than the one the operator thinks they are looking at. genaryx invariant 8.
    const events = Array.from({ length: 500 }, (_, i) => ev({ id: i + 1 }));
    expect(busCoverage(events, 500).truncated).toBe(true);
    expect(busCoverage(events.slice(0, 499), 500).truncated).toBe(false);
  });

  it("more_rows_than_the_cap_still_counts_as_partial", () => {
    // `>=` and not `===`. A backend answering with more than was asked for is
    // still a read this console cannot claim is complete, and an equality
    // check would have quietly called it whole.
    expect(busCoverage(Array.from({ length: 501 }, (_, i) => ev({ id: i + 1 })), 500).truncated).toBe(true);
  });

  it("an_empty_read_accounts_for_no_plane_at_all", () => {
    const c = busCoverage([], 500);
    expect(c.planes).toEqual([]);
    expect(c.read).toBe(0);
    expect(c.truncated).toBe(false);
  });
});

describe("ordering", () => {
  it("worst_first_across_planes", () => {
    const rows = agg([
      ev({ id: 1, source: "qryx", severity: "high", type: "crypto_drift" }),
      ev({ id: 2, source: "tokenfuse", severity: "critical", type: "budget_exhausted" }),
    ]);
    expect(rows[0].severity).toBe("critical");
  });
});

describe("the same thing happening again", () => {
  it("repeated_bus_events_collapse_into_one_row_with_a_count", () => {
    // Found by looking at the panel, not by a test: with the bus read widened,
    // nine rows of one run's budget refusals pushed every other plane off a
    // ten-row card, and every one of those rows was individually correct.
    const rows = agg([
      ev({ id: 1, type: "breaker_tripped", ts: "2026-08-26T10:00:00Z" }),
      ev({ id: 2, type: "breaker_tripped", ts: "2026-08-26T10:00:05Z" }),
      ev({ id: 3, type: "breaker_tripped", ts: "2026-08-26T10:00:09Z" }),
    ]);
    expect(rows).toHaveLength(1);
    expect(rows[0].occurrences).toBe(3);
    // The newest event's time, so the row does not claim to be older than the
    // last thing that happened in it.
    expect(rows[0].ts).toBe("2026-08-26T10:00:09Z");
  });

  it("the_same_refusal_in_two_runs_is_two_situations", () => {
    // Run is in the key on purpose. Collapsing across runs would tell an
    // operator that one run misbehaved when the whole fleet is failing.
    const rows = agg([
      ev({ id: 1, run_id: "run-a" }),
      ev({ id: 2, run_id: "run-b" }),
    ]);
    expect(rows).toHaveLength(2);
  });

  it("two_different_types_from_one_run_stay_two_rows", () => {
    const rows = agg([
      ev({ id: 1, type: "breaker_tripped" }),
      ev({ id: 2, type: "dependency_failed" }),
    ]);
    expect(rows).toHaveLength(2);
  });

  it("a_run_less_event_groups_per_agent", () => {
    // The right fallback for a fleet-wide signal with no run to belong to.
    const rows = agg([
      ev({ id: 1, run_id: null }),
      ev({ id: 2, run_id: null }),
      ev({ id: 3, run_id: null, agent_id: "agent://acme.example/other" }),
    ]);
    expect(rows).toHaveLength(2);
    expect(rows.find((r) => r.occurrences === 2)).toBeTruthy();
  });
});

describe("the anomalies tab's filters", () => {
  it("a_plane_filter_names_the_producer_not_the_route", () => {
    // `source` says how a row REACHED the console; an operator filters by the
    // plane that raised it. A bus row answers with its producer.
    const rows = agg([ev({ source: "qryx", type: "crypto_drift" })]);
    expect(incidentPlane(rows[0])).toBe("qryx");
    expect(planesPresent(rows)).toEqual(["qryx"]);
  });

  it("no_filter_means_every_row", () => {
    const rows = agg([ev({ id: 1, source: "qryx" }), ev({ id: 2, source: "wardryx" })]);
    expect(filterIncidents(rows, {})).toHaveLength(2);
    expect(filterIncidents(rows, { planes: [], severities: [], query: "" })).toHaveLength(2);
  });

  it("filters_compose_rather_than_replace_each_other", () => {
    const rows = agg([
      ev({ id: 1, source: "qryx", severity: "high", agent_id: "agent://x/one" }),
      ev({ id: 2, source: "qryx", severity: "critical", agent_id: "agent://x/two" }),
      ev({ id: 3, source: "wardryx", severity: "critical", agent_id: "agent://x/one" }),
    ]);
    expect(filterIncidents(rows, { planes: ["qryx"], severities: ["critical"] })).toHaveLength(1);
  });

  it("the_text_filter_matches_part_of_an_agent_id", () => {
    // Substring and not exact: an operator types the part of an `agent://` URI
    // they remember, never the whole thing.
    const rows = agg([
      ev({ id: 1, agent_id: "agent://meridian.io/sre/rca-copilot" }),
      ev({ id: 2, agent_id: "agent://meridian.io/data/pii-scanner" }),
    ]);
    expect(filterIncidents(rows, { query: "rca" })).toHaveLength(1);
    expect(filterIncidents(rows, { query: "RCA" })).toHaveLength(1);
  });

  it("filtering_never_reorders", () => {
    // The card above it is sorted worst-first; a filter that re-sorted would
    // quietly answer a different question than the one beside it.
    const rows = agg([
      ev({ id: 1, severity: "high", type: "a_thing" }),
      ev({ id: 2, severity: "critical", type: "b_thing" }),
      ev({ id: 3, severity: "high", type: "c_thing" }),
    ]);
    const before = rows.map((r) => r.title);
    expect(filterIncidents(rows, { severities: ["high", "critical"] }).map((r) => r.title)).toEqual(before);
  });
});

describe("what the tab may see that the card may not", () => {
  it("a_medium_bus_event_is_not_an_incident_on_the_overview_card", () => {
    // The card's rule, unchanged. Ten rows answering "is anything on fire"
    // must not fill with things that are not on fire.
    expect(agg([ev({ severity: "medium", type: "taint_shadow" })])).toHaveLength(0);
  });

  it("the_tab_can_ask_for_medium_and_gets_it", () => {
    // The gap this closes, and it is about work that shipped the same day:
    // `taint_shadow` is the entire output of a firewall shadow week and it is
    // `medium` on purpose, because paging at `taint_block`'s band during the
    // week an operator was told to watch quietly is how they learn to mute
    // the sender. Fixed at `medium`, it could not reach the console AT ALL,
    // so the one surface Yurii actually looks at would have shown nothing
    // from a subsystem built to be looked at.
    const rows = aggregateIncidents(
      {
        moneyIncidents: [],
        identityAlerts: [],
        busEvents: [
          ev({ id: 1, severity: "medium", source: "tokenfuse", type: "taint_shadow" }),
          ev({ id: 2, severity: "high", source: "tokenfuse", type: "taint_block" }),
        ],
        postureFindings: [],
      },
      { bands: TAB_BANDS },
    );
    expect(rows).toHaveLength(2);
    expect(rows[0].severity).toBe("high");
    expect(rows[1].severity).toBe("medium");
  });

  it("widening_the_bands_does_not_let_in_the_per_action_audit_rows", () => {
    // The tab widens by ONE band, not to everything. `tool_call` and
    // `taint_raised` are `low` by design at the producer: one row per action.
    // A tab that admitted them would be a bus explorer, which this console
    // already has under its own name.
    const rows = aggregateIncidents(
      {
        moneyIncidents: [],
        identityAlerts: [],
        busEvents: [
          ev({ id: 1, severity: "low", type: "tool_call" }),
          ev({ id: 2, severity: "low", type: "taint_raised" }),
          ev({ id: 3, severity: "info", type: "policy_allow" }),
        ],
        postureFindings: [],
      },
      { bands: TAB_BANDS },
    );
    expect(rows).toHaveLength(0);
    expect(TAB_BANDS.has("low")).toBe(false);
  });

  it("the_default_is_still_the_cards_bands_so_no_caller_changes_by_accident", () => {
    // Every existing caller passes no options. If the default widened, the
    // Overview card would silently gain rows nobody asked it for.
    expect([...INCIDENT_BANDS].sort()).toEqual(["critical", "high"]);
    expect([...TAB_BANDS].sort()).toEqual(["critical", "high", "medium"]);
  });
});
