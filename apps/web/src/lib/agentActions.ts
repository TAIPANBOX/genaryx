import { hasBackend, invokeBackend, isWebShell } from "./transport";
import { notifyConsoleStateChanged } from "./consoleState";
import type { AgentRecord } from "./agentRecord";
import { isPreviewOnlyRefusal } from "./previewOnly";

/**
 * Governance actions on an agent, from its detail card: reassign its business
 * unit, transfer its owner, edit its per-run budget, or edit its allowed
 * behaviour. Each returns the updated record and appends to the lifecycle.
 *
 * Only the preview backend keeps an editable agent record, so these four are
 * preview-only commands (`lib/previewOnly.ts`, invariant 50). On a real box
 * {@link canEditAgentRecord} is false, the card hides the four controls and
 * says why, and nothing is sent. Making them real needs an ownership and
 * behaviour store that does not exist yet, which is designed separately.
 */

export interface OrgDirectory {
  teams: { team: string; label: string }[];
  users: { handle: string; team: string }[];
}

/** The line the card shows on a real box in place of the four edit controls. */
export const EDIT_NEEDS_STORE =
  "Transfer owner, reassign unit, edit budget and edit behaviour need an ownership store, which this box does not keep yet. Freeze, kill, revoke and stop act on the planes and work here.";

/** True only where an editable agent record exists: the mock preview. A real
 * box (a web shell) and a bare build with no backend both answer false. */
export function canEditAgentRecord(): boolean {
  return hasBackend() && !isWebShell();
}

export async function fetchOrgDirectory(): Promise<OrgDirectory | null> {
  if (!canEditAgentRecord()) return null;
  try {
    return await invokeBackend<OrgDirectory | null>("org_directory");
  } catch {
    return null;
  }
}

/** The outcome of one edit: the record the backend answered, or the reason it
 * did not land, which the card prints. Never a silent `null`. */
export type EditOutcome = { ok: true; record: AgentRecord | null } | { ok: false; reason: string };

function editRefusalText(err: unknown): string {
  if (isPreviewOnlyRefusal(err)) return EDIT_NEEDS_STORE;
  if (typeof err === "string" && err) return err;
  if (err && typeof err === "object") {
    const msg = (err as { error?: unknown; message?: unknown }).error ?? (err as { message?: unknown }).message;
    if (typeof msg === "string" && msg) return msg;
  }
  return "the change did not land";
}

async function edit(command: string, args: Record<string, unknown>): Promise<EditOutcome> {
  if (!canEditAgentRecord()) return { ok: false, reason: EDIT_NEEDS_STORE };
  try {
    return { ok: true, record: await invokeBackend<AgentRecord | null>(command, args) };
  } catch (err) {
    return { ok: false, reason: editRefusalText(err) };
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
  edit("agent_set_budget", { agent_id: agentId, budget_usd: budgetUsd });

export const reassignAgentUnit = (agentId: string, team: string) =>
  edit("agent_reassign_unit", { agent_id: agentId, team });

export const transferAgentOwner = (agentId: string, owner: string) =>
  edit("agent_transfer_owner", { agent_id: agentId, owner });

export const setAgentBehaviour = (agentId: string, allowed: string[]) =>
  edit("agent_set_behaviour", { agent_id: agentId, allowed });

/** Freeze or unfreeze a single agent, in one idempotent toggle (freezing a
 * whole user or unit lives in `entityRecords.ts`, since those return the
 * user/unit aggregate, not one agent record). Broadcasts app-wide whenever the
 * command SUCCEEDS, including the `null` body a real box returns: the box
 * keeps no agent record to echo, it records the block in `lifecycle_blocks`
 * and stamps the agent's runs, both of which the broadcast makes every open
 * panel re-read. `act` already swallows a failure into `null`, so this can
 * over-notify at worst, never claim a block that was refused. */
export const blockAgent = (agentId: string, blocked: boolean) =>
  act("agent_block", { agent_id: agentId, blocked }).then((rec) => {
    notifyConsoleStateChanged();
    return rec;
  });
