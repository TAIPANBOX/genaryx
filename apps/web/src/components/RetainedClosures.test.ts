import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { RetainedClosuresBody } from "./RetainedClosures";
import type { RetainedError } from "../lib/retainedClosure";

const render = (rows: unknown, error: RetainedError | null = null) =>
  renderToStaticMarkup(
    createElement(RetainedClosuresBody, { rows, error, lastClosed: null, onClose: async () => null }),
  );

describe("RetainedClosuresBody", () => {
  it("the Money panel lists retained reservations with a Close action", () => {
    const out = render([
      { reservation_id: 7, revision: "0011223344556677", run_id: "run-a", chain: [], amount_microusd: 8657, unit: "finance", model: "m", retained_at_millis: 1 },
    ]);
    expect(out).toContain("#7");
    expect(out).toContain("run-a");
    expect(out).toContain("Close");
    expect(out).toContain("passkey");
  });

  it("says why it cannot list rather than showing nothing to close", () => {
    expect(render(null, { kind: "closure_off" })).toContain("closure route is off");
    expect(render(null, { kind: "no_environment" })).toContain("No gateway configured");
    expect(render({ not: "a list" })).toContain("could not be read");
    expect(render([])).toContain("no retained reservations to close");
  });
});
