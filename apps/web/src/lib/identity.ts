import { hasBackend, invokeBackend } from "./transport";
import type {
  IdentityError,
  IdentityStatus,
  IdryxAlert,
  IdryxIdentity,
  IdryxRecommendation,
} from "../identityTypes";

/** Thrown by every fetcher below when there is no backend to talk to
 * (a plain `vite build`/browser preview) - mirrors `lib/policy.ts`'s
 * identical `NO_ENVIRONMENT_ERROR` guard: there is no mock identity plane
 * to fall back to, so this surfaces the same "no environment" state a real
 * no-descriptor box would show rather than inventing fake data. */
const NO_ENVIRONMENT_ERROR: IdentityError = { kind: "no_environment" };

/** Normalize whatever `invokeBackend()` rejected with into an `IdentityError`.
 * genaryx-web passes a command's `Err` value through as the structured object it
 * was serialized from, so this is normally already an `IdentityError` in
 * disguise; the fallback branch only matters for a transport-level
 * failure. */
function toIdentityError(err: unknown): IdentityError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as IdentityError;
  }
  return { kind: "idryx", status: null, message: err instanceof Error ? err.message : String(err) };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!hasBackend()) throw NO_ENVIRONMENT_ERROR;
  try {
    return await invokeBackend<T>(command, args);
  } catch (err) {
    throw toIdentityError(err);
  }
}

/** Whole-panel connection state. Never throws: with no backend (or on any
 * transport failure) it resolves to a renderable status instead - mirrors
 * `lib/policy.ts`'s `fetchPolicyStatus` exactly. */
export async function fetchIdentityStatus(): Promise<IdentityStatus> {
  if (!hasBackend()) return { state: "no_environment" };
  try {
    return await invokeBackend<IdentityStatus>("identity_status");
  } catch (err) {
    return {
      state: "unreachable",
      source: { source: "taipan", name: "" },
      idryx_url: "",
      reason: err instanceof Error ? err.message : String(err),
    };
  }
}

export const fetchIdentities = (): Promise<IdryxIdentity[]> =>
  call<IdryxIdentity[]>("identity_list_identities");
export const fetchAlerts = (): Promise<IdryxAlert[]> => call<IdryxAlert[]>("identity_list_alerts");
export const fetchRemediations = (): Promise<IdryxRecommendation[]> =>
  call<IdryxRecommendation[]>("identity_list_remediations");

/** Recompute the 21 detectors on demand (`idryx detect --format json`).
 * Returns the same shape `fetchAlerts` does - the caller treats a
 * successful result as the new authoritative alerts view (idryx `serve` is
 * load-once, docs/PHASE3.md: never implied live). */
export const rescan = (): Promise<IdryxAlert[]> => call<IdryxAlert[]>("identity_rescan");

/** Human-readable text for any `IdentityError` - used for the plain error
 * banner (mirrors `lib/policy.ts`'s `describePolicyError`). */
export function describeIdentityError(err: IdentityError): string {
  switch (err.kind) {
    case "bootstrapping":
      return "Still connecting to an Idryx identity plane.";
    case "no_environment":
      return "No Idryx identity plane found.";
    case "unreachable":
      return `Could not reach idryx: ${err.reason}`;
    case "idryx":
      return err.status !== null ? `Idryx error ${err.status}: ${err.message}` : err.message;
    case "rescan_unavailable":
      return "Rescan needs the idryx binary at ~/.taipan/bin/idryx, which was not found.";
  }
}
