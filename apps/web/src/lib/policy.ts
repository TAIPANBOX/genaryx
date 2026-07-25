import { hasBackend, invokeBackend, requiredRoleFromCommandError, type ConsoleRole } from "./transport";
import { invokeWithCeremony } from "./webauthn";
import type {
  Approval,
  Decision,
  DecideOutcome,
  PolicyError,
  PolicyRecord,
  PolicyStatus,
  WardryxStatus,
} from "../policyTypes";

/** Thrown by every fetcher/mutator below when there is no backend to
 * talk to (a plain `vite build`/browser preview) - mirrors `lib/money.ts`'s
 * identical `NO_ENVIRONMENT_ERROR` guard: there is no mock policy plane to
 * fall back to, so this surfaces the same "no environment" state a real
 * no-descriptor box would show rather than inventing fake data. */
const NO_ENVIRONMENT_ERROR: PolicyError = { kind: "no_environment" };

/** Normalize whatever `invokeBackend()` rejected with into a `PolicyError`. genaryx-web
 * passes a command's `Err` value through as the structured object it was
 * serialized from, so this is normally already a `PolicyError` in disguise;
 * the fallback branch only matters for a transport-level failure. */
function toPolicyError(err: unknown): PolicyError {
  const role = requiredRoleFromCommandError(err);
  if (role) return { kind: "role_required", role };
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

/** Same contract as {@link call}, but for `policy_decide_approval`
 * (docs/CONSOLE-IDP.md B3/2): dispatches through the per-action WebAuthn
 * ceremony (`lib/webauthn.ts`'s `invokeWithCeremony`) instead of invoking
 * directly, so every caller of `decideApproval` inherits the hardware
 * confirmation with no change of their own. */
async function callWithCeremony<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!hasBackend()) throw NO_ENVIRONMENT_ERROR;
  try {
    return await invokeWithCeremony<T>(command, args);
  } catch (err) {
    throw toPolicyError(err);
  }
}

/** Whole-panel connection state. Never throws: with no backend (or on any
 * transport failure) it resolves to a renderable status instead - mirrors
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

/** What the policy plane is ACTUALLY enforcing, from Wardryx's `/v1/status`.
 *
 * Not a variant of {@link fetchPolicies}. That lists the STORE's
 * operator-managed policies, which is empty on every deployment whose rules
 * come from a `-policy` file - while all of those rules are enforced. Judging
 * enforcement by that list reports a guarded fleet as wide open. */
export const fetchEnforcementStatus = (): Promise<WardryxStatus> =>
  call<WardryxStatus>("policy_enforcement_status");

// snake_case argument keys on purpose, matching the Rust side's own
// snake_case field names, and `lib/money.ts`'s identical convention for its
// mutation commands.
export const decideApproval = (id: string, decision: Decision): Promise<DecideOutcome> =>
  callWithCeremony<DecideOutcome>("policy_decide_approval", { id, decision });

/** Human-readable text for any `PolicyError` - used for the plain error
 * banner (mirrors `lib/money.ts`'s `describeMoneyError`, including the
 * optional `currentRole` - the signed-in operator's own role, from
 * `useSession()` - that lets a `role_required` refusal say who you are, not
 * just what was needed). */
export function describePolicyError(err: PolicyError, currentRole?: ConsoleRole | null): string {
  switch (err.kind) {
    case "bootstrapping":
      return "Still connecting to a Wardryx policy plane.";
    case "no_environment":
      return "No Wardryx policy plane found.";
    case "unreachable":
      return `Could not reach Wardryx: ${err.reason}`;
    case "wardryx":
      return err.status !== null ? `Wardryx error ${err.status}: ${err.message}` : err.message;
    case "role_required": {
      const need = `This action needs the ${err.role} role.`;
      return currentRole ? `${need} You are signed in as ${currentRole}.` : need;
    }
  }
}
