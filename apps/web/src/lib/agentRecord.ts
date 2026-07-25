import { hasBackend, invokeBackend } from "./transport";

/**
 * The richer, owned-thing view of an agent: who owns it, which business unit it
 * belongs to, its budget and allowed-behaviour envelope, and its lifecycle
 * (launched, owned, transferred, closed + why). This is what the agent detail
 * popover shows beyond the money/identity/graph facts the console already
 * reads.
 *
 * HONESTY: today only the preview backend answers `agent_record`. A real
 * genaryx-web has no such command yet (an agent's unit is derived from the
 * `agent://org/team/name` id it emits, not a mutable owned record, and there is
 * no transfer/lifecycle store), so `fetchAgentRecord` returns `null` there and
 * the card simply omits the lifecycle/ownership/actions sections rather than
 * inventing them. Building that store is tracked work, not something this UI
 * pretends already exists.
 */

export type LifecycleKind = "launched" | "owned" | "transferred" | "budget_set" | "closed";

export interface LifecycleEntry {
  ts: string;
  kind: LifecycleKind;
  detail: string;
  actor: string;
}

export interface AttributionSegment {
  owner: string;
  team: string;
  spentUsd: number;
  from: string;
  to: string | null;
}

export interface AgentRecord {
  team: string;
  name: string;
  model: string;
  owner: string;
  budgetUsd: number;
  allowed: string[];
  spentUsd: number;
  calls: number;
  /** Spend split by ownership period, so an owner or unit is only charged for
   * what the agent spent while it was theirs. */
  segments?: AttributionSegment[];
  blocked?: boolean;
  /** Effective operator-lifecycle state (MOCK-ONLY enrichment): killed (run
   * killed or closed-for-cause) / frozen (this agent frozen) / stopped (its
   * unit or owner stopped) / live. A real box omits it and the card derives
   * the badge from `blocked`/`closed` instead. See `lib/lifecycleTypes.ts`. */
  lifecycle?: import("./lifecycleTypes").EntityLifecycleState;
  history: LifecycleEntry[];
  closed?: { by: string; reason: string; wrongdoing: string; ts: string };
}

/** The agent's owned-record, or `null` when the backend does not keep one
 * (every real box today). Never throws. */
export async function fetchAgentRecord(agentId: string): Promise<AgentRecord | null> {
  if (!hasBackend()) return null;
  try {
    return await invokeBackend<AgentRecord | null>("agent_record", { agent_id: agentId });
  } catch {
    // A real box answers 404/unknown-command here; that is the honest "no
    // owned record kept" state, not an error worth surfacing.
    return null;
  }
}

/** Short helpers shared by the card and the fleet directory. */
export function teamOf(agentId: string): string {
  const m = /^agent:\/\/[^/]+\/([^/]+)\//.exec(agentId);
  return m ? m[1] : "";
}

export function userHandle(userUri: string): string {
  const m = /\/([^/]+)$/.exec(userUri);
  return m ? m[1] : userUri;
}
