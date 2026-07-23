import { hasBackend, invokeBackend } from "./transport";
import type { PocketError, PocketQr, PocketStatus } from "../pocketTypes";

/** The honest, SETTLED fallback shape for `fetchPocketStatus` with no
 * backend - mirrors `lib/remote.ts`'s `REMOTE_UNAVAILABLE`/`lib/evidence.ts`'s
 * `EVIDENCE_UNAVAILABLE`: a real, renderable "nothing configured" state
 * (Connect would have nothing to resolve either, so `cloud_ready: false` is
 * accurate, not a guess) rather than a stuck "loading" placeholder. No
 * backend also means no relay to have armed a window, so both windows are
 * honestly `null` here too. */
const POCKET_UNAVAILABLE_NO_BACKEND: PocketStatus = {
  state: "idle",
  cloud_ready: false,
  phone_window: null,
  watch_window: null,
};

/** Thrown by `pocketConnect`/`pocketDisconnect` when there is no backend
 * to talk to - mirrors `lib/remote.ts`'s `NO_BACKEND_ERROR`. */
const NO_BACKEND_ERROR: PocketError = { kind: "relay", message: "no backend available" };

/** Normalize whatever `invokeBackend()` rejected with into a `PocketError`. genaryx-web
 * passes a command's `Err` value through as the structured object it was
 * serialized from, so this is normally already a `PocketError` in disguise;
 * the fallback branch only matters for a transport-level failure. */
function toPocketError(err: unknown): PocketError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as PocketError;
  }
  return { kind: "relay", message: err instanceof Error ? err.message : String(err) };
}

async function call<T>(command: string): Promise<T> {
  if (!hasBackend()) throw NO_BACKEND_ERROR;
  try {
    return await invokeBackend<T>(command);
  } catch (err) {
    throw toPocketError(err);
  }
}

/** Whole-panel status (idle / paired / relay-unreachable). Never throws:
 * with no backend it settles to [`POCKET_UNAVAILABLE_NO_BACKEND`], and
 * `pocket_status` itself never fails on the Rust side either (see
 * `pocket::commands::pocket_status`'s doc), so the catch branch only covers
 * a genuine transport-level failure. */
export async function fetchPocketStatus(): Promise<PocketStatus> {
  if (!hasBackend()) return POCKET_UNAVAILABLE_NO_BACKEND;
  try {
    return await invokeBackend<PocketStatus>("pocket_status");
  } catch (err) {
    return { state: "relay_unreachable", message: err instanceof Error ? err.message : String(err) };
  }
}

/** "Connect TokenFuse Pocket": mint a code at the Cloud, arm the relay's
 * pairing window, and return the QR content to render (docs/PHASE5.md W2).
 * A `{kind:"device_exists"}` rejection means a phone is already paired - the
 * caller should re-fetch `fetchPocketStatus()` and show the Paired view
 * rather than an error banner (see `PocketView.tsx`). */
export const pocketConnect = (): Promise<PocketQr> => call<PocketQr>("pocket_connect");

/** Disconnect the paired phone (always safe to call). Returns the fresh
 * whole-panel status - no second round trip needed. */
export const pocketDisconnect = (): Promise<PocketStatus> => call<PocketStatus>("pocket_disconnect");

/** Human-readable text for any `PocketError` - used for the plain error
 * banner, mirrors every sibling panel's `describe*Error`. */
export function describePocketError(err: PocketError): string {
  switch (err.kind) {
    case "no_cloud_environment":
      return "No TokenFuse Cloud environment found (see Money) - cannot mint a pairing code.";
    case "cloud":
      return `Cloud error: ${err.message}`;
    case "device_exists":
      return "A phone is already paired - disconnect it first to pair a different one.";
    case "relay":
      return `Relay error: ${err.message}`;
  }
}
