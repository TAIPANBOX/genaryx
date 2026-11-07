/**
 * What the operator is told about a token, and what happens to a list of them.
 *
 * The panel is rendered for real (`react-dom/server`): a helper returning the
 * right sentence proves nothing about a panel that never prints it. What a token
 * does over time, and what it never reaches, needs a live DOM and is in
 * `components/approvalTokenInTheBrowser.test.ts`.
 */

import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { GrantedTokenPanel } from "../components/ApprovalsInbox";
import type { DecideOutcome, DecodedToken } from "../policyTypes";
import {
  APPROVAL_TOKEN_HEADER,
  grantedTokenFrom,
  tokenPlainLine,
  withGranted,
  withoutGranted,
  type GrantedToken,
} from "./approvalToken";

const RAW = "invented-claims-segment.invented-signature-segment";

const CLAIMS: DecodedToken = {
  agent_id: "agent://example.invalid/agents/held",
  run_id: "run-held",
  tools: ["fetch_url"],
  cost_ceiling_usd: 0.5,
  exp_unix: 4_102_444_800,
  tool_call_digest: "ab".repeat(32),
};

const OUTCOME: DecideOutcome = {
  summary: "approval ap_1 granted",
  http_status: 200,
  verify_result: "granted ceiling_usd:0.50 ttl_s:600",
  sig_alg: "webauthn-es256",
  sig_fpr: "cred",
  token: CLAIMS,
  approval_token: RAW,
  bus_recorded: true,
  bus_error: null,
};

function panel(granted: GrantedToken): string {
  return renderToStaticMarkup(createElement(GrantedTokenPanel, { granted, onDismiss: () => {} }));
}

describe("the token a grant hands the operator", () => {
  it("the_panel_shows_the_raw_token_the_header_and_the_countdown", () => {
    const html = panel({ approvalId: "ap_1", raw: RAW, claims: CLAIMS });
    expect(html).toContain(RAW);
    expect(html).toContain("x-fuse-approval-token");
    expect(APPROVAL_TOKEN_HEADER).toBe("x-fuse-approval-token");
    expect(html).toContain("expires in");
    expect(html).toContain("Copy the approval token");
  });

  it("the_panel_says_the_token_allows_one_call_bound_to_it_and_cannot_be_shown_again", () => {
    expect(panel({ approvalId: "ap_1", raw: RAW, claims: CLAIMS })).toContain(
      "This token allows one call, bound to that call, and cannot be shown again.",
    );
  });

  it("a_token_granted_on_a_hold_with_no_call_does_not_claim_to_be_bound_to_one", () => {
    const unbound = { ...CLAIMS, tool_call_digest: null };
    const html = panel({ approvalId: "ap_1", raw: RAW, claims: unbound });
    expect(html).not.toContain("bound to that call");
    expect(html).toContain("The hold named no single call");
    expect(html).toContain("cannot be shown again");
  });

  it("a_token_whose_claims_could_not_be_read_is_still_shown", () => {
    const html = panel({ approvalId: "ap_1", raw: RAW, claims: null });
    expect(html).toContain(RAW);
    expect(html).toContain("x-fuse-approval-token");
    expect(html).not.toContain("expired");
    expect(html).toContain("Its claims could not be read here");
  });

  it("an_expired_token_is_no_longer_drawn", () => {
    const html = panel({ approvalId: "ap_1", raw: RAW, claims: { ...CLAIMS, exp_unix: 1 } });
    expect(html).toContain("expired");
    expect(html).not.toContain(RAW);
  });

  it("the_exception_to_one_call_is_named_where_the_operator_reads_it", () => {
    expect(panel({ approvalId: "ap_1", raw: RAW, claims: CLAIMS })).toContain("WARDRYX_APPROVAL_SINGLE_USE=false");
  });

  it("a_grant_answer_becomes_a_token_and_a_denial_does_not", () => {
    expect(grantedTokenFrom("ap_1", OUTCOME)).toEqual({ approvalId: "ap_1", raw: RAW, claims: CLAIMS });
    expect(grantedTokenFrom("ap_1", { ...OUTCOME, token: null, approval_token: null })).toBeNull();
    expect(grantedTokenFrom("ap_1", { ...OUTCOME, approval_token: "" })).toBeNull();
    expect(grantedTokenFrom("ap_1", { ...OUTCOME, token: null })).toEqual({ approvalId: "ap_1", raw: RAW, claims: null });
  });

  it("a_second_grant_never_takes_the_place_of_a_first_token", () => {
    const first: GrantedToken = { approvalId: "ap_1", raw: "first.token", claims: CLAIMS };
    const second: GrantedToken = { approvalId: "ap_2", raw: "second.token", claims: CLAIMS };
    const both = withGranted(withGranted([], first), second);
    expect(both.map((g) => g.approvalId)).toEqual(["ap_2", "ap_1"]);
    expect(withoutGranted(both, "ap_2").map((g) => g.approvalId)).toEqual(["ap_1"]);
    expect(withoutGranted(withoutGranted(both, "ap_2"), "ap_1")).toEqual([]);
  });

  it("every_plain_line_says_the_token_cannot_be_shown_again", () => {
    for (const claims of [CLAIMS, { ...CLAIMS, tool_call_digest: null }, null]) {
      expect(tokenPlainLine(claims)).toContain("cannot be shown again");
      expect(tokenPlainLine(claims)).toContain("allows one call");
    }
  });
});
