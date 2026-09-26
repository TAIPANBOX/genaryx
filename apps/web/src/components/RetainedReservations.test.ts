import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { RetainedReservationsBody, retainedFeedItems } from "./RetainedReservations";
import type { CredentialsStatus, GatewayRetained } from "../lib/credentials";

/**
 * The gateway's retained-reservations panel must never show a `0` as if
 * measured when the gateway plane is not configured or not reachable - it
 * must say so instead. `RetainedReservationsBody` takes explicit props so
 * every one of those states, and the ready state with real data, can be
 * rendered and asserted on without a live gateway or a DOM.
 */

const render = (status: CredentialsStatus | null, data: GatewayRetained | null, error: Parameters<typeof RetainedReservationsBody>[0]["error"] = null) =>
  renderToStaticMarkup(createElement(RetainedReservationsBody, { status, data, error }));

describe("RetainedReservationsBody", () => {
  it("says it is connecting rather than showing a zero while bootstrapping", () => {
    const out = render(null, null);
    expect(out).toContain("connecting to the gateway");
    expect(out).not.toContain("held after unknown outcome");
  });

  it("says no gateway is configured rather than showing a zero", () => {
    const out = render({ state: "no_environment" }, null);
    expect(out).toContain("no gateway configured");
    expect(out).not.toContain("held after unknown outcome");
  });

  it("names the reason when the gateway is unreachable, never a zero", () => {
    const out = render(
      { state: "unreachable", source: { source: "taipan", name: "p1" }, gateway_url: "http://x", reason: "connection refused" },
      null,
    );
    expect(out).toContain("connection refused");
    expect(out).not.toContain("held after unknown outcome");
  });

  it("shows the total held and the plain-sentence reason once the gateway answers", () => {
    const data: GatewayRetained = {
      runs: [{ run_id: "r1", retained: 1, retained_usd: 0.5 }],
      total_retained: 1,
      total_retained_usd: 0.5,
    };
    const out = render({ state: "ready", source: { source: "taipan", name: "p1" }, gateway_url: "http://x" }, data);
    expect(out).toContain("held after unknown outcome");
    expect(out).toContain("neither spent nor released");
    expect(out).toContain("r1");
  });

  it("says nothing is held rather than an empty table with no explanation", () => {
    const data: GatewayRetained = { runs: [], total_retained: 0, total_retained_usd: 0 };
    const out = render({ state: "ready", source: { source: "taipan", name: "p1" }, gateway_url: "http://x" }, data);
    expect(out).toContain("no reservations currently held");
  });
});

describe("retainedFeedItems", () => {
  it("pluralizes the reservation count", () => {
    const data: GatewayRetained = {
      runs: [
        { run_id: "one", retained: 1, retained_usd: 0.1 },
        { run_id: "many", retained: 3, retained_usd: 0.3 },
      ],
      total_retained: 4,
      total_retained_usd: 0.4,
    };
    const items = retainedFeedItems(data);
    expect(items[0].sub).toBe("1 reservation held");
    expect(items[1].sub).toBe("3 reservations held");
  });
});
