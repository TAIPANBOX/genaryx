/**
 * The Approvals Inbox shows what the policy plane put on the wire, and the
 * trail that leaves the console says what each of its blanks means.
 *
 * `ApprovalsInbox` is rendered for real here (`react-dom/server`, no DOM
 * needed) rather than exercised through its label helpers alone. The defect
 * being held against regression is "the field never renders", and a helper
 * returning the right string proves nothing about a component that never calls
 * it. The inbox takes plain props, and its one effect (scroll-into-view on a
 * deep link) does not run in a static render, so the static markup is the real
 * component.
 */

import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ApprovalsInbox } from "../components/ApprovalsInbox";
import {
  approvalArgumentsTitle,
  approvalModelLabel,
  approvalOrgLabel,
  approvalPolicyVersionLabel,
  approvalReasonLabel,
  APPROVALS_EXPORT_COLUMNS,
  approvalsExportMeta,
  approvalsExportRows,
  approvalTargetLabel,
  NOT_RECORDED,
  POLICIES_EXPORT_COLUMNS,
  policiesExportMeta,
  policiesExportRows,
  visibleText,
} from "./policyExport";
import { toCsv, toJson } from "./download";
import type { Approval, PolicyRecord } from "../policyTypes";

const HOLD: Approval = {
  approval_id: "apr-9f31",
  agent_id: "agent://meridian.example/treasury/cashflow-forecaster",
  run_id: "run-0217",
  requested_at: "2026-08-26T08:41:00Z",
  decided_at: null,
  decided_by: null,
  decision: null,
  pending: true,
  tool_names: ["payments.transfer"],
  est_cost_usd: 42.5,
  reason: "estimated cost above the human-approval threshold",
  on_behalf_of: ["person://meridian.example/t.hollis"],
  policy_version: "pv-2026-08-19",
  org: "meridian-treasury",
  model: "claude-opus-4",
  tool_call: {
    name: "payments.transfer",
    target: "https://bank.meridian.example/transfers",
    arguments_truncated: false,
    digest: "aa11",
  },
};

const DECIDED: Approval = {
  ...HOLD,
  approval_id: "apr-7c02",
  pending: false,
  decision: "grant",
  decided_at: "2026-08-26T08:45:00Z",
  decided_by: "person://meridian.example/a.okafor",
  model: "claude-haiku-3",
};

const POLICY: PolicyRecord = {
  id: "pol-treasury-01",
  name: "Treasury outbound payments",
  target: "agent://meridian.example/treasury/*",
  deny_tool: ["shell.exec"],
  allow_domains: ["api.meridian.example"],
  require_human_above_usd: 25,
  deny_above_usd: 500,
  max_steps: 40,
  deny_if_unattested: true,
  updated_at: "2026-08-19T11:02:00Z",
};

function inbox(approvals: Approval[]): string {
  return renderToStaticMarkup(
    createElement(ApprovalsInbox, {
      approvals,
      onDecide: async () => {},
      grantedTokens: [],
      onDismissToken: () => {},
      focusApprovalId: null,
      mutedKeys: new Set<string>(),
      onToggleMuteAgent: () => {},
      onOpenAgent: () => {},
    }),
  );
}

describe("an operator deciding a hold can see what it is about", () => {
  it("a_row_shows_why_the_hold_happened_and_the_target_of_the_tool", () => {
    const html = inbox([HOLD]);
    expect(html).toContain("estimated cost above the human-approval threshold");
    expect(html).toContain("https://bank.meridian.example/transfers");
  });

  it("a_decided_row_keeps_the_reason_and_the_target", () => {
    const html = inbox([DECIDED]);
    expect(html).toContain("estimated cost above the human-approval threshold");
    expect(html).toContain("https://bank.meridian.example/transfers");
  });

  it("a_row_says_wardryx_did_not_send_the_arguments", () => {
    expect(inbox([HOLD])).toContain("not sent by wardryx");
    expect(inbox([DECIDED])).toContain("not sent by wardryx");
  });

  it("a_row_says_when_the_arguments_were_truncated", () => {
    const truncated = { ...HOLD, tool_call: { ...HOLD.tool_call!, arguments_truncated: true } };
    const html = inbox([truncated]);
    expect(html).toContain("truncated");
    expect(html).toContain("a grant binds only the tool and target");
    expect(inbox([HOLD])).not.toContain("truncated");
  });

  it("a_hold_with_no_single_call_says_so_instead_of_leaving_a_blank", () => {
    const html = inbox([{ ...HOLD, tool_call: null }]);
    expect(html).toContain("this hold names no single call");
  });

  it("a_hold_with_no_reason_says_so_instead_of_leaving_a_blank", () => {
    expect(inbox([{ ...HOLD, reason: null }])).toContain("wardryx gave no reason for this hold");
  });

  it("a_target_that_did_not_arrive_is_not_recorded_and_an_empty_one_is_none_stated", () => {
    const call = HOLD.tool_call!;
    expect(approvalTargetLabel({ ...HOLD, tool_call: { ...call, target: null } })).toBe(NOT_RECORDED);
    expect(approvalTargetLabel({ ...HOLD, tool_call: { ...call, target: "" } })).toBe("none stated");
  });

  it("the_arguments_hover_names_the_digest_a_grant_binds", () => {
    expect(approvalArgumentsTitle(HOLD)).toContain("aa11");
    expect(approvalArgumentsTitle({ ...HOLD, tool_call: { ...HOLD.tool_call!, digest: null } })).toContain("no readable digest");
    expect(approvalArgumentsTitle({ ...HOLD, tool_call: null })).toContain("not one call");
  });

  it("a_target_cannot_hide_part_of_itself_from_the_person_reading_it", () => {
    const call = HOLD.tool_call!;
    const hostile = { ...HOLD, tool_call: { ...call, target: "safe.example\u202e/moc.live\u0000\n" } };
    const label = approvalTargetLabel(hostile);
    expect(label).toBe("safe.example\\u{202e}/moc.live\\u{0}\\u{a}");
    expect(visibleText("plain text, https://a.example/b?c=d")).toBe("plain text, https://a.example/b?c=d");
    expect(approvalReasonLabel({ ...HOLD, reason: "held\u2028now" })).toBe("held\\u{2028}now");
  });

  it("two_hundred_seeds_of_hostile_targets_never_reach_a_row_with_a_hidden_character", () => {
    const hidden = /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/u;
    const pool = ["\u202e", "\u200b", "\u2066", "\u0000", "\n", "\t", "\ufeff", "a", "/", " ", "\u{e0041}"];
    let state = 0x2545f491;
    for (let seed = 0; seed < 200; seed++) {
      let target = "";
      for (let i = 0; i < 24; i++) {
        state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
        target += pool[state % pool.length];
      }
      expect(approvalTargetLabel({ ...HOLD, tool_call: { ...HOLD.tool_call!, target } })).not.toMatch(hidden);
    }
  });

  it("the_approval_trail_carries_the_target_and_whether_the_arguments_were_truncated", () => {
    const [row] = approvalsExportRows([{ ...HOLD, tool_call: { ...HOLD.tool_call!, arguments_truncated: true } }]);
    expect(row.target).toBe("https://bank.meridian.example/transfers");
    expect(row.arguments_truncated).toBe(true);
    const [none] = approvalsExportRows([{ ...HOLD, tool_call: null }]);
    expect(none.target).toBeNull();
    expect(none.arguments_truncated).toBeNull();
  });
});

describe("the approvals inbox renders the context the hold arrived with", () => {
  it("shows the model the operator is being asked to spend money on", () => {
    // wardryx.rs: "carried through for display only". It was put on the wire
    // for this and reached no pixel.
    expect(inbox([HOLD])).toContain("claude-opus-4");
  });

  it("shows the org the hold was stamped with", () => {
    expect(inbox([HOLD])).toContain("meridian-treasury");
  });

  it("labels both, so a reader can tell which value is which", () => {
    const html = inbox([HOLD]);
    expect(html).toContain(">model<");
    expect(html).toContain(">org<");
  });

  it("keeps showing the policy version it already showed", () => {
    expect(inbox([HOLD])).toContain("pv-2026-08-19");
  });

  it("says a hold carried no model rather than leaving the row to imply one", () => {
    const html = inbox([{ ...HOLD, model: null, org: null }]);
    expect(html).toContain(NOT_RECORDED);
    // The absent value must not be dressed as a figure: no tabular styling.
    expect(html).not.toMatch(/mono tabular[^>]*>\s*not recorded/);
  });

  it("carries the model onto the decided row too, which is the half read back later", () => {
    expect(inbox([DECIDED])).toContain("claude-haiku-3");
  });
});

describe("the approval labels keep an absence apart from a value", () => {
  it("reads the three context fields off a hold", () => {
    expect(approvalModelLabel(HOLD)).toBe("claude-opus-4");
    expect(approvalOrgLabel(HOLD)).toBe("meridian-treasury");
    expect(approvalPolicyVersionLabel(HOLD)).toBe("pv-2026-08-19");
  });

  it("says so for null and for the empty string alike", () => {
    expect(approvalModelLabel({ ...HOLD, model: null })).toBe(NOT_RECORDED);
    expect(approvalModelLabel({ ...HOLD, model: "" })).toBe(NOT_RECORDED);
    expect(approvalOrgLabel({ ...HOLD, org: null })).toBe(NOT_RECORDED);
    expect(approvalPolicyVersionLabel({ ...HOLD, policy_version: null })).toBe(NOT_RECORDED);
  });
});

describe("the approval trail is the whole inbox, pending and decided", () => {
  const META = { total: 2, pending: 1, environment: "console.example", takenAt: "2026-08-26T09:00:00.000Z" };

  it("carries the two fields the inbox itself had dropped", () => {
    const [row] = approvalsExportRows([HOLD]);
    expect(row.model).toBe("claude-opus-4");
    expect(row.org).toBe("meridian-treasury");
  });

  it("carries a decided hold with its decision, decider and time", () => {
    const [row] = approvalsExportRows([DECIDED]);
    expect(row.pending).toBe(false);
    expect(row.decision).toBe("grant");
    expect(row.decided_by).toBe("person://meridian.example/a.okafor");
    expect(row.decided_at).toBe("2026-08-26T08:45:00Z");
  });

  it("writes an undecided hold's decision fields as null, not as a decision", () => {
    const [row] = approvalsExportRows([HOLD]);
    expect(row.decision).toBeNull();
    expect(row.decided_by).toBeNull();
    expect(row.decided_at).toBeNull();
    expect(row.pending).toBe(true);
  });

  it("keeps a missing delegation chain apart from an empty one", () => {
    expect(approvalsExportRows([{ ...HOLD, on_behalf_of: null }])[0].on_behalf_of).toBeNull();
    expect(approvalsExportRows([{ ...HOLD, on_behalf_of: [] }])[0].on_behalf_of).toBe("");
  });

  it("never turns a missing estimate into an estimate of zero", () => {
    expect(approvalsExportRows([{ ...HOLD, est_cost_usd: null }])[0].est_cost_usd).toBeNull();
    expect(approvalsExportRows([{ ...HOLD, est_cost_usd: 0 }])[0].est_cost_usd).toBe(0);
  });

  it("says the file is one org's trail and not the estate's", () => {
    const joined = (approvalsExportMeta(META).caveats ?? []).join(" ");
    expect(joined).toMatch(/only the calling org/);
    expect(joined).toMatch(/not the whole estate/i);
  });

  it("says what an empty estimate means, since it is the one that reads as cheap", () => {
    const joined = (approvalsExportMeta(META).caveats ?? []).join(" ");
    expect(joined).toMatch(/not an estimate of zero/);
  });

  it("writes the provenance block into both formats", () => {
    const meta = approvalsExportMeta(META);
    expect(meta.subject).toBe("Genaryx approval trail");
    expect(meta.windows.length).toBeGreaterThan(0);
    expect((meta.caveats ?? []).length).toBeGreaterThan(0);

    const csv = toCsv(APPROVALS_EXPORT_COLUMNS, approvalsExportRows([HOLD, DECIDED]), meta);
    expect(csv.startsWith("# subject: Genaryx approval trail")).toBe(true);
    expect(csv).toContain("# environment: console.example");
    expect(csv).toContain("# taken_at: 2026-08-26T09:00:00.000Z");
    // A reason carries commas and must survive as one cell.
    expect(csv).toContain("estimated cost above the human-approval threshold");
    expect(csv.trim().split("\n").filter((l) => !l.startsWith("#"))).toHaveLength(3);

    const json = JSON.parse(toJson(approvalsExportRows([HOLD]), meta)) as { meta: unknown; rows: unknown[] };
    expect(json.meta).toEqual(meta);
    expect(json.rows).toHaveLength(1);
  });
});

describe("the policy inventory says what it is not", () => {
  const META = {
    total: 1,
    policyVersion: "pv-2026-08-19",
    environment: "console.example",
    takenAt: "2026-08-26T09:00:00.000Z",
  };

  it("carries every field of the record, including the name the table has no column for", () => {
    const [row] = policiesExportRows([POLICY]);
    expect(row.name).toBe("Treasury outbound payments");
    expect(row.deny_tool).toBe("shell.exec");
    expect(row.allow_domains).toBe("api.meridian.example");
    expect(row.deny_if_unattested).toBe(true);
    expect(row.max_steps).toBe(40);
  });

  it("says outright that an enforced -policy file never appears in this list", () => {
    const joined = (policiesExportMeta(META).caveats ?? []).join(" ");
    expect(joined).toMatch(/INCOMPLETE BY CONSTRUCTION/);
    expect(joined).toMatch(/-policy file/);
    expect(joined).toMatch(/short file is not evidence of a short rule set/);
  });

  it("says a zero limit and an unset limit arrive identically", () => {
    const joined = (policiesExportMeta(META).caveats ?? []).join(" ");
    expect(joined).toMatch(/cannot tell them apart/);
  });

  it("says where the policy_version came from, and says when there is none", () => {
    expect((policiesExportMeta(META).caveats ?? []).join(" ")).toMatch(/not a field of GET \/v1\/policies/);
    const none = policiesExportMeta({ ...META, policyVersion: null });
    expect((none.caveats ?? []).join(" ")).toMatch(/policy_version is unknown/);
  });

  it("writes the provenance block and one row per policy", () => {
    const csv = toCsv(POLICIES_EXPORT_COLUMNS, policiesExportRows([POLICY]), policiesExportMeta(META));
    expect(csv.startsWith("# subject: Genaryx policy inventory")).toBe(true);
    expect(csv.trim().split("\n").filter((l) => !l.startsWith("#"))).toHaveLength(2);
  });
});
