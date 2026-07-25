import { hasBackend, invokeBackend } from "./transport";
import { notifyConsoleStateChanged } from "./consoleState";
import type { EntityLifecycleState } from "./lifecycleTypes";

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
  /** Effective operator-lifecycle state (MOCK-ONLY enrichment), so each agent
   * row can show its own LIVE/STOPPED/FROZEN/KILLED badge. Omitted by a real
   * box, where the row falls back to `blocked`/`closed`. */
  lifecycle?: EntityLifecycleState;
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
  /** Whether this user is currently stopped (all their owned agents halted).
   * MOCK-ONLY: a real box omits it and the card treats the user as running
   * until a real `user_block` handler ships. */
  stopped?: boolean;
}

export interface UnitRecord {
  team: string;
  agents: EntityAgent[];
  owners: string[];
  totalSpentUsd: number;
  totalCalls: number;
  /** Whether this unit is currently stopped (all its agents halted).
   * MOCK-ONLY, same honesty note as `UserRecord.stopped`. */
  stopped?: boolean;
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

/** Stop or start every agent a user owns, in one idempotent toggle. Returns the
 * refreshed user record. Null on a backend that keeps no such control (every
 * real box today), where the caller leaves its state untouched - an honest
 * no-op, not a faked success. Broadcasts app-wide on any non-null result so
 * every open panel re-reads the new state. */
export async function blockUser(handle: string, blocked: boolean): Promise<UserRecord | null> {
  if (!hasBackend()) return null;
  try {
    const rec = await invokeBackend<UserRecord | null>("user_block", { user: handle, blocked });
    // Broadcast on SUCCESS, not on a non-null body: a real box answers `null`
    // here (it keeps no per-user record to echo back, the block itself lives
    // in `lifecycle_blocks`), and that is exactly the case where every open
    // panel most needs to re-read.
    notifyConsoleStateChanged();
    return rec;
  } catch {
    return null;
  }
}

/** Stop or start every agent in a business unit, in one idempotent toggle.
 * Same contract and app-wide broadcast as {@link blockUser}. */
export async function blockUnit(team: string, blocked: boolean): Promise<UnitRecord | null> {
  if (!hasBackend()) return null;
  try {
    const rec = await invokeBackend<UnitRecord | null>("unit_block", { team, blocked });
    // Same as `blockUser`: success is the signal, not a non-null echo.
    notifyConsoleStateChanged();
    return rec;
  } catch {
    return null;
  }
}
