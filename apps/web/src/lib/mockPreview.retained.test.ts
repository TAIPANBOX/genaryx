import { describe, expect, it } from "vitest";

import { mockInvoke } from "./mockPreview";

/**
 * The demo build runs on this mock transport. A command it does not know
 * falls to a catch-all that answers `[]` for any name containing `_runs`,
 * which is the wrong shape for `credentials_gateway_retained_runs` (an
 * object) and blanked the whole Money tab in mock mode.
 */
describe("mockInvoke credentials_gateway_retained_runs", () => {
  it("answers a retained report, not the catch-all's empty list", async () => {
    const r = await mockInvoke<{ runs: unknown; total_retained: unknown; total_retained_usd: unknown }>(
      "credentials_gateway_retained_runs",
    );
    expect(Array.isArray(r)).toBe(false);
    expect(Array.isArray(r.runs)).toBe(true);
    expect(typeof r.total_retained).toBe("number");
    expect(typeof r.total_retained_usd).toBe("number");
  });
});
