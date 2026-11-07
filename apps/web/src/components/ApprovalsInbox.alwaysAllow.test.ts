import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ApprovalsInbox } from "./ApprovalsInbox";
import type { ConsoleRole } from "../lib/transport";
import type { Approval, PolicyRecord } from "../policyTypes";

const HOLD: Approval = {
  approval_id: "ap_1",
  agent_id: "agent://acme.example/agents/bot-1",
  run_id: "run-1",
  requested_at: "2026-11-07T10:00:00Z",
  decided_at: null,
  decided_by: null,
  decision: null,
  pending: true,
  tool_names: ["ask"],
  est_cost_usd: null,
  reason: "no grant for ask",
  on_behalf_of: null,
  policy_version: "pv-1",
  org: "acme",
  model: null,
  tool_call: { name: "ask", target: "typryx", arguments_truncated: false, digest: "aa11" },
  hold_kind: "no_grant",
  identity_source: "agent-token",
};

function inbox(approvals: Approval[], role: ConsoleRole | null = "admin", policies: PolicyRecord[] = []): string {
  return renderToStaticMarkup(
    createElement(ApprovalsInbox, {
      approvals,
      policies,
      role,
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

const buttonNamed = (html: string, name: string): string | undefined =>
  html.match(new RegExp(`<button[^>]*>${name}</button>`))?.[0];

describe("always allow on a held row", () => {
  it("the_row_keeps_grant_and_deny_and_adds_always_allow_before_them", () => {
    const html = inbox([HOLD]);
    const always = html.indexOf(">Always allow<");
    const grant = html.indexOf(">Grant<");
    const deny = html.indexOf(">Deny<");
    expect(always).toBeGreaterThan(-1);
    expect(always).toBeLessThan(grant);
    expect(grant).toBeLessThan(deny);
    expect(buttonNamed(html, "Always allow")).not.toContain("disabled");
    expect(buttonNamed(html, "Grant")).not.toContain("disabled");
  });

  it("the_button_is_quieter_than_grant_so_grant_stays_the_default", () => {
    const html = inbox([HOLD]);
    expect(buttonNamed(html, "Always allow")).toContain("var(--dim)");
    expect(buttonNamed(html, "Grant")).toContain("var(--sev-low)");
  });

  it("a_row_with_no_trust_domain_has_the_button_off_and_the_line_and_grant_still_works", () => {
    const html = inbox([{ ...HOLD, agent_id: "agent-42" }]);
    expect(buttonNamed(html, "Always allow")).toContain("disabled");
    expect(html).toContain("agent-42 names no trust domain");
    expect(buttonNamed(html, "Grant")).not.toContain("disabled");
  });

  it("a_hold_on_spend_shows_no_such_button", () => {
    expect(inbox([{ ...HOLD, hold_kind: null }])).not.toContain("Always allow");
  });

  it("a_row_the_agent_already_holds_says_this_call_still_waits", () => {
    const policy: PolicyRecord = {
      id: "g",
      name: "g",
      target: HOLD.agent_id,
      deny_tool: [],
      allow_domains: [],
      require_human_above_usd: 0,
      deny_above_usd: 0,
      max_steps: 0,
      deny_if_unattested: false,
      grant_tool: ["ask"],
      updated_at: null,
    };
    const html = inbox([HOLD], "admin", [policy]);
    expect(html).toContain("This call still waits for an answer");
    expect(html).not.toContain("Always allow<");
  });

  it("a_static_key_is_said_on_the_row", () => {
    expect(inbox([{ ...HOLD, identity_source: "client-key" }])).toContain("called with a static key");
  });
});

describe("the repeat row", () => {
  const granted = (id: string): Approval => ({ ...HOLD, approval_id: id, pending: false, decision: "grant" });

  it("the_third_grant_of_a_tool_puts_a_row_in_the_inbox_offering_it", () => {
    const html = inbox([granted("a"), granted("b"), granted("c")]);
    expect(html).toContain("was granted");
    expect(html).toContain("3 times. Always allow it?");
    expect(html).toContain(">Always allow<");
  });

  it("the_second_grant_puts_no_row_in_the_inbox", () => {
    expect(inbox([granted("a"), granted("b")])).not.toContain("Always allow it?");
  });
});
