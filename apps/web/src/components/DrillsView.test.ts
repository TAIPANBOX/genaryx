import { describe, expect, it } from "vitest";

import { environmentLabel } from "./DrillsView";
import type { DrillsStatus } from "../drillsTypes";

type Ready = Extract<DrillsStatus, { state: "ready" }>;

function ready(over: Partial<Ready> = {}): Ready {
  return {
    state: "ready",
    source: { source: "taipan", name: "acme-staging" },
    mockryx_bin: "/Users/x/.taipan/bin/mockryx",
    gateway_url: "http://127.0.0.1:8080",
    has_api_key: true,
    scenario_dir: "/Users/x/Development/mockryx/scenarios",
    ...over,
  };
}

/**
 * "Runs real gateway calls and burns real budget" is written under the button.
 * WHICH environment it burns them in came down the wire in `status.source`
 * (`drills::env::EnvSource`, the `taipan up` descriptor's own name) and was
 * never shown, so the only thing on screen to tell two environments apart was
 * a loopback URL that is identical in both.
 */
describe("environmentLabel", () => {
  it("names the taipan environment the gateway was resolved from", () => {
    expect(environmentLabel(ready())).toContain("acme-staging");
  });

  it("says the environment is unnamed rather than implying a default", () => {
    const out = environmentLabel(ready({ source: undefined as unknown as Ready["source"] }));
    expect(out).toContain("not recorded");
    expect(out).not.toContain("undefined");
  });
});
