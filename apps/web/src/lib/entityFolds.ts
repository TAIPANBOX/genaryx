import { userHandle } from "./agentRecord";
import { agentTeam, spendByAgent } from "./dashData";
import { unitForTeam } from "./views";
import type { IdryxIdentity } from "../identityTypes";
import type { Run } from "../moneyTypes";

/**
 * Rolling `money_runs` up one level: by business unit, and by the human who
 * owns the agent.
 *
 * These two folds lived inside `WatchDock.tsx` and were private to it, which
 * is why the console could show a unit's or an owner's spend only for entities
 * an operator had pinned by hand. They are the only unit-spend and owner-spend
 * sources that answer on a REAL box (`unit_record`/`user_record` are
 * preview-only, see `entityRecords.ts`), so the Statistics view needs the same
 * two. Moved here rather than copied: two implementations of one fold drift,
 * and they drift silently, because both keep returning a plausible number.
 *
 * Nothing about the maths changed in the move.
 */

/** One unit's real, runs-derived aggregate. */
export interface UnitSpendAgg {
  /** This window's total spend across every agent whose id parses to this
   * team, straight off `Run.spent_usd` - the same number `spendByAgent`
   * already totals per agent, just grouped one level up. */
  spentUsd: number;
  /** Count of DISTINCT agents (not runs) seen for this team in the current
   * fetch - what the dock shows as "N agents". */
  agentCount: number;
  /** Always `null` today, and deliberately so: summing per-run ceilings is not
   * a unit monthly cap (it produced absurd "33000%" bars), and no real
   * unit-cap field or command exists client-side. The field stays on the shape
   * so a caller reads an explicit "no cap known" rather than having to know
   * the concept is missing. */
  budgetUsd: number | null;
}

/** One user's real, runs-derived aggregate - the identity-joined mirror of
 * [`UnitSpendAgg`]. No budget field: same as units, there is no per-user
 * budget or ceiling anywhere in this data model, mock or real. */
export interface UserSpendAgg {
  spentUsd: number;
  agentCount: number;
}

/** Groups `fetchRuns()`'s result by business unit (`agentTeam()` of each
 * run's `agent_id`) - the one unit-spend source that also answers on a real
 * box, since it rides `money_runs` rather than the mock-only `unit_record`.
 * Keyed by business UNIT (via `unitForTeam`), not raw team, so a pinned unit
 * id like "financial-crime" matches and multi-team units (fraud + kyc-aml)
 * aggregate into one row instead of never resolving. A team absent from
 * `runs` entirely is simply absent from the returned map, not a zero entry. */
export function unitSpendFromRuns(runs: Run[]): Map<string, UnitSpendAgg> {
  const byTeam = new Map<
    string,
    { spentUsd: number; agents: Set<string>; budgetUsd: number; hasBudget: boolean }
  >();
  for (const agent of spendByAgent(runs)) {
    const unit = unitForTeam(agent.team);
    const entry = byTeam.get(unit) ?? {
      spentUsd: 0,
      agents: new Set<string>(),
      budgetUsd: 0,
      hasBudget: false,
    };
    entry.spentUsd += agent.spent;
    entry.agents.add(agent.agent);
    byTeam.set(unit, entry);
  }
  for (const r of runs) {
    if (r.killed || r.budget_usd === null || r.budget_usd <= 0) continue;
    const entry = byTeam.get(unitForTeam(agentTeam(r.agent_id)));
    if (!entry) continue; // agentTeam() disagreeing with spendByAgent() above never happens (same parse), but never trust that silently.
    entry.budgetUsd += r.budget_usd;
    entry.hasBudget = true;
  }
  const out = new Map<string, UnitSpendAgg>();
  for (const [team, entry] of byTeam) {
    // budgetUsd stays null on purpose: see the field's own doc above.
    void entry.hasBudget;
    out.set(team, { spentUsd: entry.spentUsd, agentCount: entry.agents.size, budgetUsd: null });
  }
  return out;
}

/** Groups `fetchRuns()`'s result by OWNER, joined through
 * `fetchIdentities()`'s own `owner` field rather than anything on `Run`
 * itself - `Run` (`moneyTypes.ts`) carries no owner/on_behalf_of field at
 * all, unlike `agent_id` (`agentTeam`) which [`unitSpendFromRuns`] above
 * groups by directly. `identity_list_identities`, like `money_runs`, has a
 * REAL `crates/web/src/dispatch.rs` handler (unlike the mock-only
 * `user_record`/`unit_record`), so this resolves genuine spend and distinct
 * agent count on a real box too, not just the preview. Mock owners come
 * back `user://<org>/<handle>` (`userId()`, `lib/mockPreview.ts`); a real
 * idryx owner may already be a bare handle - `userHandle()`
 * (`lib/agentRecord.ts`, the SAME parse `AgentDetailCard.tsx`'s own "owner"
 * field already uses) takes the last path segment either way, so both
 * shapes land on the exact key `UserCard`/`WatchToggleButton` use. An
 * identity with no owner at all (`""`) contributes no entry, never a fake
 * "" user; an agent whose id is simply absent from the current identities
 * fetch is likewise excluded, not zero-charged to a guessed owner.
 *
 * That exclusion is why [`ownerByAgent`] is exported separately: a caller
 * listing owners needs to know how much it could NOT attribute, and a fold
 * that silently drops those agents cannot tell it. */
export function userSpendFromRuns(
  runs: Run[],
  identities: IdryxIdentity[],
): Map<string, UserSpendAgg> {
  const owners = ownerByAgent(identities);
  const byOwner = new Map<string, { spentUsd: number; agents: Set<string> }>();
  for (const agent of spendByAgent(runs)) {
    const owner = owners.get(agent.agent);
    if (!owner) continue;
    const entry = byOwner.get(owner) ?? { spentUsd: 0, agents: new Set<string>() };
    entry.spentUsd += agent.spent;
    entry.agents.add(agent.agent);
    byOwner.set(owner, entry);
  }
  const out = new Map<string, UserSpendAgg>();
  for (const [owner, entry] of byOwner) {
    out.set(owner, { spentUsd: entry.spentUsd, agentCount: entry.agents.size });
  }
  return out;
}

/** Agent id -> owner handle, from idryx. The join [`userSpendFromRuns`] uses,
 * exposed on its own so a caller can also count the agents this map has no
 * answer for. An identity with an empty `owner` is omitted: idryx itself
 * treats an unowned identity as a finding (its `OrphanedNHI` detector), so the
 * honest render is a visible "no owner in idryx" bucket, never a blank key
 * that reads as a person. */
export function ownerByAgent(identities: IdryxIdentity[]): Map<string, string> {
  const out = new Map<string, string>();
  for (const identity of identities) {
    if (identity.owner) out.set(identity.id, userHandle(identity.owner));
  }
  return out;
}
