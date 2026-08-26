/**
 * What an approval carried, and what leaves the Policy tab as a file.
 *
 * # THE APPROVAL TRAIL IS THE DOCUMENT SOMEBODY ASKS FOR LATER
 *
 * "Who approved this, when, on what evidence" is the question an auditor asks
 * months after the console has scrolled past it, and until now the only answer
 * was a screenshot. So the approval export carries every field
 * `GET /v1/approvals` returns, including the two the inbox itself dropped
 * (`model` and `org`), and states in its own provenance block what each empty
 * cell means. Several of them mean different things, and getting that wrong in
 * a file that outlives the session is worse than not exporting at all.
 *
 * # THE POLICY LIST IS NOT THE ENFORCED RULE SET
 *
 * `GET /v1/policies` lists the STORE's operator-managed policies only. A
 * deployment whose rules come from a `-policy` file sees an empty list there
 * while every one of those rules is enforced (`WardryxClient::list_policies`'s
 * own doc comment says so, and `WardryxStatus.effective_policies` is the field
 * that answers "is anything enforced at all"). An export of this list that did
 * not say so would read as proof that a guarded fleet is wide open.
 */

import type { ExportMeta } from "./download";
import type { Approval, PolicyRecord } from "../policyTypes";

/** What the console says where a field never reached it. Same word the Money
 * tab uses (`lib/moneyExport.ts`), deliberately: one vocabulary for absence
 * across the console. */
export const NOT_RECORDED = "not recorded";

/** The model the held action declared.
 *
 * Wardryx carries it "for display only - the PDP never branches on it"
 * (`crates/connectors/src/wardryx.rs`), which is to say it was put on the wire
 * for exactly this and then rendered nowhere. `null` means the hold's context
 * carried no model, not that the model was hidden. */
export function approvalModelLabel(approval: Approval): string {
  return approval.model && approval.model.length > 0 ? approval.model : NOT_RECORDED;
}

/** The org the hold was stamped with, from the authenticated principal that
 * triggered it rather than from the request body (`api.go:278`). */
export function approvalOrgLabel(approval: Approval): string {
  return approval.org && approval.org.length > 0 ? approval.org : NOT_RECORDED;
}

/** The policy generation that decided this hold. Same absence wording as the
 * two above, because it is the same kind of absence: a key the context did not
 * carry. */
export function approvalPolicyVersionLabel(approval: Approval): string {
  return approval.policy_version && approval.policy_version.length > 0 ? approval.policy_version : NOT_RECORDED;
}

// ---- The approval trail ----------------------------------------------------

export interface ApprovalExportRow {
  approval_id: string;
  agent_id: string;
  run_id: string;
  requested_at: string;
  pending: boolean;
  decision: string | null;
  decided_at: string | null;
  decided_by: string | null;
  est_cost_usd: number | null;
  tool_names: string | null;
  on_behalf_of: string | null;
  policy_version: string | null;
  org: string | null;
  model: string | null;
  reason: string | null;
}

export const APPROVALS_EXPORT_COLUMNS: { key: keyof ApprovalExportRow & string; header: string }[] = [
  { key: "approval_id", header: "approval_id" },
  { key: "agent_id", header: "agent_id" },
  { key: "run_id", header: "run_id" },
  { key: "requested_at", header: "requested_at" },
  { key: "pending", header: "pending" },
  { key: "decision", header: "decision" },
  { key: "decided_at", header: "decided_at" },
  { key: "decided_by", header: "decided_by" },
  { key: "est_cost_usd", header: "est_cost_usd" },
  { key: "tool_names", header: "tool_names" },
  { key: "on_behalf_of", header: "on_behalf_of" },
  { key: "policy_version", header: "policy_version" },
  { key: "org", header: "org" },
  { key: "model", header: "model" },
  { key: "reason", header: "reason" },
];

function textOrNull(value: string | null | undefined): string | null {
  return value !== null && value !== undefined && value.length > 0 ? value : null;
}

/** A list field as one cell, or `null` when the list itself was absent.
 *
 * The two are not the same and the wire keeps them apart: `on_behalf_of` is
 * `null` when the request declared no delegation chain, and an array when it
 * declared one. An empty array flattened into the same blank as a missing
 * field would erase that distinction, so an empty array becomes an empty
 * string (present, nothing in it) and only a missing list becomes `null`. */
function listOrNull(value: string[] | null | undefined): string | null {
  if (value === null || value === undefined) return null;
  return value.join(" -> ");
}

export function approvalsExportRows(approvals: Approval[]): ApprovalExportRow[] {
  return approvals.map((a) => ({
    approval_id: a.approval_id,
    agent_id: a.agent_id,
    run_id: a.run_id,
    requested_at: a.requested_at,
    pending: a.pending,
    decision: textOrNull(a.decision),
    decided_at: textOrNull(a.decided_at),
    decided_by: textOrNull(a.decided_by),
    est_cost_usd: typeof a.est_cost_usd === "number" && Number.isFinite(a.est_cost_usd) ? a.est_cost_usd : null,
    tool_names: listOrNull(a.tool_names),
    on_behalf_of: listOrNull(a.on_behalf_of),
    policy_version: textOrNull(a.policy_version),
    org: textOrNull(a.org),
    model: textOrNull(a.model),
    reason: textOrNull(a.reason),
  }));
}

export function approvalsExportMeta(opts: {
  total: number;
  pending: number;
  environment: string;
  takenAt: string;
}): ExportMeta {
  return {
    subject: "Genaryx approval trail",
    environment: opts.environment,
    takenAt: opts.takenAt,
    windows: [
      "approvals: every hold the policy plane holds for this org, GET /v1/approvals, which is a bare array with no paging and no window parameter",
    ],
    caveats: [
      `${opts.total.toLocaleString("en-US")} approval(s) in this file, ${opts.pending.toLocaleString("en-US")} of them still pending. The inbox on screen shows the same set, split into a queue and a history.`,
      "The plane returns only the calling org's approvals, matched server-side against the hold's own org. This is not the whole estate's trail.",
      "An empty decision, decided_at or decided_by is a hold nobody has decided yet, which pending says outright.",
      "An empty est_cost_usd means the hold carried no estimate. It is not an estimate of zero.",
      "An empty on_behalf_of means the request declared no delegation chain. The plane stores that as null, so the chain was not lost in transit.",
      "An empty tool_names means the hold's context carried no tool list. This console cannot tell that from a hold that declared an empty one, because the two arrive identically.",
      "model and org are display-only context: the PDP never branches on the model, and org is stamped from the authenticated principal rather than from the request body.",
      "reason is the policy plane's own one-sentence explanation of the hold, not an operator's note.",
    ],
  };
}

// ---- The policy inventory --------------------------------------------------

export interface PolicyExportRow {
  id: string;
  name: string;
  target: string;
  deny_tool: string;
  allow_domains: string;
  require_human_above_usd: number;
  deny_above_usd: number;
  max_steps: number;
  deny_if_unattested: boolean;
  updated_at: string | null;
}

export const POLICIES_EXPORT_COLUMNS: { key: keyof PolicyExportRow & string; header: string }[] = [
  { key: "id", header: "id" },
  { key: "name", header: "name" },
  { key: "target", header: "target" },
  { key: "deny_tool", header: "deny_tool" },
  { key: "allow_domains", header: "allow_domains" },
  { key: "require_human_above_usd", header: "require_human_above_usd" },
  { key: "deny_above_usd", header: "deny_above_usd" },
  { key: "max_steps", header: "max_steps" },
  { key: "deny_if_unattested", header: "deny_if_unattested" },
  { key: "updated_at", header: "updated_at" },
];

export function policiesExportRows(policies: PolicyRecord[]): PolicyExportRow[] {
  return policies.map((p) => ({
    id: p.id,
    name: p.name,
    target: p.target,
    // Joined rather than nulled when empty: Wardryx omits an empty list on the
    // wire and this console reads the absence as "this policy denies no tool",
    // which is a statement about the policy, not a gap in the file.
    deny_tool: p.deny_tool.join(" "),
    allow_domains: p.allow_domains.join(" "),
    require_human_above_usd: p.require_human_above_usd,
    deny_above_usd: p.deny_above_usd,
    max_steps: p.max_steps,
    deny_if_unattested: p.deny_if_unattested,
    updated_at: textOrNull(p.updated_at),
  }));
}

export function policiesExportMeta(opts: {
  total: number;
  policyVersion: string | null;
  environment: string;
  takenAt: string;
}): ExportMeta {
  return {
    subject: "Genaryx policy inventory",
    environment: opts.environment,
    takenAt: opts.takenAt,
    windows: [
      "policies: the policy store as GET /v1/policies returned it at the moment above, which is a snapshot and not a history",
    ],
    caveats: [
      "INCOMPLETE BY CONSTRUCTION: this is the STORE's operator-managed policies only. Rules loaded from a wardryx -policy file are enforced and never appear here, so a short file is not evidence of a short rule set.",
      "This export does not read GET /v1/status, so it does not state how many policies the PDP is actually evaluating. That figure is the only one that answers whether anything is enforced at all.",
      opts.policyVersion
        ? `policy_version ${opts.policyVersion} is not a field of GET /v1/policies. It is this console's own best effort: the version stamped on the most recently requested approval.`
        : "policy_version is unknown: it is not a field of GET /v1/policies, and this console derives it from the most recently requested approval, of which there have been none.",
      "A 0 in require_human_above_usd, deny_above_usd or max_steps means the policy sets no such limit. Wardryx drops a zero-valued field on the wire, so an unset limit and one deliberately set to zero arrive identically and this console cannot tell them apart.",
      "An empty deny_tool or allow_domains means the policy names none, not that the list was lost.",
      "An empty updated_at means the store recorded no update time for that policy.",
      `${opts.total.toLocaleString("en-US")} policy record(s) in this file, which is every row the Policies table shows: neither is capped.`,
    ],
  };
}
