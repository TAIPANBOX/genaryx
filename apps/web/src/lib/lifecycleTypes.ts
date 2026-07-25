/**
 * The one shared vocabulary for an entity's operator-driven lifecycle state,
 * used app-wide so an agent/unit/user reads the same way on every panel it
 * appears on (Overview spend-by-agent, Money runs, the Graph, the detail
 * cards, the watch dock).
 *
 * Deliberately dependency-free (no React, no wire modules) so the wire DTOs
 * that carry a lifecycle field - `Run` (`moneyTypes.ts`), `AgentRecord`,
 * `EntityAgent`, `PositionedNode` - can all name this type without any of them
 * taking on a UI dependency. The presentational `StateBadge` and the action
 * buttons that use it live in `lib/lifecycle.tsx`; the refetch signal lives in
 * `lib/consoleState.ts`.
 *
 * MOCK-ONLY on the wire: only the preview world (`lib/mockPreview.ts`)
 * populates the optional `lifecycle`/`stopped` fields on its reads. A real box
 * omits them (they stay `undefined`), and each surface falls back to the
 * plain `blocked`/`closed`/`killed` booleans those DTOs already carried, so an
 * as-yet-unimplemented backend handler stays a visible, honest no-op there.
 */

/**
 * - `live`: running normally.
 * - `stopped`: its business unit or its owning user was stopped (which stops
 *   every agent under them at once).
 * - `frozen`: this agent was individually frozen.
 * - `killed`: its live run was killed (a sticky, one-way operator action), or
 *   it is the fleet's closed-for-cause runaway.
 */
export type EntityLifecycleState = "live" | "stopped" | "frozen" | "killed";

/** The label + tone every state badge renders with, so LIVE/STOPPED/FROZEN/
 * KILLED read identically everywhere. Tones are the same theme variables the
 * rest of the console already uses for these severities (green live, amber
 * halted, iris frozen, critical killed). */
export const LIFECYCLE_BADGE: Record<EntityLifecycleState, { label: string; tone: string }> = {
  live: { label: "LIVE", tone: "var(--mint)" },
  stopped: { label: "STOPPED", tone: "var(--amber)" },
  frozen: { label: "FROZEN", tone: "var(--iris)" },
  killed: { label: "KILLED", tone: "var(--sev-critical)" },
};

/** The `.d-pill` status-cell variant a lifecycle state maps to, for the two
 * dashboard tables (`RunsBoard.tsx`, and the watch dock's own run status)
 * that render statuses as pills rather than badges. `killed` reuses the
 * existing `dead` pill; `stopped`/`frozen` get their own (added to index.css).
 * `live` returns `null`: a live run's pill is decided by its utilisation
 * (live/near/over), not by this. */
export function lifecyclePillClass(state: EntityLifecycleState): string | null {
  switch (state) {
    case "killed":
      return "dead";
    case "stopped":
      return "stopped";
    case "frozen":
      return "frozen";
    case "live":
      return null;
  }
}
