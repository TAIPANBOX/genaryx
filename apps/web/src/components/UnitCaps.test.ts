import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { UnitCapsBody } from "./UnitCaps";
import { capLabel, capUsd, monthSpentLabel, unitCapsCaption } from "../lib/unitCaps";
import type { MoneyError, UnitCap, UnitCaps } from "../moneyTypes";

/**
 * The Money tab's unit month caps, rendered for real (`react-dom/server`) from
 * the forge lab's figures of 2026-10-14: unit `capped` over a USD 0.00005
 * cap, `w2x` over its central cap, `lab` under, `console` with none.
 */
const row = (over: Partial<UnitCap>): UnitCap => ({
  unit: "lab",
  month_spent_usd: 0.0024,
  month_calls: 3,
  cap_usd: 0.006,
  fraction: 0.4,
  state: "under_cap",
  cappable: true,
  ...over,
});

const FORGE: UnitCaps = {
  month: "2026-10",
  units_read: true,
  caps_read: true,
  notes: [],
  units: [
    row({ unit: "capped", month_spent_usd: 0.000061, month_calls: 2, cap_usd: 0.00005, fraction: 1.22, state: "over_cap" }),
    row({ unit: "w2x", month_spent_usd: 0.013376, month_calls: 6, cap_usd: 0.007168, fraction: 1.866, state: "over_cap" }),
    row({ unit: "lab" }),
    row({ unit: "console", month_spent_usd: 0.002, cap_usd: null, fraction: null, state: "no_cap" }),
    row({ unit: "unassigned", month_spent_usd: 0.0009, cap_usd: null, fraction: null, state: "no_cap", cappable: false }),
  ],
};

const render = (data: UnitCaps | null, error: MoneyError | null = null) =>
  renderToStaticMarkup(createElement(UnitCapsBody, { data, error, onSetCap: async () => {} }));

/** The row for one unit, so an assertion cannot match another row's text. */
function rowOf(html: string, unit: string): string {
  const start = html.indexOf(`title="${unit}"`);
  const next = html.indexOf('<div class="d-tr"', start);
  return html.slice(start, next === -1 ? undefined : next);
}

describe("the Money tab's unit month caps", () => {
  it("the_unit_caps_section_shows_each_units_cap_spend_and_state", () => {
    const html = render(FORGE);
    const capped = rowOf(html, "capped");
    expect(capped).toContain("$0.000061");
    expect(capped).toContain("$0.000050");
    expect(capped).toContain("over cap");
    expect(rowOf(html, "lab")).toContain("under cap");
    expect(rowOf(html, "lab")).toContain("$0.0060");
    expect(rowOf(html, "console")).toContain("no central cap");
    expect(rowOf(html, "console")).toContain(">none<");
    expect(unitCapsCaption(FORGE)).toBe("5 unit(s) · 2 over cap · month 2026-10 (UTC)");
  });

  it("offers a cap control on a unit and says why the unassigned bucket has none", () => {
    const html = render(FORGE);
    expect(rowOf(html, "lab")).toContain(">Cap<");
    const unassigned = rowOf(html, "unassigned");
    expect(unassigned).not.toContain(">Cap<");
    expect(unassigned).toContain("unmapped spend");
  });

  it("caps_the_console_could_not_read_read_cap_unknown_never_none", () => {
    const data: UnitCaps = {
      ...FORGE,
      caps_read: false,
      notes: ["TokenFuse Cloud did not answer GET /v1/unit-budgets: cloud returned HTTP 500"],
      units: [row({ unit: "lab", cap_usd: null, fraction: null, state: "cap_unknown" })],
    };
    const html = render(data);
    expect(rowOf(html, "lab")).toContain("cap unknown");
    expect(rowOf(html, "lab")).toContain(">unknown<");
    expect(rowOf(html, "lab")).not.toContain(">none<");
    expect(html).toContain("did not answer GET /v1/unit-budgets");
  });

  it("says a spend the Cloud did not report is not reported, never zero", () => {
    const html = render({ ...FORGE, units: [row({ month_spent_usd: null, month_calls: null, fraction: null, state: "spend_unknown" })] });
    expect(html).toContain("not reported");
    expect(html).toContain("spend unknown");
    expect(html).not.toContain("$0.00<");
  });

  it("a_zero_cap_says_it_refuses_every_call", () => {
    const zero = row({ cap_usd: 0, fraction: null, state: "over_cap" });
    expect(capLabel(zero, FORGE)).toBe("$0.00 · refuses every call");
    expect(render({ ...FORGE, units: [zero] })).toContain("refuses every call");
  });

  it("names what the section cannot see, and every empty or failed state", () => {
    expect(render(FORGE)).toContain("identity map file");
    expect(render({ ...FORGE, units: [] })).toContain("no unit has spent this month and no central cap is set");
    expect(render(null)).toContain("reading unit caps");
    expect(render(null, { kind: "cloud", status: 500, message: "boom" })).toContain("unit caps could not be read: Cloud error 500: boom");
    expect(unitCapsCaption({ ...FORGE, month: null })).toContain("month not reported");
  });

  it("keeps a micro-dollar cap readable", () => {
    expect(capUsd(0.00005)).toBe("$0.000050");
    expect(capUsd(0.25)).toBe("$0.2500");
    expect(capUsd(12)).toBe("$12.00");
    expect(capUsd(0)).toBe("$0.00");
    expect(monthSpentLabel(row({ month_spent_usd: 0 }))).toBe("$0.00");
  });
});
