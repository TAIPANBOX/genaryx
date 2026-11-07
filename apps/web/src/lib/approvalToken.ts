/**
 * A person's grant, in the only form an agent can use it: the raw token.
 *
 * Wardryx returns `approval_token` from the grant and never again, and the
 * enforcement point takes it, unchanged, in {@link APPROVAL_TOKEN_HEADER} on a
 * retry. So the answer to the grant is the one copy there is, and this module
 * is the one place the browser turns it into something held.
 *
 * A {@link GrantedToken} lives in a component's state and nowhere else
 * (CLAUDE.md invariant 46): never in browser storage, a URL, a log line or a
 * transcript, and it goes when the component does, so leaving the view drops it.
 * `scripts/approval-token-stays-in-memory.sh` names the modules allowed to touch
 * the field, and those modules may not reach any of those places.
 */

import type { DecideOutcome, DecodedToken } from "../policyTypes";

/** The header the enforcement point reads a granted token from on a retry. */
export const APPROVAL_TOKEN_HEADER = "x-fuse-approval-token";

/** One grant's token, as the operator gets to see it. */
export interface GrantedToken {
  approvalId: string;
  /** The raw token, exactly as wardryx minted it. */
  raw: string;
  /** What the token says it authorises, or `null` when its claims could not be
   * read. The raw token is shown either way: it is what the agent needs, and a
   * console that cannot read it is no reason to lose it. */
  claims: DecodedToken | null;
}

/** The token a decision's answer carries, or `null` when it carries none: a
 * denial, or a grant wardryx answered with no token. */
export function grantedTokenFrom(approvalId: string, outcome: DecideOutcome): GrantedToken | null {
  const raw = outcome.approval_token;
  return raw ? { approvalId, raw, claims: outcome.token } : null;
}

/** Add a grant to the ones on screen, newest first. A list and not a slot: each
 * token can be shown only this once, so a second grant must never take the place
 * of a first the operator has not copied yet. */
export function withGranted(list: readonly GrantedToken[], granted: GrantedToken): GrantedToken[] {
  return [granted, ...list];
}

/** The tokens left once the operator has dismissed one. */
export function withoutGranted(list: readonly GrantedToken[], approvalId: string): GrantedToken[] {
  return list.filter((g) => g.approvalId !== approvalId);
}

/** The plain line under a token. What it says it is bound to comes from the
 * token's own claims: wardryx binds a token to one call only when the hold named
 * one, so "bound to that call" would be false of any other. */
export function tokenPlainLine(claims: DecodedToken | null): string {
  if (claims === null) {
    return "This token allows one call and cannot be shown again. Its claims could not be read here, so this console cannot say what it is bound to.";
  }
  if (claims.tool_call_digest) {
    return "This token allows one call, bound to that call, and cannot be shown again.";
  }
  return "This token allows one call and cannot be shown again. The hold named no single call, so it is bound to this agent, run and tool set under the ceiling above, not to one call.";
}

/** The one exception to "one call", which this console cannot see from here. */
export const SINGLE_USE_EXCEPTION =
  "One call is what wardryx does unless a box sets WARDRYX_APPROVAL_SINGLE_USE=false. Leaving this view drops the token.";
