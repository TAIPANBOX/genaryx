import { hasBackend, invokeBackend } from "./transport";
import { maxLastSeenMillis, totalCalls } from "./credentials";
import type { AdmissionBaseline, AdmissionCheck, AdmissionError, AdmissionStatus } from "../admissionTypes";
import type { MockryxReport } from "../drillsTypes";
import { hasGaps } from "../drillsTypes";
import type { DrillsError } from "../drillsTypes";

/** Thrown by every fetcher below when there is no backend to talk to -
 * mirrors `lib/credentials.ts`'s identical `NO_ENVIRONMENT_ERROR` guard. */
const NO_ENVIRONMENT_ERROR: AdmissionError = { kind: "no_environment" };

function toAdmissionError(err: unknown): AdmissionError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as AdmissionError;
  }
  return {
    kind: "gateway",
    status: null,
    message: err instanceof Error ? err.message : String(err),
  };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!hasBackend()) throw NO_ENVIRONMENT_ERROR;
  try {
    return await invokeBackend<T>(command, args);
  } catch (err) {
    throw toAdmissionError(err);
  }
}

/** Whole-plane status. Never throws: outside a backend (or on any IPC
 * failure) it resolves to a renderable status instead - mirrors
 * `lib/credentials.ts`'s `fetchCredentialsStatus` exactly. */
export async function fetchAdmissionStatus(): Promise<AdmissionStatus> {
  if (!hasBackend()) {
    return {
      gateway: { state: "no_environment" },
      verdryx_bin: "",
      verdryx_bin_present: false,
      verdryx_db: null,
      drills_scenario_dir: null,
    };
  }
  try {
    return await invokeBackend<AdmissionStatus>("admission_status");
  } catch (err) {
    return {
      gateway: {
        state: "unreachable",
        source: { source: "taipan", name: "" },
        gateway_url: "",
        reason: err instanceof Error ? err.message : String(err),
      },
      verdryx_bin: "",
      verdryx_bin_present: false,
      verdryx_db: null,
      drills_scenario_dir: null,
    };
  }
}

/** `admission_check` - viewer-safe: is `keyId` known to the gateway, is it
 * bound, has it seen traffic, does `agentId` match the live identity map
 * anywhere. Never auto-triggered; the operator clicks "Run checks". */
export const runAdmissionCheck = (keyId: string, agentId: string): Promise<AdmissionCheck> =>
  call<AdmissionCheck>("admission_check", { key_id: keyId, agent_id: agentId });

/** `admission_baseline` - admin-only: runs a Verdryx eval THROUGH the
 * gateway under the newcomer's own key, then a baseline snapshot, then
 * reads the result back. Fires real provider spend; only on an explicit
 * confirm (see `AdmissionVerify.tsx`). `apiKey` is used only for this one
 * call, never persisted by this module or its caller. */
export const runAdmissionBaseline = (
  evalsetPath: string,
  model: string,
  agentId: string,
  apiKey: string,
): Promise<AdmissionBaseline> =>
  call<AdmissionBaseline>("admission_baseline", {
    evalset_path: evalsetPath,
    model,
    agent_id: agentId,
    api_key: apiKey,
  });

/** Human-readable text for any `AdmissionError` - mirrors every sibling
 * panel's `describe*Error`. */
export function describeAdmissionError(err: AdmissionError): string {
  switch (err.kind) {
    case "bootstrapping":
      return "Still connecting to the gateway.";
    case "no_environment":
      return "No gateway found in this environment.";
    case "unreachable":
      return `Could not reach the gateway: ${err.reason}`;
    case "gateway":
      return err.status !== null ? `Gateway error ${err.status}: ${err.message}` : err.message;
    case "verdryx_bin_missing":
      return `verdryx binary not found at ${err.path} - install it there (or symlink it) for the console to auto-discover it.`;
    case "verdryx_db_missing":
      return "No verdryx.db found - no descriptor entry and no ~/.taipan/verdryx.db.";
    case "verdryx":
      return err.message;
    case "unparseable_output":
      return `Could not read verdryx's ${err.context} from its output: ${err.stdout_excerpt}`;
    case "run_not_found":
      return `The eval run ${err.run_id} has no summary right after it was written - this should not happen.`;
  }
}

// ============================================================================
// Scoreboard assembly (pure - no backend calls, no Date.now() inside; the
// caller ticks a `nowMillis` the same way `lib/credentials.ts`'s own
// age-based derivations do)
// ============================================================================

/** Whether `check.key` has recorded ANY call, in either stats block - "first
 * traffic" for the scoreboard's own card. `false` when the key was not even
 * found (`check.key === null`). */
export function hasFirstTraffic(check: AdmissionCheck | null): boolean {
  return check?.key != null && totalCalls(check.key) > 0;
}

/** The later of `check.key`'s two `last_seen_millis` fields, or `null` when
 * there is no key or it never recorded one - thin pass-through to
 * `lib/credentials.ts`'s `maxLastSeenMillis` so the two planes can never
 * compute "last seen" differently for the same wire shape. */
export function lastTrafficMillis(check: AdmissionCheck | null): number | null {
  return check?.key != null ? maxLastSeenMillis(check.key) : null;
}

/**
 * Whether the "Enable strict" proposal block should show (docs/ADMISSION.md):
 * the key is bound, `agent_id` is in the map, first traffic has been seen,
 * and the LAST drill attempt ran without an infrastructure error (a report
 * came back at all - gaps inside that report are informative, not blocking:
 * a drill that finds every guardrail gapping is exactly the reason to fix
 * the map/policy BEFORE flipping strict, not a reason to hide the proposal).
 */
export function readyToProposeStrict(
  check: AdmissionCheck | null,
  drillReport: MockryxReport | null,
  drillError: DrillsError | null,
): boolean {
  return Boolean(
    check?.key?.bound &&
      check.in_map &&
      hasFirstTraffic(check) &&
      drillReport !== null &&
      drillError === null,
  );
}

/** One sentence explaining WHY the drill leg does not block the proposal on
 * its own - shown next to the proposal block whenever a report exists and
 * has gaps, so an operator is never confused about why "found gaps" did not
 * grey the button out. */
export function drillGapNote(drillReport: MockryxReport | null): string | null {
  if (drillReport === null) return null;
  return hasGaps(drillReport)
    ? "The last drill found gaps - informative, not blocking: review them, but they do not by themselves hide this proposal."
    : null;
}
