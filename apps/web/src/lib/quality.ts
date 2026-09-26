import { hasBackend, invokeBackend } from "./transport";
import type {
  QualityError,
  QualityStatus,
  VerdryxBaselineSummary,
  VerdryxRunSummary,
  VerdryxScore,
} from "../qualityTypes";

/** Thrown by every fetcher below when there is no backend to talk to
 * (a plain `vite build`/browser preview) - mirrors `lib/identity.ts`'s
 * identical `NO_ENVIRONMENT_ERROR` guard: there is no mock quality plane to
 * fall back to, so this surfaces the same "no environment" state a real
 * verdryx-less box would show rather than inventing fake data. */
const NO_ENVIRONMENT_ERROR: QualityError = { kind: "no_environment" };

/** Normalize whatever `invokeBackend()` rejected with into a `QualityError`. genaryx-web
 * passes a command's `Err` value through as the structured object it was
 * serialized from, so this is normally already a `QualityError` in
 * disguise; the fallback branch only matters for a transport-level
 * failure. */
function toQualityError(err: unknown): QualityError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as QualityError;
  }
  return { kind: "verdryx", message: err instanceof Error ? err.message : String(err) };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!hasBackend()) throw NO_ENVIRONMENT_ERROR;
  try {
    return await invokeBackend<T>(command, args);
  } catch (err) {
    throw toQualityError(err);
  }
}

/** Whole-panel connection state. Never throws: with no backend (or on any
 * transport failure) it resolves to a renderable status instead - mirrors
 * `lib/identity.ts`'s `fetchIdentityStatus` exactly. */
export async function fetchQualityStatus(): Promise<QualityStatus> {
  if (!hasBackend()) return { state: "no_environment" };
  try {
    return await invokeBackend<QualityStatus>("quality_status");
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

/** Every saved baseline, newest-created first, each paired with the
 * unanswered accounting of the run it was snapshotted from. */
export const fetchBaselines = (): Promise<VerdryxBaselineSummary[]> =>
  call<VerdryxBaselineSummary[]>("quality_list_baselines");

/** Something with a mean score and an unanswered breakdown - the shape both
 * [`VerdryxRunSummary`](../qualityTypes) and [`VerdryxBaselineSummary`] carry,
 * factored out so one formatter serves both the Eval-runs table and
 * Baselines. */
export interface QualityMeanFields {
  mean_score: number | null;
  case_count: number;
  unanswered_count: number;
  unanswered_by_reason: { reason: string; count: number }[];
  unanswered_supported: boolean;
}

/** The mean-score sentence the Quality panel reads instead of a bare number:
 * "mean 0.82 over 57 answered, 3 unanswered (label_mass_too_low: 3)", or
 * "unmeasured" for a run that answered none at all - never a mean of `0`.
 *
 * On a store that predates verdryx's `unanswered` table
 * (`unanswered_supported` false), the unanswered half is omitted entirely
 * rather than rendered as a `0` that looks measured. */
export function formatQualityMean(s: QualityMeanFields): string {
  if (s.mean_score === null) return "unmeasured";
  const base = `mean ${s.mean_score.toFixed(2)} over ${s.case_count} answered`;
  if (!s.unanswered_supported || s.unanswered_count === 0) return base;
  const reasons = s.unanswered_by_reason.map((r) => `${r.reason}: ${r.count}`).join(", ");
  return `${base}, ${s.unanswered_count} unanswered (${reasons})`;
}

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
