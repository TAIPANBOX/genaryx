import { describe, expect, it } from "vitest";

import { buildCloseArgs, describeRetainedError, isCloseOutcome } from "./retainedClosure";
import { mockInvoke } from "./mockPreview";

const row = { reservation_id: 7, revision: "0011223344556677" };

describe("buildCloseArgs", () => {
  it("an observed closure carries the reservation, its revision, the cost and the reference", () => {
    const r = buildCloseArgs(row, { evidenceKind: "observed", observedMicrousd: " 4210 ", evidenceRef: " INV-14 " });
    expect(r).toEqual({
      ok: true,
      args: {
        reservation_id: 7,
        expected_revision: "0011223344556677",
        evidence_kind: "observed",
        observed_microusd: 4210,
        evidence_ref: "INV-14",
      },
    });
  });

  it("a no-charge closure sends no amount even if one was typed", () => {
    const r = buildCloseArgs(row, { evidenceKind: "no_charge", observedMicrousd: "99", evidenceRef: "provider ticket 8" });
    expect(r.ok && r.args.observed_microusd).toBe(null);
  });

  it("refuses evidence the gateway would refuse, before any passkey is asked for", () => {
    const bad = [
      { evidenceKind: "observed" as const, observedMicrousd: "", evidenceRef: "x" },
      { evidenceKind: "observed" as const, observedMicrousd: "-5", evidenceRef: "x" },
      { evidenceKind: "observed" as const, observedMicrousd: "1.5", evidenceRef: "x" },
      { evidenceKind: "observed" as const, observedMicrousd: "99999999999999999999", evidenceRef: "x" },
      { evidenceKind: "no_charge" as const, observedMicrousd: "", evidenceRef: "   " },
      { evidenceKind: "no_charge" as const, observedMicrousd: "", evidenceRef: "a\nb" },
      { evidenceKind: "no_charge" as const, observedMicrousd: "", evidenceRef: "é".repeat(257) },
    ];
    for (const form of bad) expect(buildCloseArgs(row, form).ok, JSON.stringify(form)).toBe(false);
  });
});

describe("a closure is shown as closed only when the server confirmed it", () => {
  it("rejects null, a page, and an outcome that does not say closed", () => {
    expect(isCloseOutcome(null)).toBe(false);
    expect(isCloseOutcome("<html>")).toBe(false);
    expect(isCloseOutcome({ verify_result: "refused:x", operation_id: "o" })).toBe(false);
    expect(isCloseOutcome({ verify_result: "closed", operation_id: "genaryx-1" })).toBe(true);
    expect(isCloseOutcome({ verify_result: "closed (replayed)", operation_id: "genaryx-1" })).toBe(true);
  });

  it("names every refusal in words, including the gateway's own kind", () => {
    expect(describeRetainedError({ kind: "not_configured" })).toContain("GENARYX_CLOSURE_SECRET_FILE");
    expect(describeRetainedError({ kind: "refused", status: 409, error: "stale_revision", message: "m" })).toContain(
      "409 stale_revision",
    );
    expect(describeRetainedError({ kind: "unconfirmed", status: 200, body: {} })).toContain("not shown as closed");
    expect(describeRetainedError({ kind: "unconfirmed", status: 0, body: null })).toContain("No confirmed closure");
  });
});

describe("mockInvoke money_list_retained", () => {
  it("lists one reservation per retained count the demo's runs report shows", async () => {
    const rows = await mockInvoke<{ run_id: string; revision: string }[]>("money_list_retained");
    const report = await mockInvoke<{ total_retained: number }>("credentials_gateway_retained_runs");
    expect(rows.length).toBe(report.total_retained);
    for (const r of rows) expect(r.revision).toMatch(/^[0-9a-f]{16}$/);
  });
});
