import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import {
  FreezeToggleButton,
  KillRunButton,
  RevokeDelegationButton,
  StopReportView,
  StopStartButton,
  agentBlockedStateFromRuns,
  agentStateFromRecord,
  entityAgentState,
  isUnitStopped,
  isUserStopped,
  runBlockedState,
} from "./lifecycle";

// The records are wire shapes with many fields the state rules never read.
const rec = (o: object) => o as never;

describe("an agent's state from its record", () => {
  it("no_record_reads_live_and_an_explicit_lifecycle_beats_every_flag", () => {
    expect(agentStateFromRecord(null)).toBe("live");
    expect(agentStateFromRecord(undefined)).toBe("live");
    expect(agentStateFromRecord(rec({ lifecycle: "frozen", closed: true, blocked: false }))).toBe("frozen");
  });

  it("closed_reads_killed_before_blocked_reads_frozen_and_neither_reads_live", () => {
    expect(agentStateFromRecord(rec({ closed: true, blocked: true }))).toBe("killed");
    expect(agentStateFromRecord(rec({ blocked: true }))).toBe("frozen");
    expect(agentStateFromRecord(rec({}))).toBe("live");
  });

  it("a_row_inside_a_unit_follows_the_same_order", () => {
    expect(entityAgentState(rec({ lifecycle: "killed" }))).toBe("killed");
    expect(entityAgentState(rec({ closed: true, blocked: true }))).toBe("killed");
    expect(entityAgentState(rec({ blocked: true }))).toBe("frozen");
    expect(entityAgentState(rec({}))).toBe("live");
  });
});

describe("a unit or a user is stopped", () => {
  for (const [name, fn] of [
    ["unit", isUnitStopped],
    ["user", isUserStopped],
  ] as const) {
    it(`a_${name}_is_stopped_by_its_own_flag_or_by_every_agent_being_blocked`, () => {
      expect(fn(null)).toBe(false);
      expect(fn(undefined)).toBe(false);
      expect(fn(rec({ stopped: true, agents: [{ blocked: false }] }))).toBe(true);
      expect(fn(rec({ stopped: false, agents: [{ blocked: true }] }))).toBe(false);
      expect(fn(rec({ agents: [{ blocked: true }, { blocked: true }] }))).toBe(true);
      expect(fn(rec({ agents: [{ blocked: true }, { blocked: false }] }))).toBe(false);
    });

    it(`a_${name}_with_no_agents_is_not_stopped`, () => {
      expect(fn(rec({ agents: [] }))).toBe(false);
    });
  }
});

describe("a run's blocked state", () => {
  it("a_non_live_lifecycle_wins_a_live_one_falls_through_to_killed_or_null", () => {
    expect(runBlockedState(rec({ lifecycle: "frozen", killed: true }))).toBe("frozen");
    expect(runBlockedState(rec({ lifecycle: "live", killed: true }))).toBe("killed");
    expect(runBlockedState(rec({ killed: true }))).toBe("killed");
    expect(runBlockedState(rec({}))).toBeNull();
  });

  it("an_agent_takes_the_first_non_live_state_of_its_runs_and_a_live_only_agent_is_absent", () => {
    const m = agentBlockedStateFromRuns([
      rec({ agent_id: "a", lifecycle: "frozen" }),
      rec({ agent_id: "a", killed: true }),
      rec({ agent_id: "b" }),
      rec({ agent_id: "c", killed: true }),
    ]);
    expect(m.get("a")).toBe("frozen");
    expect(m.get("c")).toBe("killed");
    expect(m.has("b")).toBe(false);
  });
});

const html = (el: Parameters<typeof renderToStaticMarkup>[0]) => renderToStaticMarkup(el);
const noop = async () => {};

describe("the action buttons say the opposite of the current state", () => {
  it("freeze_and_start_stop_toggle_their_label", () => {
    expect(html(createElement(FreezeToggleButton, { frozen: false, onToggle: noop }))).toContain("Freeze");
    expect(html(createElement(FreezeToggleButton, { frozen: true, onToggle: noop }))).toContain("Unfreeze");
    expect(html(createElement(StopStartButton, { stopped: false, onToggle: noop }))).toContain(">Stop<");
    expect(html(createElement(StopStartButton, { stopped: true, onToggle: noop }))).toContain(">Start<");
  });

  it("with_no_live_run_kill_is_a_disabled_button_that_says_why", () => {
    const out = html(createElement(KillRunButton, { run: null, onKill: noop }));
    expect(out).toContain("disabled");
    expect(out).toContain("No live run to kill");
  });

  it("with_a_live_run_kill_is_a_live_button", () => {
    const out = html(createElement(KillRunButton, { run: rec({ run_id: "r1" }), onKill: noop }));
    expect(out).toContain("Kill");
    expect(out).not.toContain("No live run to kill");
  });

  it("revoking_a_delegation_is_offered_even_with_nothing_running", () => {
    expect(html(createElement(RevokeDelegationButton, { agentId: "a", onRevoke: noop }))).toContain(
      "Revoke delegation",
    );
  });
});

describe("a stop report", () => {
  const report = {
    done: false,
    headline: "NOT stopped",
    lines: [
      { tone: "ok", text: "ban set" },
      { tone: "bad", text: "kill failed" },
      { tone: "warn", text: "slow" },
    ],
  };

  it("a_failed_report_is_red_says_not_and_lists_every_step_in_its_tone", () => {
    const out = html(createElement(StopReportView, { report: rec(report), onClose: () => {}, align: "inline" }));
    expect(out).toContain("NOT stopped");
    expect(out).toContain("var(--sev-critical)");
    for (const t of ["ban set", "kill failed", "slow"]) expect(out).toContain(t);
    expect(out).toContain("var(--mint)");
    expect(out).toContain("var(--sev-high)");
  });

  it("a_done_report_is_bordered_mint_and_floats_from_the_chosen_edge", () => {
    const out = html(
      createElement(StopReportView, { report: rec({ ...report, done: true }), onClose: () => {}, align: "left" }),
    );
    expect(out).toContain("border:1px solid var(--mint)");
    expect(out).toContain("position:absolute");
    expect(out).toContain("left:0");
  });
});
