import { hasBackend, invokeBackend } from "./transport";
import type {
  OnboardBundle,
  OnboardError,
  OnboardGenerateRequest,
  OnboardStatus,
  OnboardStatusRequest,
  OnboardWritePassportRequest,
  OnboardWriteResult,
} from "../onboardTypes";

/** Thrown by every call below when there is no backend to talk to (a plain
 * `vite build`/browser preview with no Tauri runtime and no configured web
 * API) - mirrors `lib/identity.ts`'s identical `NO_ENVIRONMENT_ERROR` guard.
 * Onboard has no mock-data path of its own (docs/ONBOARD.md's wizard reads
 * the OPERATOR's own local filesystem - there is nothing honest to invent in
 * a browser preview), so a missing backend surfaces the same "no
 * environment" kind every other plane's own no-backend guard reports. */
const NO_ENVIRONMENT_ERROR: OnboardError = {
  kind: "no_environment",
  message: "no backend available",
};

/** Normalize whatever `invoke()` rejected with into an `OnboardError`. Tauri
 * (and `genaryx-web`'s non-2xx body) pass a command's `Err` value through as
 * the structured object it was serialized from, so this is normally already
 * an `OnboardError` in disguise; the fallback branch only matters for a
 * genuine transport-level IPC failure. */
function toOnboardError(err: unknown): OnboardError {
  if (err && typeof err === "object" && "kind" in err && "message" in err) {
    return err as OnboardError;
  }
  return { kind: "transport", message: err instanceof Error ? err.message : String(err) };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!hasBackend()) throw NO_ENVIRONMENT_ERROR;
  try {
    return await invokeBackend<T>(command, args);
  } catch (err) {
    throw toOnboardError(err);
  }
}

/** Whole-panel status: the loaded identity map (if any), the passports dir,
 * and what is already provisioned there. Re-reads the local filesystem fresh
 * on every call - there is no live connection behind this to go stale. */
export const fetchOnboardStatus = (request: OnboardStatusRequest = { map_path: null, passports_dir: null }): Promise<OnboardStatus> =>
  call<OnboardStatus>("onboard_status", { request });

/** Propose a passport + client key + identity-map fragment + Wardryx policy
 * stub + Terraform alternative for one new agent. Writes nothing: the
 * console never mutates on this call, it only returns text the operator
 * copies (or, for the passport alone, can choose to write via
 * `writeOnboardPassport` below). */
export const generateOnboardBundle = (request: OnboardGenerateRequest): Promise<OnboardBundle> =>
  call<OnboardBundle>("onboard_generate", { request });

/** The wizard's ONE write: the passport JSON into the local staging dir.
 * Refused with `{kind: "io", ...}` when the file already exists and
 * `overwrite` is not set - the caller re-tries with `overwrite: true` after
 * an explicit operator confirmation (see `OnboardView.tsx`). */
export const writeOnboardPassport = (request: OnboardWritePassportRequest): Promise<OnboardWriteResult> =>
  call<OnboardWriteResult>("onboard_write_passport", { request });

/** True when `err` is the specific "passport file already exists" refusal -
 * the one case this UI reacts to with an explicit Overwrite confirm rather
 * than a plain error banner. `kind === "io"` alone is not enough (a
 * permission error or an unwritable dir is also `"io"`), so this also checks
 * the message names an existing file. */
export function isExistingFileError(err: OnboardError): boolean {
  return err.kind === "io" && /exist/i.test(err.message);
}

/** Human-readable text for any `OnboardError` - used for the plain error
 * banner, mirrors every sibling panel's `describe*Error`. `kind` is an open
 * string on the wire (see `onboardTypes.ts`'s doc comment), so this falls
 * back to the message itself for any value it does not special-case. */
export function describeOnboardError(err: OnboardError): string {
  switch (err.kind) {
    case "no_environment":
      return "No backend available - open this console from the desktop app, or from a live web session.";
    case "transport":
      return `Could not reach the console backend: ${err.message}`;
    case "validation":
      return err.message;
    case "io":
      return isExistingFileError(err) ? err.message : `Local filesystem error: ${err.message}`;
    default:
      return err.message;
  }
}
