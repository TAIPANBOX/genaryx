import { cssVar } from "./cssVars";
import { ConfirmButton } from "../components/ConfirmButton";
import { LIFECYCLE_BADGE, type EntityLifecycleState } from "./lifecycleTypes";
import type { AgentRecord } from "./agentRecord";
import type { EntityAgent, UnitRecord, UserRecord } from "./entityRecords";
import type { Run } from "../moneyTypes";

/**
 * The shared, app-wide "lifecycle actions" surface: the one place the four call
 * sites (the watch dock, the agent card, the unit card, the user card) get the
 * SAME state badge, the SAME state derivation, and the SAME state-driven toggle
 * buttons, so the model reads and behaves identically everywhere instead of
 * being re-implemented per panel.
 *
 * Product model (Yurii): a state-driven toggle that shows the OPPOSITE of the
 * current state (a running thing offers Stop/Freeze; a halted one offers
 * Start/Unfreeze), plus a small LIVE/STOPPED/FROZEN/KILLED badge on the entity.
 * Confirmation reuses the existing `ConfirmButton`: its plain inline confirm
 * for Freeze/Unfreeze and Stop/Start, its `breakGlass` path for the agent Kill.
 * No new confirm UI, and never a password.
 *
 * The buttons only reflect what the command's RESPONSE (or the reflected read)
 * says: in the mock world every command mutates the one lifecycle store and
 * the reads reflect it, so the demo works end to end; on a real box a command
 * with no handler yet returns null / an error and the button stays put - a
 * visible, honest no-op, never a faked local success.
 */

/** The small state pill shown on an entity. Reuses the app's `.badge` (its
 * `--tone` var drives colour), so it matches every other badge in the console
 * exactly. */
export function StateBadge({ state, title }: { state: EntityLifecycleState; title?: string }) {
  const { label, tone } = LIFECYCLE_BADGE[state];
  return (
    <span className="badge" style={cssVar("tone", tone)} title={title}>
      {label}
    </span>
  );
}

// ---- State derivation (one definition, every surface) ----------------------
// Each helper prefers the mock's precise `lifecycle`/`stopped` enrichment and
// falls back to the plain booleans a real box still sends, so a surface reads
// honestly on both. Precedence for an agent, when only booleans are known:
// killed (closed) > frozen (blocked) > live.

/** An agent's effective state from its detail record. */
export function agentStateFromRecord(rec: AgentRecord | null | undefined): EntityLifecycleState {
  if (!rec) return "live";
  if (rec.lifecycle) return rec.lifecycle;
  if (rec.closed) return "killed";
  if (rec.blocked) return "frozen";
  return "live";
}

/** One agent row's effective state, inside a unit/user record. */
export function entityAgentState(a: EntityAgent): EntityLifecycleState {
  if (a.lifecycle) return a.lifecycle;
  if (a.closed) return "killed";
  if (a.blocked) return "frozen";
  return "live";
}

/** Whether a unit is currently stopped: the mock's `stopped` flag when known,
 * else derived from every agent in the record being blocked (so a real box,
 * which omits `stopped`, still reads a fully-blocked unit as stopped rather
 * than never). A record with no agents is not "stopped". */
export function isUnitStopped(rec: UnitRecord | null | undefined): boolean {
  if (!rec) return false;
  if (typeof rec.stopped === "boolean") return rec.stopped;
  return rec.agents.length > 0 && rec.agents.every((a) => a.blocked);
}

/** Whether a user is currently stopped - same rule as {@link isUnitStopped}. */
export function isUserStopped(rec: UserRecord | null | undefined): boolean {
  if (!rec) return false;
  if (typeof rec.stopped === "boolean") return rec.stopped;
  return rec.agents.length > 0 && rec.agents.every((a) => a.blocked);
}

/** A run's blocked lifecycle, or `null` when it is live (in which case the
 * caller decides its own live/near/over pill from utilisation). Prefers the
 * mock's precise `lifecycle`; on a real box, a `killed` run reads `killed`. */
export function runBlockedState(r: Run): EntityLifecycleState | null {
  if (r.lifecycle && r.lifecycle !== "live") return r.lifecycle;
  if (r.killed) return "killed";
  return null;
}

/** Per-agent blocked state derived from a runs list, for surfaces that only
 * hold runs (the Overview "spend by agent" bars). An agent is keyed to the
 * worst (non-live) lifecycle any of its runs reports; agents with only live
 * runs are absent from the map. */
export function agentBlockedStateFromRuns(runs: Run[]): Map<string, EntityLifecycleState> {
  const out = new Map<string, EntityLifecycleState>();
  for (const r of runs) {
    const state = runBlockedState(r);
    if (state && !out.has(r.agent_id)) out.set(r.agent_id, state);
  }
  return out;
}

// ---- Shared state-driven action buttons ------------------------------------

/** Freeze <-> Unfreeze for one agent. Plain inline confirm (no break-glass:
 * freezing carries no server-side audit ceremony). State-driven: shows the
 * opposite of the current state. */
export function FreezeToggleButton({
  frozen,
  onToggle,
  disabled,
}: {
  frozen: boolean;
  onToggle: () => Promise<void>;
  disabled?: boolean;
}) {
  return (
    <ConfirmButton
      label={frozen ? "Unfreeze" : "Freeze"}
      confirmLabel={frozen ? "Confirm unfreeze" : "Confirm freeze"}
      tone={frozen ? "var(--mint)" : "var(--iris)"}
      disabled={disabled}
      onConfirm={() => onToggle()}
    />
  );
}

/** Stop <-> Start for a unit or a user (stops/starts every agent under them).
 * Plain inline confirm, state-driven. */
export function StopStartButton({
  stopped,
  onToggle,
  disabled,
}: {
  stopped: boolean;
  onToggle: () => Promise<void>;
  disabled?: boolean;
}) {
  return (
    <ConfirmButton
      label={stopped ? "Start" : "Stop"}
      confirmLabel={stopped ? "Confirm start" : "Confirm stop"}
      tone={stopped ? "var(--mint)" : "var(--sev-high)"}
      disabled={disabled}
      onConfirm={() => onToggle()}
    />
  );
}

/** Kill an agent's live run: the break-glass ceremony (required reason) +
 * WebAuthn step-up `RunsBoard.tsx`'s Money-panel Kill already uses, targeting a
 * caller-chosen live run. Disabled (never a silent no-op) when there is no live
 * run to kill. `formatSpent` keeps this module free of a `format.ts` import
 * cycle - callers pass their own already-formatted spend string for the modal
 * detail. */
export function KillRunButton({
  run,
  onKill,
  detail,
}: {
  run: Run | null;
  onKill: (runId: string, reason: string) => Promise<void>;
  /** Extra context for the break-glass modal (e.g. "run x · spent $1.20"). */
  detail?: string;
}) {
  if (!run) {
    return (
      <button
        type="button"
        className="icon-btn"
        style={{ width: "auto", padding: "0 8px", fontSize: 10.5, opacity: 0.45, cursor: "not-allowed" }}
        disabled
        title="No live run to kill"
      >
        Kill
      </button>
    );
  }
  return (
    <ConfirmButton
      label="Kill"
      confirmLabel="Confirm kill"
      tone="var(--sev-critical)"
      breakGlass
      breakGlassDetail={detail ?? `run ${run.run_id}`}
      onConfirm={(reason) => onKill(run.run_id, reason)}
    />
  );
}
