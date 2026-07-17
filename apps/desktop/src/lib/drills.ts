import { invoke, isTauri } from "@tauri-apps/api/core";
import type { DrillsError, DrillsStatus, MockryxReport } from "../drillsTypes";

/** Thrown by every fetcher below when there is no Tauri runtime to talk to -
 * mirrors `lib/crypto.ts`'s identical `NO_ENVIRONMENT_ERROR` guard. */
const NO_ENVIRONMENT_ERROR: DrillsError = { kind: "no_environment" };

/** Normalize whatever `invoke()` rejected with into a `DrillsError` - mirrors
 * `lib/crypto.ts`'s `toCryptoError`. */
function toDrillsError(err: unknown): DrillsError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as DrillsError;
  }
  return { kind: "mockryx", message: err instanceof Error ? err.message : String(err) };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw NO_ENVIRONMENT_ERROR;
  try {
    return await invoke<T>(command, args);
  } catch (err) {
    throw toDrillsError(err);
  }
}

/** Whole-panel connection state. Never throws: `drills_status` itself never
 * fails (see `drills::commands::drills_status`'s doc comment), so the only
 * way this catches is a genuine IPC-transport failure - folded into the same
 * honest "no drills plane" state a missing mockryx binary/gateway would
 * show (mirrors `lib/crypto.ts`'s `fetchCryptoStatus`: no `unreachable`
 * variant to fall back to either). */
export async function fetchDrillsStatus(): Promise<DrillsStatus> {
  if (!isTauri()) return { state: "no_environment" };
  try {
    return await invoke<DrillsStatus>("drills_status");
  } catch {
    return { state: "no_environment" };
  }
}

/** `mockryx run --gateway <resolved> --format json [--api-key K]
 * [--fail-on-skip] [--save P] <scenarioDir>` - never auto-run, only on an
 * explicit operator click. Blank `apiKey`/`savePath` fall back to the
 * resolved environment's own values (see
 * `drills::commands::drills_run`'s doc comment). */
export const runDrills = (
  scenarioDir: string,
  apiKey: string,
  failOnSkip: boolean,
  savePath: string,
): Promise<MockryxReport> =>
  call<MockryxReport>("drills_run", {
    scenario_dir: scenarioDir,
    api_key: apiKey.trim().length > 0 ? apiKey : null,
    fail_on_skip: failOnSkip,
    save_path: savePath.trim().length > 0 ? savePath : null,
  });

/** Human-readable text for any `DrillsError` - used for the plain error
 * banner. */
export function describeDrillsError(err: DrillsError): string {
  switch (err.kind) {
    case "bootstrapping":
      return "Still resolving the mockryx binary and gateway.";
    case "no_environment":
      return "No drills plane found.";
    case "mockryx":
      return err.message;
  }
}
