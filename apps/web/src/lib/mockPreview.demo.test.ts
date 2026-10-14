import { describe, expect, it } from "vitest";

import { mockInvoke, simulatedNoticeText } from "./mockPreview";

// The demo answers every command from this mock. Two rules hold for each
// action it stands in for: the answer says plainly that nothing was done, and
// a panel that must never show invented data gets the same "no backend"
// answer the real build would give it.

describe("the simulated notice", () => {
  it("the_demo_marks_its_example_panels_as_simulated", () => {
    const text = simulatedNoticeText(true);

    expect(text).toContain("nothing was done");
    expect(text).toContain("no box was contacted");
  });

  it("the_real_console_renders_no_simulated_notice", () => {
    expect(simulatedNoticeText(false)).toBeNull();
  });
});

describe("the demo's quality baselines", () => {
  it("quality_baselines_answer_the_summary_shape_the_panel_reads", async () => {
    // QualityBaselines.tsx destructures `{ baseline, unanswered_* }` from each
    // row. A bare baseline row crashed the whole demo on the Quality tab.
    const rows = await mockInvoke<Array<{ baseline: { eval_run_id: string }; unanswered_supported: boolean }>>(
      "quality_list_baselines",
    );

    expect(rows.length).toBeGreaterThan(0);
    for (const row of rows) {
      expect(typeof row.baseline.eval_run_id).toBe("string");
      expect(typeof row.unanswered_supported).toBe("boolean");
    }
  });
});

describe("the demo's web egress", () => {
  it("the_demo_egress_panel_holds_no_invented_rows", async () => {
    // EgressView says in its own words that it has no demo data, by design.
    await expect(mockInvoke("egress_recent", { limit: 10 })).rejects.toEqual({ kind: "no_environment" });
  });
});

describe("the demo's remote actions", () => {
  it("the_demo_tunnel_connect_says_nothing_was_done", async () => {
    const status = await mockInvoke<{ tunnel: { state: string; message?: string } }>("remote_wg_connect");

    expect(status.tunnel.state).not.toBe("connected");
    expect(status.tunnel.message).toContain("nothing was done");
  });

  it("the_demo_ssh_file_read_says_nothing_was_read", async () => {
    const file = await mockInvoke<{ content: string }>("remote_ssh_read_file", { path: "/etc/hostname" });

    expect(file.content).toContain("nothing was done");
  });

  it("the_demo_wireguard_issue_and_revoke_leave_the_peer_list_as_it_was", async () => {
    const before = await mockInvoke<{ peers: Array<{ public_key: string }> }>("remote_operator_wg_peers");

    // Issuing a config is refused in the demo as well.
    await expect(mockInvoke("remote_operator_wg_config")).rejects.toMatchObject({ kind: "wg" });
    const afterIssue = await mockInvoke<{ peers: unknown[] }>("remote_operator_wg_peers");
    expect(afterIssue.peers).toHaveLength(before.peers.length);

    // Revoking is refused in the demo, so the device list cannot change.
    await expect(
      mockInvoke("remote_operator_wg_revoke", { public_key: before.peers[0].public_key }),
    ).rejects.toMatchObject({ kind: "wg" });
    const afterRevoke = await mockInvoke<{ peers: unknown[] }>("remote_operator_wg_peers");
    expect(afterRevoke.peers).toHaveLength(before.peers.length);
  });

  it("the_demo_felyx_connect_says_nothing_was_done", async () => {
    await expect(
      mockInvoke("copilot_connect", { provider: "ollama", model: "qwen2.5:7b-instruct", local: true }),
    ).rejects.toThrow("nothing was done");
  });
});
