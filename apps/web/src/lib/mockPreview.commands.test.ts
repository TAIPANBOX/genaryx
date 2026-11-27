import { describe, expect, it } from "vitest";

import { SIMULATED_NOTE, mockInvoke } from "./mockPreview";

type Rec = { id: string; owner: string; team: string; blocked?: boolean; budgetUsd: number };
type Receipt = { summary: string; http_status: number; verify_result: string; bus_recorded: boolean };

async function firstAgent(): Promise<Rec> {
  const g = await mockInvoke<{ nodes: { id: string; kind: string }[] }>("agent_graph");
  const id = g.nodes.find((n) => n.kind === "agent")?.id;
  expect(typeof id).toBe("string");
  return mockInvoke<Rec>("agent_record", { agent_id: id });
}

describe("the demo's remote and box commands refuse to pretend", () => {
  it("a_tunnel_connect_says_no_tunnel_was_opened_and_a_disconnect_resets_it", async () => {
    const s = await mockInvoke<{ tunnel: { state: string; message?: string } }>("remote_wg_connect");
    expect(s.tunnel.state).toBe("failed");
    expect(s.tunnel.message).toContain(SIMULATED_NOTE);
    const after = await mockInvoke<{ tunnel: { state: string } }>("remote_wg_disconnect");
    expect(after.tunnel.state).toBe("disconnected");
  });

  it("ssh_commands_throw_a_refusal_that_names_the_simulation_and_returns_no_host_data", async () => {
    await expect(mockInvoke("remote_ssh_tail_start")).rejects.toMatchObject({
      kind: "ssh",
      message: `${SIMULATED_NOTE}: no log was tailed from any box.`,
    });
    await expect(mockInvoke("remote_ssh_check_reachable")).rejects.toMatchObject({
      message: expect.stringContaining(SIMULATED_NOTE),
    });
    const file = await mockInvoke<{ content: string; valid_utf8: boolean; size_bytes: number }>(
      "remote_ssh_read_file",
    );
    expect(file.content).toContain("no file was read from any box");
    expect(file.size_bytes).toBe(file.content.length);
    await expect(mockInvoke("remote_ssh_tail_stop")).resolves.toBeTruthy();
  });

  it("changing_the_model_provider_is_refused_and_the_bus_says_it_is_a_demo", async () => {
    await expect(mockInvoke("copilot_connect")).rejects.toThrow(SIMULATED_NOTE);
    await expect(mockInvoke("bus_status")).resolves.toEqual({ kind: "demo", dir: "/preview" });
  });

  it("an_unknown_command_answers_an_empty_list_for_list_names_and_null_otherwise", async () => {
    await expect(mockInvoke("something_list_things")).resolves.toEqual([]);
    await expect(mockInvoke("something_unheard_of")).resolves.toBeNull();
  });

  it("an_unknown_agent_has_no_stops_and_says_so", async () => {
    const r = await mockInvoke<{ total: number; entries: unknown[]; note: string }>("agent_stops", {
      agent_id: "no-such-agent",
    });
    expect(r.total).toBe(0);
    expect(r.entries).toEqual([]);
    expect(r.note).toMatch(/nothing on the bus/);
  });
});

describe("the demo's money commands keep the operator's act", () => {
  it("a_budget_set_on_a_run_says_the_amount_and_is_recorded_on_the_bus", async () => {
    const r = await mockInvoke<Receipt>("money_set_budget", { run_id: "run-x", budget_usd: 12.5, reason: "cap" });
    expect(r.summary).toBe("Budget for run-x set to $12.50 - cap");
    expect(r.verify_result).toBe("budget_usd:12.5");
    expect(r.bus_recorded).toBe(true);
    expect(r.http_status).toBe(200);
  });

  it("a_budget_without_a_run_names_no_run_instead_of_inventing_one", async () => {
    const r = await mockInvoke<Receipt>("money_set_budget", {});
    expect(r.summary).toBe("Budget for run set to $0.00");
  });

  it("killing_a_run_twice_says_the_second_time_it_was_already_killed", async () => {
    const first = await mockInvoke<Receipt>("money_kill_run", { run_id: "run-kill-twice", reason: "loop" });
    expect(first.summary).toBe("Killed run-kill-twice - loop");
    const again = await mockInvoke<Receipt>("money_kill_run", { run_id: "run-kill-twice" });
    expect(again.summary).toBe("run-kill-twice was already killed");
  });

  it("acknowledging_an_incident_twice_says_it_was_already_acknowledged", async () => {
    const first = await mockInvoke<Receipt>("money_ack_incident", { id: "inc-ack-twice" });
    expect(first.summary).toBe("Acknowledged inc-ack-twice");
    const again = await mockInvoke<Receipt>("money_ack_incident", { id: "inc-ack-twice" });
    expect(again.summary).toBe("inc-ack-twice already acknowledged");
    expect(again.verify_result).toBe("acknowledged:true");
  });

  it("a_unit_cap_needs_a_unit_and_a_finite_non_negative_amount", async () => {
    for (const args of [
      { unit: "", budget_usd: 5 },
      { unit: "unassigned", budget_usd: 5 },
      { unit: "platform", budget_usd: -1 },
      { unit: "platform" },
    ]) {
      await expect(mockInvoke("money_set_unit_budget", args)).rejects.toMatchObject({
        kind: "invalid_argument",
      });
    }
    const ok = await mockInvoke<Receipt>("money_set_unit_budget", { unit: "platform", budget_usd: 0 });
    expect(ok.http_status).toBe(200);
  });
});

describe("the demo's agent commands change the agent and say what changed", () => {
  it("a_budget_edit_changes_the_per_run_ceiling_and_a_zero_keeps_the_old_one", async () => {
    const a = await firstAgent();
    const set = await mockInvoke<Rec>("agent_set_budget", { agent_id: a.id, budget_usd: 7.25 });
    expect(set.budgetUsd).toBe(7.25);
    const kept = await mockInvoke<Rec>("agent_set_budget", { agent_id: a.id, budget_usd: 0 });
    expect(kept.budgetUsd).toBe(7.25);
  });

  it("an_edit_to_an_unknown_agent_answers_null_and_changes_nothing", async () => {
    for (const cmd of ["agent_set_budget", "agent_reassign_unit", "agent_transfer_owner", "agent_set_behaviour", "agent_block"]) {
      await expect(mockInvoke(cmd, { agent_id: "ghost", team: "x", owner: "y", allowed: [] })).resolves.toBeNull();
    }
  });

  it("reassigning_a_unit_and_transferring_an_owner_move_the_agent_and_a_same_value_does_nothing", async () => {
    const a = await firstAgent();
    const moved = await mockInvoke<Rec>("agent_reassign_unit", { agent_id: a.id, team: "demo-team-z" });
    expect(moved.team).toBe("demo-team-z");
    const same = await mockInvoke<Rec>("agent_reassign_unit", { agent_id: a.id, team: "demo-team-z" });
    expect(same.team).toBe("demo-team-z");
    const owned = await mockInvoke<Rec>("agent_transfer_owner", { agent_id: a.id, owner: "demo-owner-z" });
    expect(owned.owner).toBe("demo-owner-z");
    const stay = await mockInvoke<Rec>("agent_transfer_owner", { agent_id: a.id, owner: "demo-owner-z" });
    expect(stay.owner).toBe("demo-owner-z");
  });

  it("setting_behaviour_takes_a_list_and_ignores_anything_else", async () => {
    const a = await firstAgent();
    const set = await mockInvoke<{ allowed?: string[] }>("agent_set_behaviour", { agent_id: a.id, allowed: ["read"] });
    expect(set.allowed).toEqual(["read"]);
    const ignored = await mockInvoke<{ allowed?: string[] }>("agent_set_behaviour", { agent_id: a.id, allowed: "all" });
    expect(ignored.allowed).toEqual(["read"]);
  });

  it("freezing_an_agent_blocks_it_and_unfreezing_lets_it_go_and_the_stop_history_lists_the_act", async () => {
    const a = await firstAgent();
    const frozen = await mockInvoke<Rec>("agent_block", { agent_id: a.id, blocked: true });
    expect(frozen.blocked).toBe(true);
    const stops = await mockInvoke<{ entries: { reason: string; by_operator: boolean }[] }>("agent_stops", {
      agent_id: a.id,
    });
    expect(stops.entries.some((e) => e.reason === "stopped from the console" && e.by_operator)).toBe(true);
    const free = await mockInvoke<Rec>("agent_block", { agent_id: a.id, blocked: false });
    expect(free.blocked).toBeFalsy();
  });

  it("stopping_and_starting_a_user_or_a_unit_flips_the_record_and_an_empty_name_does_nothing", async () => {
    const a = await firstAgent();
    for (const [cmd, key, val] of [
      ["user_block", "user", a.owner],
      ["unit_block", "team", a.team],
    ] as const) {
      const stopped = await mockInvoke<{ blocked?: boolean; stopped?: boolean }>(cmd, { [key]: val, blocked: true });
      expect(stopped).toBeTruthy();
      const rec = await mockInvoke<Rec>("agent_record", { agent_id: a.id });
      expect(rec.blocked).toBe(true);
      await mockInvoke(cmd, { [key]: val, blocked: false });
      const back = await mockInvoke<Rec>("agent_record", { agent_id: a.id });
      expect(back.blocked).toBeFalsy();
    }
    await expect(mockInvoke("user_block", { user: "", blocked: true })).resolves.toBeTruthy();
    await expect(mockInvoke("unit_block", { team: "", blocked: true })).resolves.toBeTruthy();
  });
});

describe("the demo's read commands answer a fixed shape", () => {
  it("the_on_demand_scans_and_builds_answer_without_a_box", async () => {
    const verify = await mockInvoke<{ verified: boolean; message: string }>("crypto_verify_evidence");
    expect(verify.verified).toBe(true);
    expect(verify.message).toMatch(/unsigned bundle/);
    for (const cmd of [
      "record_status",
      "record_verify",
      "quality_list_run_summaries",
      "quality_run_scores",
      "quality_list_baselines",
      "crypto_scan_ncsc",
      "crypto_scan_cbom",
      "crypto_scan_evidence",
      "drills_run",
      "evidence_build",
      "onboard_status",
      "copilot_ask",
      "policy_enforcement_status",
      "identity_list_remediations",
      "egress_recent",
      "notifications_sent",
      "keys_custody",
    ]) {
      await expect(mockInvoke(cmd, {})).resolves.not.toBeUndefined();
    }
    const v = await mockInvoke<{ verify: { state: string; findings: string[] } }>("record_verify");
    expect(v.verify.state).toBe("verified");
    expect(v.verify.findings.length).toBeGreaterThan(0);
  });
});
