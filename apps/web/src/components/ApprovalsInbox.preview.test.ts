import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ApprovalsInbox, grantGate } from "./ApprovalsInbox";
import type { Approval, ApprovalPreview } from "../policyTypes";

const PVD = "2".repeat(64);

const HOLD: Approval = {
  approval_id: "ap_1",
  agent_id: "agent://acme.example/agents/bot-1",
  run_id: "run-1",
  requested_at: "2026-11-12T10:00:00Z",
  decided_at: null,
  decided_by: null,
  decision: null,
  pending: true,
  tool_names: ["gh_api"],
  est_cost_usd: null,
  reason: "no grant for gh_api",
  on_behalf_of: null,
  policy_version: "pv-1",
  org: "acme",
  model: null,
  tool_call: { name: "gh_api", target: "github", arguments_truncated: false, digest: "1".repeat(64) },
  hold_kind: "no_grant",
  identity_source: "agent-token",
  preview: { state: "sealed", pvd: PVD },
  require_preview: false,
};

const OPENED: ApprovalPreview = {
  approval_id: "ap_1",
  state: "sealed",
  pvd: PVD,
  ruleset: "r1",
  expires_at: "2026-11-13T10:00:00Z",
  document: { v: 1, ruleset: "r1", root: { k: "null" } },
};

function inbox(approvals: Approval[]): string {
  return renderToStaticMarkup(
    createElement(ApprovalsInbox, {
      approvals,
      policies: [],
      role: "admin",
      onDecide: async () => {},
      onAlwaysAllow: async () => {},
      grantedTokens: [],
      onDismissToken: () => {},
      focusApprovalId: null,
      mutedKeys: new Set<string>(),
      onToggleMuteAgent: () => {},
      onOpenAgent: () => {},
    }),
  );
}

describe("a held row and the preview of its arguments", () => {
  it("a_sealed_hold_offers_to_show_its_arguments_and_holds_the_grant_until_then", () => {
    const html = inbox([HOLD]);
    expect(html).toContain("Show arguments");
    expect(html).toMatch(/<button[^>]*disabled[^>]*>Grant<\/button>/);
    expect(html).toContain("sealed for this console");
  });

  it("a_grant_of_a_sealed_hold_is_allowed_once_the_preview_is_open", () => {
    expect(grantGate(HOLD, null).kind).toBe("blocked");
    const open = grantGate(HOLD, OPENED);
    expect(open.kind).toBe("grant");
  });

  it("a_call_whose_arguments_could_not_be_shown_is_granted_unseen_and_says_so", () => {
    for (const state of ["truncated", "off", "unavailable"]) {
      const gate = grantGate({ ...HOLD, preview: { state, pvd: null } }, null);
      expect(gate).toEqual({ kind: "grant", label: "Grant unseen", confirm: "Confirm grant without seeing the arguments" });
    }
    const html = inbox([{ ...HOLD, preview: { state: "off", pvd: null } }]);
    expect(html).toContain("Grant unseen");
    expect(html).not.toContain("Show arguments");
  });

  it("a_policy_that_requires_a_preview_blocks_an_unseen_grant", () => {
    const gate = grantGate({ ...HOLD, preview: { state: "truncated", pvd: null }, require_preview: true }, null);
    expect(gate.kind).toBe("blocked");
  });

  it("a_hold_from_a_wardryx_without_previews_keeps_the_plain_grant", () => {
    const old = { ...HOLD, preview: null };
    expect(grantGate(old, null)).toEqual({ kind: "grant", label: "Grant", confirm: "Confirm grant" });
    const html = inbox([old]);
    expect(html).toContain("not sent by wardryx");
  });
});
