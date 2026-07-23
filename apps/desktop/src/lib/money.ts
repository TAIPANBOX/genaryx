import { hasBackend, invokeBackend, requiredRoleFromCommandError, type ConsoleRole } from "./transport";
import type {
  Incident,
  MoneyError,
  MoneyStatus,
  MutationOutcome,
  Overview,
  Run,
  Savings,
} from "../moneyTypes";

/** Thrown by every fetcher/mutator below when there is no Tauri runtime to
 * talk to (a plain `vite build`/browser preview) - mirrors
 * `lib/recentEvents.ts`'s `hasBackend()` guard, except the Money panel has no
 * sensible mock data to fall back to (there is no mock Cloud), so it
 * surfaces the same "no environment" state a real no-descriptor box would
 * show rather than inventing fake numbers. */
const NO_ENVIRONMENT_ERROR: MoneyError = { kind: "no_environment" };

/** Normalize whatever `invoke()` rejected with into a `MoneyError`. Tauri
 * passes a command's `Err` value through as the structured object it was
 * serialized from, so this is normally already a `MoneyError` in disguise;
 * the fallback branch only matters for a transport-level IPC failure (e.g.
 * "command not found"), which is not a shape `money::commands::MoneyError`
 * itself ever produces. */
function toMoneyError(err: unknown): MoneyError {
  const role = requiredRoleFromCommandError(err);
  if (role) return { kind: "role_required", role };
  if (err && typeof err === "object" && "kind" in err) {
    return err as MoneyError;
  }
  return { kind: "cloud", status: null, message: err instanceof Error ? err.message : String(err) };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!hasBackend()) throw NO_ENVIRONMENT_ERROR;
  try {
    return await invokeBackend<T>(command, args);
  } catch (err) {
    throw toMoneyError(err);
  }
}

/** Whole-panel connection state. Never throws: outside Tauri (or on any IPC
 * failure) it resolves to a renderable status instead, matching every other
 * money_* fetcher's fail-closed contract but without a `MoneyError` to
 * unwrap since this is the command the UI uses to decide whether to call
 * the others at all. */
export async function fetchMoneyStatus(): Promise<MoneyStatus> {
  if (!hasBackend()) return { state: "no_environment" };
  try {
    return await invokeBackend<MoneyStatus>("money_status");
  } catch (err) {
    return {
      state: "pairing_failed",
      source: { source: "env_fallback" },
      cloud_url: "",
      reason: err instanceof Error ? err.message : String(err),
    };
  }
}

export const fetchOverview = (): Promise<Overview> => call<Overview>("money_overview");
export const fetchRuns = (): Promise<Run[]> => call<Run[]>("money_runs");
export const fetchIncidents = (): Promise<Incident[]> => call<Incident[]>("money_incidents");
export const fetchSavings = (): Promise<Savings> => call<Savings>("money_savings");

// Argument keys are snake_case on purpose: the Rust side pins
// `#[tauri::command(rename_all = "snake_case")]` on every mutation so the
// whole IPC surface (args AND return values) stays one convention, matching
// `UiEvent`'s existing snake_case wire shape rather than Tauri's camelCase
// default.

// `killRun`/`setBudget` are break-glass overrides (Phase-2 wave 3B): both take
// a mandatory `reason`, threaded straight into the Rust side's
// `require_break_glass_reason` guard (`money::commands`) and, once past that,
// into the journaled `CommandRecord`'s `params`. `ConfirmButton`'s break-glass
// ceremony is what actually collects `reason` from the operator - it never
// reaches this module empty in normal use, but the Rust side still refuses a
// blank one rather than trusting the frontend alone (fail-closed, 06 §0.5).

export const killRun = (runId: string, reason: string): Promise<MutationOutcome> =>
  call<MutationOutcome>("money_kill_run", { run_id: runId, reason });

export const setBudget = (runId: string, budgetUsd: number, reason: string): Promise<MutationOutcome> =>
  call<MutationOutcome>("money_set_budget", { run_id: runId, budget_usd: budgetUsd, reason });

/** NOT break-glass: acknowledging an incident overrides no governance
 * decision, so this carries no reason and the Rust side journals it as
 * `decision: "allow"` rather than `"break_glass"`. */
export const ackIncident = (id: string): Promise<MutationOutcome> =>
  call<MutationOutcome>("money_ack_incident", { id });

/** Human-readable text for any `MoneyError` - used for the plain error
 * banner. `plan_required` is deliberately NOT routed through this: the UI
 * renders that one as an upsell tile instead of an error message (spec).
 *
 * `currentRole` is the signed-in operator's OWN role (from `useSession()`),
 * threaded in by callers that know it (`MoneyView.tsx`) so a `role_required`
 * refusal can say who you are, not just what was needed - optional, so every
 * OTHER existing call site (`OverviewView.tsx`, `Agent360.tsx`, ...) keeps
 * compiling unchanged and still gets an honest, just less complete, message. */
export function describeMoneyError(err: MoneyError, currentRole?: ConsoleRole | null): string {
  switch (err.kind) {
    case "bootstrapping":
      return "Still connecting to the Cloud environment.";
    case "no_environment":
      return "No TokenFuse Cloud environment found.";
    case "pairing_failed":
      return `Pairing failed: ${err.reason}`;
    case "plan_required":
      return `Upgrade required: ${err.feature} is not on the ${err.org} plan.`;
    case "break_glass_missing_reason":
      return "Break-glass override requires a non-empty reason.";
    case "cloud":
      return err.status !== null ? `Cloud error ${err.status}: ${err.message}` : err.message;
    case "role_required": {
      const need = `This action needs the ${err.role} role.`;
      return currentRole ? `${need} You are signed in as ${currentRole}.` : need;
    }
  }
}
