import { hasBackend, requiredRoleFromCommandError } from "./transport";
import { invokeWithCeremony } from "./webauthn";
import type {
  DelegationError,
  LiftOutcome,
  LiftResult,
  RevokeOutcome,
  StopError,
  StopOutcome,
  StopResult,
} from "../delegationTypes";

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

/** Whatever a stop or a lift rejected with, as a `StopError`. The command's
 * own refusals (`not_stopped`, `not_lifted`, an argument refusal) arrive as
 * the structured value `genaryx-web` serialized; the role gate's 403 and a
 * transport failure happen before the command, and are named here. */
function toStopError(err: unknown): StopError {
  const role = requiredRoleFromCommandError(err);
  if (role) return { kind: "role_required", role };
  if (err && typeof err === "object" && "kind" in err) return err as StopError;
  return { kind: "failed", detail: err instanceof Error ? err.message : String(err) };
}

/**
 * Stop an agent everywhere (`agent_stop`, admin-only, WebAuthn-ceremony-gated):
 * a ban at vouchryx, its open runs killed, every step in the console's chain.
 * Never throws: the caller always gets the outcome, done or not, with every
 * step, so a failed step is shown rather than lost in a rejected promise.
 */
export async function stopAgent(subject: string, reason: string): Promise<StopResult> {
  if (!hasBackend()) return { ok: false, error: { kind: "failed", detail: "no console backend in this build" } };
  try {
    return { ok: true, outcome: await invokeWithCeremony<StopOutcome>("agent_stop", { subject, reason }) };
  } catch (err) {
    return { ok: false, error: toStopError(err) };
  }
}

/** Lift the ban a stop set (`agent_lift`), after the same ceremony. Never
 * throws, for the same reason as {@link stopAgent}. */
export async function liftAgent(subject: string, reason: string): Promise<LiftResult> {
  if (!hasBackend()) return { ok: false, error: { kind: "failed", detail: "no console backend in this build" } };
  try {
    return { ok: true, outcome: await invokeWithCeremony<LiftOutcome>("agent_lift", { subject, reason }) };
  } catch (err) {
    return { ok: false, error: toStopError(err) };
  }
}
