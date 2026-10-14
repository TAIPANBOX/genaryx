import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { GatewaysBody, GatewaysSection, gatewaysLoader } from "./GatewaysSection";
import { ageLabel, gatewaysCaption } from "../lib/gateways";
import type { Gateways, GatewaySite, MoneyError } from "../moneyTypes";

/**
 * The Overview's Gateways section, rendered for real (`react-dom/server`).
 * The forge lab of 2026-10-14 ran two gateways, the Anthropic door `forge`
 * and the OpenAI door `forge-openai`; here the second has stopped pushing.
 */
const site = (over: Partial<GatewaySite>): GatewaySite => ({
  site: "forge",
  state: "reporting",
  last_push: "2026-10-14T13:49:40.000Z",
  last_push_age_secs: 20,
  last_record: "2026-10-14T13:49:35.000Z",
  first_push: "2026-10-13T13:50:00.000Z",
  pushes: 930,
  calls: 12,
  spent_usd: 0.005168,
  ...over,
});

const FORGE: Gateways = {
  reported: true,
  silent_after_secs: 300,
  checked_at: "2026-10-14T13:50:00.000Z",
  gateways: [
    site({ site: "forge-openai", state: "silent", last_push_age_secs: 2820, calls: 4, spent_usd: 0.002584 }),
    site({}),
  ],
};

const render = (data: Gateways | null, error: MoneyError | null = null) =>
  renderToStaticMarkup(createElement(GatewaysBody, { data, error }));

describe("the Overview's Gateways section", () => {
  it("a_gateway_that_stopped_reporting_reads_silent_on_the_overview", () => {
    const html = render(FORGE);
    expect(html).toContain("forge-openai");
    expect(html).toContain("silent 47 min");
    expect(html).toContain("silent for 47 min");
    expect(html).toContain("stopped pushing calls and heartbeats");
    expect(html.indexOf("forge-openai")).toBeLessThan(html.indexOf(">forge<"));
    expect(gatewaysCaption(FORGE)).toBe("2 site(s) · 1 silent");
  });

  it("a reporting gateway says when it was last heard", () => {
    const html = render({ ...FORGE, gateways: [site({})] });
    expect(html).toContain(">reporting<");
    expect(html).toContain("heard 20 s ago");
    expect(html).not.toContain("silent for");
    expect(html).not.toContain("d-pill over");
  });

  it("a_cloud_without_the_gateways_route_is_not_shown_as_none_silent", () => {
    const data: Gateways = { ...FORGE, reported: false, gateways: [] };
    expect(render(data)).toContain("cannot be shown here");
    expect(render(data)).toContain("not the same as none having stopped");
    expect(gatewaysCaption(data)).toBe("not reported by this Cloud");
  });

  it("says every empty and failed state in words", () => {
    expect(render({ ...FORGE, gateways: [] })).toContain("no gateway has pushed to this Cloud yet");
    expect(render(null)).toContain("reading gateways");
    expect(render(null, { kind: "cloud", status: 500, message: "boom" })).toContain(
      "gateways could not be read: Cloud error 500: boom",
    );
    expect(render(FORGE)).toContain("TOKENFUSE_CLOUD_HEARTBEAT_SECONDS=0");
    expect(render({ ...FORGE, gateways: [site({ last_record: null })] })).toContain("no call reported yet");
  });

  it("rounds an age down to its largest unit", () => {
    expect(ageLabel(-3)).toBe("just now");
    expect(ageLabel(0)).toBe("just now");
    expect(ageLabel(59)).toBe("59 s");
    expect(ageLabel(2820)).toBe("47 min");
    expect(ageLabel(3 * 3600 + 5)).toBe("3 h");
    expect(ageLabel(2 * 86_400)).toBe("2 d");
  });

  it("loads into data or error, never both stale", async () => {
    const seen: { data: Gateways[]; errors: (MoneyError | null)[] } = { data: [], errors: [] };
    const ok = gatewaysLoader({ fetch: async () => FORGE, onData: (d) => seen.data.push(d), onError: (e) => seen.errors.push(e) });
    await ok();
    expect(seen.data).toEqual([FORGE]);
    expect(seen.errors).toEqual([null]);
    const boom: MoneyError = { kind: "cloud", status: 500, message: "boom" };
    const bad = gatewaysLoader({
      fetch: async () => {
        throw boom;
      },
      onData: (d) => seen.data.push(d),
      onError: (e) => seen.errors.push(e),
    });
    await bad();
    expect(seen.errors).toEqual([null, boom]);
    expect(renderToStaticMarkup(createElement(GatewaysSection))).toContain("Gateways");
  });
});

describe("the gateways card's inset", () => {
  it("the_gateways_body_is_inset_like_the_other_dashboard_cards", () => {
    // The card itself has no padding; each body insets itself, as `.d-comp`
    // does with 6px 20px 18px. A body without it puts its rows, pills and
    // note against the card's edges.
    for (const html of [render(FORGE), render(null)]) {
      expect(html).toMatch(/^<div[^>]*style="[^"]*padding:\s*(6px|10px) 20px/);
    }
  });
});
