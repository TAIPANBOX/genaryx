import { hasBackend, invokeBackend } from "./transport";
import type { Approval, Decision, DecideOutcome, PolicyError, PolicyRecord, PolicyStatus } from "../policyTypes";

/** Thrown by every fetcher/mutator below when there is no Tauri runtime to
 * talk to (a plain `vite build`/browser preview) - mirrors `lib/money.ts`'s
 * identical `NO_ENVIRONMENT_ERROR` guard: there is no mock policy plane to
 * fall back to, so this surfaces the same "no environment" state a real
 * no-descriptor box would show rather than inventing fake data. */
const NO_ENVIRONMENT_ERROR: PolicyError = { kind: "no_environment" };

/** Normalize whatever `invoke()` rejected with into a `PolicyError`. Tauri
 * passes a command's `Err` value through as the structured object it was
 * serialized from, so this is normally already a `PolicyError` in disguise;
 * the fallback branch only matters for a transport-level IPC failure. */
function toPolicyError(err: unknown): PolicyError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as PolicyError;
  }
  return { kind: "wardryx", status: null, message: err instanceof Error ? err.message : String(err) };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!hasBackend()) throw NO_ENVIRONMENT_ERROR;
  try {
    return await invokeBackend<T>(command, args);
  } catch (err) {
    throw toPolicyError(err);
  }
}

/** Whole-panel connection state. Never throws: outside Tauri (or on any IPC
 * failure) it resolves to a renderable status instead - mirrors
 * `lib/money.ts`'s `fetchMoneyStatus` exactly. */
export async function fetchPolicyStatus(): Promise<PolicyStatus> {
  if (!hasBackend()) return { state: "no_environment" };
  try {
    return await invokeBackend<PolicyStatus>("policy_status");
  } catch (err) {
    return {
      state: "unreachable",
      source: { source: "env_fallback" },
      wardryx_url: "",
      reason: err instanceof Error ? err.message : String(err),
    };
  }
}

export const fetchApprovals = (): Promise<Approval[]> => call<Approval[]>("policy_list_approvals");
export const fetchPolicies = (): Promise<PolicyRecord[]> => call<PolicyRecord[]>("policy_list_policies");

// snake_case argument keys on purpose: the Rust side pins
// `#[tauri::command(rename_all = "snake_case")]`, matching `lib/money.ts`'s
// identical convention for its mutation commands.
export const decideApproval = (id: string, decision: Decision): Promise<DecideOutcome> =>
  call<DecideOutcome>("policy_decide_approval", { id, decision });

/** Human-readable text for any `PolicyError` - used for the plain error
 * banner (mirrors `lib/money.ts`'s `describeMoneyError`). */
export function describePolicyError(err: PolicyError): string {
  switch (err.kind) {
    case "bootstrapping":
      return "Still connecting to a Wardryx policy plane.";
    case "no_environment":
      return "No Wardryx policy plane found.";
    case "unreachable":
      return `Could not reach Wardryx: ${err.reason}`;
    case "wardryx":
      return err.status !== null ? `Wardryx error ${err.status}: ${err.message}` : err.message;
  }
}
