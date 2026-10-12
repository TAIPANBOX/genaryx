import { hasBackend, invokeBackend, requiredRoleFromCommandError } from "./transport";
import { invokeWithCeremony } from "./webauthn";

/**
 * D8: list the gateway's retained reservations and close one with evidence.
 * Wire types mirror `crates/api/src/money/retained.rs` field for field.
 *
 * Closing goes through `invokeWithCeremony`, so the passkey assertion runs
 * before the command reaches the server, and the server mints the gateway's
 * closure token only after its own gate has checked that assertion. Nothing
 * here, and nothing in the browser, ever sees the secret or the token.
 */

/** One row of `money_list_retained` (the gateway's `GET /v1/retained`). */
export interface RetainedReservation {
  reservation_id: number;
  revision: string;
  run_id: string;
  chain: string[];
  amount_microusd: number;
  unit: string | null;
  model: string;
  retained_at_millis: number;
}

export type EvidenceKind = "observed" | "no_charge";

/** `money_close_retained`'s arguments. The operation id is not here: the
 * console generates it. */
export interface CloseRetainedArgs {
  reservation_id: number;
  expected_revision: string;
  evidence_kind: EvidenceKind;
  observed_microusd: number | null;
  evidence_ref: string;
}

/** Mirrors `RetainedError` (`#[serde(tag = "kind", rename_all = "snake_case")]`),
 * plus the web shell's role gate, which refuses before the command runs. */
export type RetainedError =
  | { kind: "not_configured" }
  | { kind: "bootstrapping" }
  | { kind: "no_environment" }
  | { kind: "gateway_unreachable"; reason: string }
  | { kind: "invalid_args"; reason: string }
  | { kind: "closure_off" }
  | { kind: "refused"; status: number; error: string; message: string }
  | { kind: "unconfirmed"; status: number; body: unknown }
  | { kind: "unreachable"; detail: string }
  | { kind: "role_required"; role: "viewer" | "approver" | "admin" };

/** Mirrors `CloseOutcome`. */
export interface CloseOutcome {
  http_status: number;
  verify_result: string;
  operation_id: string;
  replayed: boolean;
  settled_microusd: number | null;
  gateway_response: unknown;
  bus_recorded: boolean;
  bus_error: string | null;
}

function toRetainedError(err: unknown): RetainedError {
  const role = requiredRoleFromCommandError(err);
  if (role) return { kind: "role_required", role };
  if (err && typeof err === "object" && "kind" in err) return err as RetainedError;
  return { kind: "unreachable", detail: err instanceof Error ? err.message : String(err) };
}

export async function listRetained(): Promise<RetainedReservation[]> {
  if (!hasBackend()) throw { kind: "no_environment" } satisfies RetainedError;
  try {
    return await invokeBackend<RetainedReservation[]>("money_list_retained");
  } catch (err) {
    throw toRetainedError(err);
  }
}

export async function closeRetained(args: CloseRetainedArgs): Promise<CloseOutcome> {
  if (!hasBackend()) throw { kind: "no_environment" } satisfies RetainedError;
  let out: unknown;
  try {
    out = await invokeWithCeremony<unknown>("money_close_retained", { ...args });
  } catch (err) {
    throw toRetainedError(err);
  }
  // Only the server's own confirmed outcome counts as closed. Anything else
  // (the demo transport's `null`, a proxy page) is unconfirmed, never success.
  if (!isCloseOutcome(out)) throw { kind: "unconfirmed", status: 0, body: out } satisfies RetainedError;
  return out;
}

export function isCloseOutcome(v: unknown): v is CloseOutcome {
  if (typeof v !== "object" || v === null) return false;
  const o = v as Record<string, unknown>;
  return typeof o.verify_result === "string" && o.verify_result.startsWith("closed") && typeof o.operation_id === "string";
}

/** What the operator typed into the Close form. */
export interface CloseForm {
  evidenceKind: EvidenceKind;
  observedMicrousd: string;
  evidenceRef: string;
}

/**
 * Turn the form into the command's arguments, or say what is wrong with it.
 * The same rules the server and the gateway apply, so a form that cannot
 * succeed never starts a passkey ceremony. Pure, so it is tested directly.
 */
export function buildCloseArgs(
  row: Pick<RetainedReservation, "reservation_id" | "revision">,
  form: CloseForm,
): { ok: true; args: CloseRetainedArgs } | { ok: false; reason: string } {
  const ref = form.evidenceRef.trim();
  if (ref === "") return { ok: false, reason: "Name the evidence: an invoice line or the provider's usage record." };
  if (new TextEncoder().encode(ref).length > 512) return { ok: false, reason: "The evidence reference is over 512 bytes." };
  if (/\p{Cc}/u.test(ref)) return { ok: false, reason: "The evidence reference cannot contain control characters." };

  let observed: number | null = null;
  if (form.evidenceKind === "observed") {
    const raw = form.observedMicrousd.trim();
    if (!/^\d+$/.test(raw)) return { ok: false, reason: "An observed closure needs the cost the evidence shows, in whole micro-USD." };
    observed = Number(raw);
    if (!Number.isSafeInteger(observed)) return { ok: false, reason: "That micro-USD amount is too large." };
  }
  return {
    ok: true,
    args: {
      reservation_id: row.reservation_id,
      expected_revision: row.revision,
      evidence_kind: form.evidenceKind,
      observed_microusd: observed,
      evidence_ref: ref,
    },
  };
}

export function describeRetainedError(err: RetainedError): string {
  switch (err.kind) {
    case "not_configured":
      return "This console holds no closure secret (GENARYX_CLOSURE_SECRET_FILE is unset), so it cannot close a reservation.";
    case "bootstrapping":
      return "Still connecting to the gateway.";
    case "no_environment":
      return "No gateway configured for this environment.";
    case "gateway_unreachable":
      return `Could not reach the gateway: ${err.reason}`;
    case "invalid_args":
      return err.reason;
    case "closure_off":
      return "The gateway's closure route is off (TOKENFUSE_CLOSURE_SECRET is not set there).";
    case "refused":
      return `The gateway refused (${err.status} ${err.error})${err.message ? `: ${err.message}` : ""}`;
    case "unconfirmed":
      return err.status === 0
        ? "No confirmed closure came back; it is not shown as closed."
        : `The gateway answered ${err.status} without confirming the closure; it is not shown as closed.`;
    case "unreachable":
      return `No answer from the gateway: ${err.detail}`;
    case "role_required":
      return `Closing a reservation needs the ${err.role} role.`;
  }
}
