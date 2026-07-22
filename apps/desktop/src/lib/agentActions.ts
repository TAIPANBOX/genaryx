import { hasBackend, invokeBackend } from "./transport";
import type { AgentRecord } from "./agentRecord";

/**
 * Governance actions on an agent, from its detail card: reassign its business
 * unit, transfer its owner, edit its per-run budget, or edit its allowed
 * behaviour. Each returns the updated record and appends to the lifecycle.
 *
 * HONESTY: only the preview backend keeps an editable agent record, so these
 * work in the preview and are unavailable on a real box (the fetchers return
 * null, and the card should tell the operator so rather than pretend the change
 * landed). Making these real needs an ownership/behaviour store that does not
 * exist yet, which is tracked work.
 */

export interface OrgDirectory {
  teams: { team: string; label: string }[];
  users: { handle: string; team: string }[];
}

export async function fetchOrgDirectory(): Promise<OrgDirectory | null> {
  if (!hasBackend()) return null;
  try {
    return await invokeBackend<OrgDirectory | null>("org_directory");
  } catch {
    return null;
  }
}

async function act(command: string, args: Record<string, unknown>): Promise<AgentRecord | null> {
  if (!hasBackend()) return null;
  try {
    return await invokeBackend<AgentRecord | null>(command, args);
  } catch {
    return null;
  }
}

export const setAgentBudget = (agentId: string, budgetUsd: number) =>
  act("agent_set_budget", { agent_id: agentId, budget_usd: budgetUsd });

export const reassignAgentUnit = (agentId: string, team: string) =>
  act("agent_reassign_unit", { agent_id: agentId, team });

export const transferAgentOwner = (agentId: string, owner: string) =>
  act("agent_transfer_owner", { agent_id: agentId, owner });

export const setAgentBehaviour = (agentId: string, allowed: string[]) =>
  act("agent_set_behaviour", { agent_id: agentId, allowed });

/** Disable or re-enable a single agent. Blocking a whole user or unit lives in
 * `entityRecords.ts` because those return the user/unit aggregate, not one
 * agent record. */
export const blockAgent = (agentId: string, blocked: boolean) =>
  act("agent_block", { agent_id: agentId, blocked });
