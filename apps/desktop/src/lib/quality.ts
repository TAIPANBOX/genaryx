import { invoke, isTauri } from "@tauri-apps/api/core";
import type {
  QualityError,
  QualityStatus,
  VerdryxBaseline,
  VerdryxRunSummary,
  VerdryxScore,
} from "../qualityTypes";

/** Thrown by every fetcher below when there is no Tauri runtime to talk to
 * (a plain `vite build`/browser preview) - mirrors `lib/identity.ts`'s
 * identical `NO_ENVIRONMENT_ERROR` guard: there is no mock quality plane to
 * fall back to, so this surfaces the same "no environment" state a real
 * verdryx-less box would show rather than inventing fake data. */
const NO_ENVIRONMENT_ERROR: QualityError = { kind: "no_environment" };

/** Normalize whatever `invoke()` rejected with into a `QualityError`. Tauri
 * passes a command's `Err` value through as the structured object it was
 * serialized from, so this is normally already a `QualityError` in
 * disguise; the fallback branch only matters for a transport-level IPC
 * failure. */
function toQualityError(err: unknown): QualityError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as QualityError;
  }
  return { kind: "verdryx", message: err instanceof Error ? err.message : String(err) };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw NO_ENVIRONMENT_ERROR;
  try {
    return await invoke<T>(command, args);
  } catch (err) {
    throw toQualityError(err);
  }
}

/** Whole-panel connection state. Never throws: outside Tauri (or on any IPC
 * failure) it resolves to a renderable status instead - mirrors
 * `lib/identity.ts`'s `fetchIdentityStatus` exactly. */
export async function fetchQualityStatus(): Promise<QualityStatus> {
  if (!isTauri()) return { state: "no_environment" };
  try {
    return await invoke<QualityStatus>("quality_status");
  } catch (err) {
    return {
      state: "unreachable",
      source: { source: "well_known" },
      db_path: "",
      reason: err instanceof Error ? err.message : String(err),
    };
  }
}

/** Every eval run, newest-started first, each pre-joined with its own
 * summary - drives BOTH the Eval-runs history table and the Run-detail
 * header once a row is selected (see `quality::commands::quality_list_run_summaries`'s
 * doc comment for why this is one read, not two). */
export const fetchRunSummaries = (): Promise<VerdryxRunSummary[]> =>
  call<VerdryxRunSummary[]>("quality_list_run_summaries");

/** One run's per-case scores, in evaluation order - the Run-detail table. */
export const fetchRunScores = (runId: string): Promise<VerdryxScore[]> =>
  call<VerdryxScore[]>("quality_run_scores", { run_id: runId });

/** Every saved baseline, newest-created first. */
export const fetchBaselines = (): Promise<VerdryxBaseline[]> =>
  call<VerdryxBaseline[]>("quality_list_baselines");

/** Human-readable text for any `QualityError` - used for the plain error
 * banner (mirrors `lib/identity.ts`'s `describeIdentityError`). */
export function describeQualityError(err: QualityError): string {
  switch (err.kind) {
    case "bootstrapping":
      return "Still connecting to a Verdryx quality plane.";
    case "no_environment":
      return "No Verdryx quality plane found.";
    case "unreachable":
      return `Could not open verdryx.db: ${err.reason}`;
    case "verdryx":
      return err.message;
  }
}
