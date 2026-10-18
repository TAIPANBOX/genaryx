import { describe, expect, it } from "vitest";

import type { LiftOutcome, StopOutcome, StopStep } from "../delegationTypes";
import { liftReport, stopReport } from "./stopAgent";

const AGENT = "agent://acme.example/bots/researcher";

function s(step: string, target: string, ok: boolean, verify: string, status = 200): StopStep {
  return { step, target, ok, http_status: status, verify_result: verify, journaled: true, journal_error: null };
}

const complete: StopOutcome = {
  subject: AGENT,
  stopped: true,
  ban_durable: true,
  open_runs: 2,
  runs_killed: 2,
  warnings: [],
  steps: [
    s("ceremony", AGENT, true, "confirmed:webauthn-es256", 0),
    s("ban", AGENT, true, "banned:durable"),
    s("list_runs", AGENT, true, "listed:2 open"),
    s("kill_run", "r-new", true, "killed:true"),
    s("kill_run", "r-old", true, "killed:true"),
    s("verdict", AGENT, true, "stopped", 0),
  ],
};

describe("stopReport", () => {
  it("a_complete_stop_reads_stopped_and_lists_every_step", () => {
    const r = stopReport({ ok: true, outcome: complete });
    expect(r.done).toBe(true);
    expect(r.headline).toContain("Stopped");
    expect(r.headline).toContain(AGENT);
    expect(r.lines.filter((l) => l.tone === "ok")).toHaveLength(complete.steps.length);
    expect(r.lines.some((l) => l.text.includes("banned:durable"))).toBe(true);
    expect(r.lines.some((l) => l.text.includes("r-old"))).toBe(true);
  });

  it("a_stop_with_a_failed_step_never_reads_as_stopped", () => {
    const outcome: StopOutcome = {
      ...complete,
      stopped: false,
      ban_durable: null,
      steps: [
        s("ceremony", AGENT, true, "confirmed:webauthn-es256", 0),
        s("ban", AGENT, false, "refused:invalid_client", 401),
        s("list_runs", AGENT, true, "listed:2 open"),
        s("kill_run", "r-new", true, "killed:true"),
        s("kill_run", "r-old", true, "killed:true"),
        s("verdict", AGENT, false, "not_stopped: ban refused:invalid_client", 0),
      ],
    };
    const r = stopReport({ ok: false, error: { kind: "not_stopped", outcome } });
    expect(r.done).toBe(false);
    expect(r.headline).toMatch(/not stopped/i);
    expect(r.headline).not.toMatch(/^Stopped/);
    const bad = r.lines.filter((l) => l.tone === "bad");
    expect(bad.some((l) => l.text.includes("refused:invalid_client") && l.text.includes("401"))).toBe(true);
    // A refusal before the command (no outcome at all) is not a stop either.
    const refused = stopReport({ ok: false, error: { kind: "role_required", role: "admin" } });
    expect(refused.done).toBe(false);
    expect(refused.headline).toMatch(/not stopped/i);
    expect(refused.lines.some((l) => l.tone === "bad" && l.text.includes("admin"))).toBe(true);
  });

  it("a_step_the_journal_did_not_take_is_said", () => {
    const outcome: StopOutcome = {
      ...complete,
      steps: complete.steps.map((st, i) =>
        i === 1 ? { ...st, journaled: false, journal_error: "no live event bus available" } : st,
      ),
    };
    const r = stopReport({ ok: true, outcome });
    expect(r.lines.some((l) => l.tone === "warn" && l.text.includes("no live event bus available"))).toBe(true);
  });

  it("a_ban_a_restart_forgets_is_a_warning", () => {
    const outcome: StopOutcome = {
      ...complete,
      ban_durable: false,
      warnings: ["the ban is not durable: vouchryx keeps no revocation store, so restarting it lifts the ban"],
    };
    const r = stopReport({ ok: true, outcome });
    expect(r.done).toBe(true);
    expect(r.lines.some((l) => l.tone === "warn" && l.text.includes("not durable"))).toBe(true);
  });
});

describe("an answer that is not an outcome", () => {
  it("an_answer_that_is_not_a_stop_outcome_never_reads_as_stopped", () => {
    // The demo transport answers null for a command it has no handler for,
    // and a wrong box could answer anything: none of it is a stop.
    const answers: unknown[] = [
      null,
      undefined,
      {},
      "ok",
      { subject: AGENT, stopped: false, steps: [] },
      { subject: AGENT, stopped: "true", steps: [] },
      { subject: AGENT, stopped: true },
    ];
    for (const a of answers) {
      const r = stopReport({ ok: true, outcome: a as StopOutcome });
      expect(r.done, JSON.stringify(a)).toBe(false);
      expect(r.headline, JSON.stringify(a)).toMatch(/not stopped/i);
      expect(r.lines.some((l) => l.tone === "bad")).toBe(true);
      const l = liftReport({ ok: true, outcome: a as LiftOutcome });
      expect(l.done, JSON.stringify(a)).toBe(false);
      expect(l.headline, JSON.stringify(a)).toMatch(/not lifted/i);
    }
    // And a refusal whose outcome is missing is still a refusal, not a crash.
    const r = stopReport({ ok: false, error: { kind: "not_stopped", outcome: null as unknown as StopOutcome } });
    expect(r.done).toBe(false);
    expect(r.headline).toMatch(/not stopped/i);
  });
});

describe("liftReport", () => {
  it("a_lift_vouchryx_refused_never_reads_as_lifted", () => {
    const outcome: LiftOutcome = {
      subject: AGENT,
      lifted: false,
      lift_durable: null,
      warnings: [],
      steps: [
        s("ceremony", AGENT, true, "confirmed:webauthn-es256", 0),
        s("lift", AGENT, false, "refused:invalid_request", 400),
        s("verdict", AGENT, false, "not_lifted: refused:invalid_request", 0),
      ],
    };
    const r = liftReport({ ok: false, error: { kind: "not_lifted", outcome } });
    expect(r.done).toBe(false);
    expect(r.headline).toMatch(/not lifted/i);
    expect(r.lines.some((l) => l.tone === "bad" && l.text.includes("refused:invalid_request"))).toBe(true);

    const lifted = liftReport({
      ok: true,
      outcome: {
        ...outcome,
        lifted: true,
        lift_durable: true,
        steps: [
          s("ceremony", AGENT, true, "confirmed:webauthn-es256", 0),
          s("lift", AGENT, true, "lifted:durable"),
          s("verdict", AGENT, true, "lifted", 0),
        ],
      },
    });
    expect(lifted.done).toBe(true);
    expect(lifted.headline).toContain("lifted");
  });
});
