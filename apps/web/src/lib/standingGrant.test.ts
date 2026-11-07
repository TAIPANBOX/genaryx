import { describe, expect, it } from "vitest";
import { heldTool, pinsTrustDomain, repeatOffers, standingGrantOffer } from "./standingGrant";
import type { Approval, PolicyRecord } from "../policyTypes";

const AGENT = "agent://acme.example/agents/bot-1";

const HOLD: Approval = {
  approval_id: "ap_1",
  agent_id: AGENT,
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

const granted = (id: string, over: Partial<Approval> = {}): Approval => ({
  ...HOLD,
  approval_id: id,
  pending: false,
  decision: "grant",
  decided_at: "2026-11-07T10:01:00Z",
  decided_by: "user://acme.example/ops",
  ...over,
});

const POLICY: PolicyRecord = {
  id: "grant-acme.example-agents-bot-1-2ffe7f24",
  name: "grant",
  target: AGENT,
  deny_tool: [],
  allow_domains: [],
  require_human_above_usd: 0,
  deny_above_usd: 0,
  max_steps: 0,
  deny_if_unattested: false,
  grant_tool: ["ask"],
  updated_at: null,
};

describe("what a held row offers", () => {
  it("a_floor_hold_offers_always_allow_for_its_tool", () => {
    expect(standingGrantOffer(HOLD, [], "admin")).toEqual({ kind: "ready", tool: "ask", staticKey: false });
  });

  it("an_id_with_no_trust_domain_is_off_and_the_line_says_why", () => {
    const offer = standingGrantOffer({ ...HOLD, agent_id: "agent-42" }, [], "admin");
    expect(offer.kind).toBe("blocked");
    expect(offer.kind === "blocked" && offer.line).toContain("agent-42 names no trust domain");
    expect(offer.kind === "blocked" && offer.line).toContain("Grant this call once");
  });

  it("a_glob_an_agent_wrote_is_not_a_trust_domain", () => {
    expect(pinsTrustDomain("agent://acme.example/*")).toBe(false);
    expect(pinsTrustDomain("agent://*/x")).toBe(false);
    expect(pinsTrustDomain("agent://")).toBe(false);
    expect(pinsTrustDomain("agent://acme.example:8443/a")).toBe(true);
    expect(pinsTrustDomain("agent://acme.example")).toBe(true);
  });

  it("a_hold_of_any_other_kind_offers_nothing", () => {
    expect(standingGrantOffer({ ...HOLD, hold_kind: null }, [], "admin")).toEqual({ kind: "none" });
  });

  it("a_hold_with_no_one_tool_is_off", () => {
    const several = { ...HOLD, tool_call: null, tool_names: ["a", "b"] };
    expect(heldTool(several)).toBeNull();
    expect(standingGrantOffer(several, [], "admin").kind).toBe("blocked");
  });

  it("an_approver_sees_the_button_off_and_why", () => {
    const offer = standingGrantOffer(HOLD, [], "approver");
    expect(offer.kind === "blocked" && offer.line).toContain("needs the admin role");
  });

  it("an_agent_that_already_holds_the_tool_is_told_so", () => {
    expect(standingGrantOffer(HOLD, [POLICY], "admin")).toEqual({ kind: "granted", tool: "ask" });
    expect(standingGrantOffer(HOLD, [{ ...POLICY, grant_tool: ["*"] }], "admin").kind).toBe("granted");
  });

  it("a_static_key_is_named_before_it_is_granted", () => {
    expect(standingGrantOffer({ ...HOLD, identity_source: "client-key" }, [], "admin")).toEqual({
      kind: "ready",
      tool: "ask",
      staticKey: true,
    });
  });
});

describe("the row that offers a standing grant after three grants", () => {
  it("three_grants_of_one_tool_offer_it", () => {
    const offers = repeatOffers([granted("a"), granted("b"), granted("c")], [], "admin");
    expect(offers).toEqual([{ agent_id: AGENT, tool: "ask", grants: 3, approval_id: "c" }]);
  });

  it("two_grants_offer_nothing", () => {
    expect(repeatOffers([granted("a"), granted("b")], [], "admin")).toEqual([]);
  });

  it("a_denial_a_pending_hold_or_a_cost_hold_does_not_count", () => {
    const rows = [
      granted("a"),
      granted("b"),
      granted("c", { decision: "deny" }),
      granted("d", { hold_kind: null }),
      { ...HOLD, approval_id: "e" },
    ];
    expect(repeatOffers(rows, [], "admin")).toEqual([]);
  });

  it("two_agents_or_two_tools_do_not_add_up", () => {
    const other = "agent://acme.example/agents/bot-2";
    const rows = [
      granted("a"),
      granted("b"),
      granted("c", { agent_id: other }),
      granted("d", { tool_names: ["fetch_url"], tool_call: { ...HOLD.tool_call!, name: "fetch_url" } }),
    ];
    expect(repeatOffers(rows, [], "admin")).toEqual([]);
  });

  it("a_tool_the_agent_already_holds_is_not_offered_again", () => {
    const rows = [granted("a"), granted("b"), granted("c")];
    expect(repeatOffers(rows, [POLICY], "admin")).toEqual([]);
    expect(repeatOffers(rows, [{ ...POLICY, grant_tool: ["*"] }], "admin")).toEqual([]);
  });

  it("an_agent_with_no_trust_domain_is_not_offered", () => {
    const rows = ["a", "b", "c"].map((id) => granted(id, { agent_id: "agent-42" }));
    expect(repeatOffers(rows, [], "admin")).toEqual([]);
  });

  it("nobody_but_an_admin_is_offered_it", () => {
    const rows = [granted("a"), granted("b"), granted("c")];
    expect(repeatOffers(rows, [], "approver")).toEqual([]);
    expect(repeatOffers(rows, [], "viewer")).toEqual([]);
  });
});
