import { hasBackend, invokeBackend } from "./transport";

/**
 * The owner's fleet and the business unit's fleet, so a detail card can jump
 * from an agent to the human who owns it or the unit it belongs to, and on
 * from there. Same honesty note as `agentRecord`: only the preview backend
 * answers these; a real box derives unit and owner from the agent id and the
 * delegation chain and keeps no such aggregate, so the fetchers return `null`
 * there and the cards that need them simply are not offered.
 */

export interface EntityAgent {
  agentId: string;
  name: string;
  team: string;
  owner: string;
  model: string;
  /** Only this user's / unit's share of the agent's spend (its segments while
   * they owned it), not the agent's all-time total. */
  spentUsd: number;
  calls: number;
  closed: boolean;
  blocked: boolean;
  /** True when the agent is currently theirs; false when it is here only
   * because they owned it in the past. */
  current: boolean;
}

export interface UserRecord {
  handle: string;
  agents: EntityAgent[];
  totalSpentUsd: number;
  totalCalls: number;
  teams: string[];
}

export interface UnitRecord {
  team: string;
  agents: EntityAgent[];
  owners: string[];
  totalSpentUsd: number;
  totalCalls: number;
}

export async function fetchUserRecord(handle: string): Promise<UserRecord | null> {
  if (!hasBackend()) return null;
  try {
    return await invokeBackend<UserRecord | null>("user_record", { user: handle });
  } catch {
    return null;
  }
}

export async function fetchUnitRecord(team: string): Promise<UnitRecord | null> {
  if (!hasBackend()) return null;
  try {
    return await invokeBackend<UnitRecord | null>("unit_record", { team });
  } catch {
    return null;
  }
}

/** Disable or re-enable every agent a user owns, in one action. Returns the
 * refreshed user record. Null on a backend that keeps no such control. */
export async function blockUser(handle: string, blocked: boolean): Promise<UserRecord | null> {
  if (!hasBackend()) return null;
  try {
    return await invokeBackend<UserRecord | null>("user_block", { user: handle, blocked });
  } catch {
    return null;
  }
}

/** Disable or re-enable every agent in a business unit, in one action. */
export async function blockUnit(team: string, blocked: boolean): Promise<UnitRecord | null> {
  if (!hasBackend()) return null;
  try {
    return await invokeBackend<UnitRecord | null>("unit_block", { team, blocked });
  } catch {
    return null;
  }
}
