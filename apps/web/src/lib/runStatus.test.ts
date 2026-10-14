import { describe, expect, it } from "vitest";

import {
  decisionsReported,
  latestRefusal,
  refusedCountLabel,
  runDecisionSentence,
  runAttentionRank,
  runModelSummary,
  runSignals,
  runSiteLabel,
  statusTitle,
} from "./runStatus";
import type { Run } from "../moneyTypes";

/**
 * The words for how the gateway decided a run, as Incident 360 and the runs
 * board both print them. Figures from the forge lab, 2026-10-14.
 */
const OLD: Run = {
  run_id: "mig-flint",
  unit: "",
  owner: "",
  model: "claude-haiku-4-5",
  agent_id: "agent://taipanbox.dev/routers/flint",
  spent_usd: 0.00321,
  budget_usd: 0.0045,
  budgets_read: true,
  calls: 3,
  cache_hits: 0,
  steps: 3,
  last_seen: "2026-10-05T09:00:00Z",
  killed: false,
};

const V18: Run = {
  ...OLD,
  site: "forge",
  blocked: 0,
  last_decision: "allow",
  last_decision_at: "2026-10-14T13:50:00.000Z",
  fallback_calls: 0,
  basis_unreported_calls: 0,
  identity_warned: 0,
  last_identity_reason: "",
};

describe("Incident 360's sentence about how the gateway decided the run", () => {
  it("a_refused_run_is_described_with_its_reason_in_incident_360", () => {
    const s = runDecisionSentence({ ...V18, blocked: 1, last_decision: "budget_exceeded" });
    expect(s).toContain("latest call refused: budget_exceeded");
    expect(s).toContain("1 call refused");
  });

  it("names every charge and identity signal the run carries", () => {
    const s = runDecisionSentence({
      ...V18,
      blocked: 3,
      fallback_calls: 2,
      basis_unreported_calls: 1,
      identity_warned: 4,
      last_identity_reason: "agent_id_not_allowed",
    });
    expect(s).toContain("latest call admitted (allow)");
    expect(s).toContain("3 calls refused");
    expect(s).toContain("2 charged at the fallback rate");
    expect(s).toContain("1 with no price basis named");
    expect(s).toContain("4 forwarded under identity warn (agent_id_not_allowed)");
  });

  it("says no decision was recorded rather than calling the run admitted", () => {
    expect(runDecisionSentence({ ...V18, last_decision: "" })).toContain("no decision recorded yet");
  });

  it("says an older Cloud does not report decisions, never that nothing was refused", () => {
    const s = runDecisionSentence(OLD);
    expect(s).toContain("does not report refusals");
    expect(s).not.toContain("0 calls refused");
  });
});

describe("the helpers behind the runs board", () => {
  it("treats null and absent alike, and a negative or non-finite count as unreported", () => {
    expect(decisionsReported(OLD)).toBe(false);
    expect(decisionsReported({ ...OLD, blocked: null })).toBe(false);
    expect(decisionsReported({ ...OLD, blocked: -1 })).toBe(false);
    expect(decisionsReported({ ...OLD, blocked: Number.NaN })).toBe(false);
    expect(decisionsReported(V18)).toBe(true);
    expect(refusedCountLabel({ ...V18, blocked: Number.POSITIVE_INFINITY })).toBeNull();
    expect(refusedCountLabel({ ...V18, blocked: 1200 })).toBe("1,200 refused");
  });

  it("explains a plain live pill only when the Cloud does not report refusals", () => {
    expect(statusTitle(V18)).toBeUndefined();
    expect(statusTitle(OLD)).toContain("older than TokenFuse v1.8.0");
  });

  it("bounds a refusal string from outside so it cannot push the row apart", () => {
    const long = "x".repeat(500);
    const r = latestRefusal({ ...V18, last_decision: long });
    expect(r).not.toBeNull();
    expect(r!.word.length).toBeLessThanOrEqual(24);
    expect(r!.title.length).toBeLessThan(220);
    expect(runSiteLabel({ ...V18, site: long }).length).toBeLessThanOrEqual(53);
  });

  it("puts the time of the refusal in its hover", () => {
    const r = latestRefusal({ ...V18, last_decision: "loop_detected", last_decision_at: "2026-10-14T13:41:00.000Z" });
    expect(r!.word).toBe("loop");
    expect(r!.title).toContain("at 2026-10-14T13:41:00.000Z");
  });

  it("names the identity reason only on an identity refusal", () => {
    const withReason = { ...V18, last_identity_reason: "key_has_no_unit_binding" };
    expect(latestRefusal({ ...withReason, last_decision: "identity_mismatch" })!.title).toContain(
      "key_has_no_unit_binding",
    );
    expect(latestRefusal({ ...withReason, last_decision: "taint_blocked" })!.title).not.toContain(
      "key_has_no_unit_binding",
    );
  });

  it("gives no pill for a zero or an unreported field, and keeps a fixed order", () => {
    expect(runSignals(V18)).toEqual([]);
    expect(runSignals(OLD)).toEqual([]);
    const keys = runSignals({ ...V18, identity_warned: 1, fallback_calls: 1, basis_unreported_calls: 1 }).map((s) => s.key);
    expect(keys).toEqual(["fallback", "basis_unreported", "identity_warn"]);
    const warn = runSignals({ ...V18, identity_warned: 1 })[0];
    expect(warn.title).not.toContain(": ");
  });

  it("sorting_by_status_puts_killed_then_refused_then_flagged_runs_first", () => {
    expect(runAttentionRank({ ...V18, killed: true })).toBe(3);
    expect(runAttentionRank({ ...V18, blocked: 2, last_decision: "taint_blocked" })).toBe(2);
    expect(runAttentionRank({ ...V18, fallback_calls: 5 })).toBe(1);
    expect(runAttentionRank(V18)).toBe(0);
    expect(runAttentionRank(OLD)).toBe(0);
  });

  it("says the model is not recorded, and falls back to the list's last when the Cloud sent no model", () => {
    expect(runModelSummary({ ...V18, model: "" }).label).toBe("last model not recorded");
    const s = runModelSummary({
      ...V18,
      model: "",
      models: [
        { model: "a", calls: 1, spent_usd: 0.1 },
        { model: "b", calls: 1, spent_usd: 0.2 },
      ],
    });
    expect(s.label).toBe("b +1");
    expect(s.title).not.toContain("changed");
    expect(runModelSummary({ ...V18, model: "m", model_switches: 2 }).title).toContain("changed 2 time(s)");
  });
});
