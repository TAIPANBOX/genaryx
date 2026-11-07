import type { Approval, PolicyRecord } from "../policyTypes";
import type { ConsoleRole } from "./transport";

/**
 * What the inbox says about a standing grant: "always allow this tool for this
 * agent". The command checks all of this again on wardryx's own record of the
 * hold (`crates/api/src/policy/standing_grant.rs`), so this file decides what a
 * person is shown and nothing about what happens: a line here that lied would be
 * refused there.
 */

/** Grants of one tool to one agent before the inbox offers to stop asking. */
export const REPEATS_BEFORE_OFFER = 3;

/** `hold_kind` on a tool call nothing grants, the one hold a grant lifts. */
const NO_GRANT = "no_grant";

/** The same pattern as `pins_trust_domain` in the command: `agent-allow`'s, with no
 * `*`, because this id is whatever the caller wrote and not a glob a person typed. */
const TRUST_DOMAIN_ID = /^agent:\/\/[A-Za-z0-9._~-]+(:[0-9]+)?(\/[A-Za-z0-9._~:/-]*)?$/;

export const pinsTrustDomain = (agentId: string): boolean => TRUST_DOMAIN_ID.test(agentId);

/** The one tool of a hold, or `null` when it names none or several. */
export function heldTool(approval: Approval): string | null {
  const called = approval.tool_call?.name;
  if (called) return called;
  return approval.tool_names.length === 1 ? approval.tool_names[0] : null;
}

/** Whether a stored policy already lifts the floor for this agent and tool. Exact
 * targets only: what an inbox row offers is a policy on one agent. */
export function alreadyGranted(policies: readonly PolicyRecord[], agentId: string, tool: string): boolean {
  return policies.some((p) => p.target === agentId && (p.grant_tool.includes("*") || p.grant_tool.includes(tool)));
}

/** What one held row shows beside Grant and Deny. */
export type StandingGrantOffer =
  /** Not a hold a grant lifts: the row says nothing about it. */
  | { kind: "none" }
  /** The agent already holds it. The call on the row still waits for an answer. */
  | { kind: "granted"; tool: string }
  /** The button is there and off, and the line says why. */
  | { kind: "blocked"; line: string }
  | { kind: "ready"; tool: string; staticKey: boolean };

export function standingGrantOffer(
  approval: Approval,
  policies: readonly PolicyRecord[],
  role: ConsoleRole | null | undefined,
): StandingGrantOffer {
  if (approval.hold_kind !== NO_GRANT) return { kind: "none" };
  if (!pinsTrustDomain(approval.agent_id)) {
    return {
      kind: "blocked",
      line: `Always allow is off: ${approval.agent_id} names no trust domain, and wardryx refuses a grant that does not. Grant this call once, or give the agent an id of the form agent://<domain>/<name>.`,
    };
  }
  const tool = heldTool(approval);
  if (tool === null) {
    return { kind: "blocked", line: "Always allow is off: this hold has no one tool to allow. Decide this call." };
  }
  if (alreadyGranted(policies, approval.agent_id, tool)) return { kind: "granted", tool };
  if (role && role !== "admin") {
    return { kind: "blocked", line: `Always allow needs the admin role: it outlives this call. You are signed in as ${role}.` };
  }
  return { kind: "ready", tool, staticKey: approval.identity_source === "client-key" };
}

/** An agent and a tool a person keeps answering the same way. */
export interface RepeatOffer {
  agent_id: string;
  tool: string;
  grants: number;
  /** The latest of those holds: what the command reads the agent and the tool from. */
  approval_id: string;
}

/**
 * The agents and tools granted at least {@link REPEATS_BEFORE_OFFER} times that a
 * standing grant could stop asking about. Only the no-grant floor counts: a grant
 * of a cost hold says nothing about the tool. An admin's offer: nobody else could
 * take it. `approvals` is in `requested_at` order, so the last hold seen is the
 * latest.
 */
export function repeatOffers(
  approvals: readonly Approval[],
  policies: readonly PolicyRecord[],
  role: ConsoleRole | null | undefined,
): RepeatOffer[] {
  if (role && role !== "admin") return [];
  const seen = new Map<string, RepeatOffer>();
  for (const a of approvals) {
    if (a.pending || a.decision !== "grant" || a.hold_kind !== NO_GRANT) continue;
    const tool = heldTool(a);
    if (tool === null || !pinsTrustDomain(a.agent_id)) continue;
    const key = `${a.agent_id}\n${tool}`;
    const grants = (seen.get(key)?.grants ?? 0) + 1;
    seen.set(key, { agent_id: a.agent_id, tool, grants, approval_id: a.approval_id });
  }
  return Array.from(seen.values()).filter(
    (o) => o.grants >= REPEATS_BEFORE_OFFER && !alreadyGranted(policies, o.agent_id, o.tool),
  );
}
