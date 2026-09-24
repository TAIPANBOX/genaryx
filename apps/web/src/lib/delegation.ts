import { hasBackend, requiredRoleFromCommandError } from "./transport";
import { invokeWithCeremony } from "./webauthn";
import type { DelegationError, RevokeOutcome } from "../delegationTypes";

/** Thrown when there is no backend to talk to (a plain `vite build`/browser
 * preview) - mirrors `lib/money.ts`'s identical guard. There is no mock
 * vouchryx either, so this surfaces the same "no environment" shape a real
 * no-descriptor box would rather than inventing a fake outcome. */
const NO_ENVIRONMENT_ERROR: DelegationError = { kind: "not_configured" };

/** Normalize whatever `invokeBackend()`/`invokeWithCeremony()` rejected with
 * into a `DelegationError`. `genaryx-web` passes a command's `Err` value
 * through as the structured object it was serialized from, so this is
 * normally already a `DelegationError` in disguise; the fallback only
 * matters for a transport-level failure `delegation::commands` itself never
 * produces. */
function toDelegationError(err: unknown): DelegationError {
  const role = requiredRoleFromCommandError(err);
  if (role) return { kind: "role_required", role };
  if (err && typeof err === "object" && "kind" in err) {
    return err as DelegationError;
  }
  return { kind: "unreachable", detail: err instanceof Error ? err.message : String(err) };
}

/** Same contract as `lib/money.ts`'s `callWithCeremony`: dispatches through
 * the per-action WebAuthn ceremony (docs/CONSOLE-IDP.md B3/2,
 * `lib/webauthn.ts`'s `invokeWithCeremony`), so this module's one caller
 * inherits the hardware confirmation with no change of its own. */
async function callWithCeremony<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!hasBackend()) throw NO_ENVIRONMENT_ERROR;
  try {
    return await invokeWithCeremony<T>(command, args);
  } catch (err) {
    throw toDelegationError(err);
  }
}

/**
 * Revoke a delegation: the console's own cut-off switch for a compromised
 * agent's (or user's) authority (`delegation_revoke`, admin-only,
 * WebAuthn-ceremony-gated). Exactly one of `subject`/`jti` reaches the
 * backend; this module's one caller (`RevokeDelegationButton`,
 * `lib/lifecycle.tsx`) always has an agent id in hand, so it only ever
 * offers `subject`.
 *
 * Deliberately never invented as a local "reflected" state the way
 * `agent_block` projects a freeze everywhere: revocation is vouchryx's own
 * durable fact, not this console's, and there is no local lifecycle store
 * to update on success. The caller re-reads whatever it shows next time it
 * asks, the same honesty `money::commands::money_kill_run`'s own callers
 * already rely on for the Cloud's state.
 */
export const revokeDelegation = (subject: string, reason: string): Promise<RevokeOutcome> =>
  callWithCeremony<RevokeOutcome>("delegation_revoke", { subject, reason });
