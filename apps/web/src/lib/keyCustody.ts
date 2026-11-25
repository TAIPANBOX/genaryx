import { hasBackend, invokeBackend } from "./transport";
import type { KeysPanel } from "../keysTypes";

/**
 * The Keys panel's read (invariant 53). `null` only when there is no backend
 * to ask; a call that fails says so, so "nothing held" and "nobody could look"
 * never render the same.
 */
export type KeysRead = { ok: true; panel: KeysPanel } | { ok: false; reason: string };

export async function fetchKeys(): Promise<KeysRead | null> {
  if (!hasBackend()) return null;
  try {
    return { ok: true, panel: await invokeBackend<KeysPanel>("keys_custody") };
  } catch (err) {
    const msg =
      err && typeof err === "object" && typeof (err as { error?: unknown }).error === "string"
        ? (err as { error: string }).error
        : typeof err === "string"
          ? err
          : "the box did not answer";
    return { ok: false, reason: msg };
  }
}
