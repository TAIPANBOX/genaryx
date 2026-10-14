import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { GatewayPostureBody, GatewayPostureSection, NOT_REPORTED, postureLoader } from "./GatewayPosture";
import type { CredentialsError, GatewayPosture } from "../lib/credentials";

/**
 * The Posture view's "Gateway settings" section, rendered for real. The
 * figures are the forge lab gateway's of 2026-10-14 (stack-single v1.1.32).
 */
const FORGE: GatewayPosture = {
  gateway_url: "http://192.168.1.10:4100",
  settings: {
    reported: true,
    items: [
      { key: "firewall", label: "Firewall (taint)", value: "enforce" },
      { key: "dlp_pii", label: "PII detection", value: "shadow" },
      { key: "max_run_budget_usd", label: "Run budget ceiling (USD)", value: "5.0" },
      { key: "wire", label: "Wire (provider shape)", value: "anthropic" },
      { key: "router", label: "FinOps router", value: null },
    ],
  },
  keys: { identity_strict: "enforce", identity_map_configured: true, configured_keys: 5, keys_listed: 6, unauthorized_attempts: 4 },
  price_book: {
    rows: 61,
    built_in_rows: 60,
    operator_file: true,
    overridden_models: [],
    added_models: ["openai/gpt-4.1-mini"],
    fallback_input_per_mtok_usd: 15,
    fallback_output_per_mtok_usd: 75,
    tiers: [{ model: "gpt-6-astra", above_prompt_tokens: 200000, input_per_mtok_usd: 4, output_per_mtok_usd: 18 }],
    tiers_dropped_models: [],
    fallback_models_seen: ["openai/gpt-4.1-nano"],
    fallback_models_seen_total: 1,
  },
  notes: [],
};

const render = (data: GatewayPosture | null, error: CredentialsError | null = null) =>
  renderToStaticMarkup(createElement(GatewayPostureBody, { data, error }));

describe("the Posture view's gateway settings", () => {
  it("the_posture_view_shows_the_gateways_switches_keys_and_price_book", () => {
    const html = render(FORGE);
    for (const text of [
      "Firewall (taint)",
      ">enforce<",
      ">shadow<",
      ">5.0<",
      ">anthropic<",
      "5 of 6 listed",
      "openai/gpt-4.1-mini",
      "15 in / 75 out",
      "gpt-6-astra above 200,000 tokens: 4/18",
      "openai/gpt-4.1-nano",
    ]) {
      expect(html).toContain(text);
    }
    // A switch the gateway did not include reads as not reported, not off.
    expect(html).toContain(`FinOps router</span><span class="mono" style="font-size:11.5px;color:var(--faint);text-align:right">${NOT_REPORTED}`);
  });

  it("a_gateway_without_the_settings_route_reads_not_reported_never_off", () => {
    const html = render({ ...FORGE, settings: { reported: false, items: [] } });
    expect(html).toContain(NOT_REPORTED);
    expect(html).toContain("That is not the same as off");
    expect(html).not.toContain(">off<");
  });

  it("an older book says its tiers are not reported, and an empty one says none", () => {
    expect(render({ ...FORGE, price_book: { ...FORGE.price_book!, tiers: null } })).toContain(NOT_REPORTED);
    expect(render({ ...FORGE, price_book: { ...FORGE.price_book!, tiers: [] } })).toContain(">none<");
    const dropped = render({ ...FORGE, price_book: { ...FORGE.price_book!, tiers_dropped_models: ["gpt-6-astra"] } });
    expect(dropped).toContain("Tiers dropped by the operator file");
  });

  it("says each failed read and every empty state in words", () => {
    const failed = render({ ...FORGE, settings: null, keys: null, price_book: null, notes: ["GET /v1/settings did not answer: 500"] });
    expect(failed).toContain("the settings read failed");
    expect(failed).toContain("the keys read failed");
    expect(failed).toContain("GET /v1/settings did not answer: 500");
    expect(render(null)).toContain("reading the gateway");
    expect(render(null, { kind: "gateway", status: 401, message: "refused" })).toContain(
      "gateway posture could not be read: Gateway error 401: refused",
    );
    const many = render({ ...FORGE, price_book: { ...FORGE.price_book!, added_models: Array.from({ length: 15 }, (_, i) => `m${i}`) } });
    expect(many).toContain("(+3)");
  });

  it("loads into data or error, and the section waits for the gateway", async () => {
    const seen: { data: GatewayPosture[]; errors: (CredentialsError | null)[] } = { data: [], errors: [] };
    await postureLoader({ fetch: async () => FORGE, onData: (d) => seen.data.push(d), onError: (e) => seen.errors.push(e) })();
    const boom: CredentialsError = { kind: "unreachable", reason: "connection refused" };
    await postureLoader({
      fetch: async () => {
        throw boom;
      },
      onData: (d) => seen.data.push(d),
      onError: (e) => seen.errors.push(e),
    })();
    expect(seen.data).toEqual([FORGE]);
    expect(seen.errors).toEqual([null, boom]);
    expect(renderToStaticMarkup(createElement(GatewayPostureSection))).toContain("connecting to the gateway");
  });
});
