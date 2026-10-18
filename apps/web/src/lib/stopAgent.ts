import type { LiftResult, StopResult } from "../delegationTypes";

/** One line of an outcome the operator reads. */
export interface ReportLine {
  tone: "ok" | "bad" | "warn";
  text: string;
}

/** A stop's or a lift's outcome, as the operator reads it. */
export interface StopReport {
  /** True only for a complete stop or lift. */
  done: boolean;
  headline: string;
  lines: ReportLine[];
}

/** SCAFFOLD (red-first commit): the console has no stop yet. */
export function stopReport(_result: StopResult): StopReport {
  return { done: false, headline: "", lines: [] };
}

/** SCAFFOLD (red-first commit): the console has no lift yet. */
export function liftReport(_result: LiftResult): StopReport {
  return { done: false, headline: "", lines: [] };
}
