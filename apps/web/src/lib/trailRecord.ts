import { hasBackend, invokeBackend } from "./transport";
import type { RecordPanel } from "../recordTypes";

/**
 * The Record panel's reads (invariant 54). `null` only when there is no backend
 * to ask; a call that fails says so, so "nothing sealed" and "nobody could
 * look" never render the same.
 */
export type RecordRead = { ok: true; panel: RecordPanel } | { ok: false; reason: string };

function reasonOf(err: unknown): string {
  return err && typeof err === "object" && typeof (err as { error?: unknown }).error === "string"
    ? (err as { error: string }).error
    : typeof err === "string"
      ? err
      : "the box did not answer";
}

export async function fetchRecord(): Promise<RecordRead | null> {
  if (!hasBackend()) return null;
  try {
    return { ok: true, panel: await invokeBackend<RecordPanel>("record_status") };
  } catch (err) {
    return { ok: false, reason: reasonOf(err) };
  }
}

/** Runs a verification (up to a minute on the box) and answers the new panel. */
export async function verifyRecord(): Promise<RecordRead | null> {
  if (!hasBackend()) return null;
  try {
    return { ok: true, panel: await invokeBackend<RecordPanel>("record_verify") };
  } catch (err) {
    return { ok: false, reason: reasonOf(err) };
  }
}
