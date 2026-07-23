/**
 * Routines wire types (I7b "Routines tab"). Mirrors the Rust DTOs in
 * `crates/api/src/routines/commands.rs` field-for-field (same convention
 * `identityTypes.ts`/`admissionTypes.ts` follow for their own panels).
 *
 * The console is READ-ONLY here: it does not install, uninstall, or run a
 * routine (that stays the operator's own `routines.sh` on the box, invoked
 * as `./routines.sh install`/`run <name>`/`uninstall`). These types
 * describe what `routines.sh` already recorded under
 * `$STACK_UP_HOME/routines/` - the stable `stackup.routine-run/v1` contract
 * (`~/Development/stack-up/README.md`, "The record").
 */

/** Mirrors `routines::commands::RoutineRunDto` field-for-field. `status` is
 * deliberately a plain string union with an open fallback, not a closed
 * enum: the contract names four values, but an unrecognized fifth must
 * still render - the same tolerance `GatewayKeysReport.strict_mode`
 * (`lib/credentials.ts`) already keeps for its own open-ended wire string. */
export interface RoutineRunDto {
  schema: string;
  routine: string;
  /** RFC3339 UTC. */
  started_at: string;
  /** RFC3339 UTC. */
  finished_at: string;
  exit_code: number;
  /** `ok` (ran, nothing wrong - includes "found something, that's the
   * point"), `findings` (mockryx-drill found a gap), `skipped` (a
   * precondition was not met - see `reason`), or `error` (a real tool
   * failure - see `reason`). An unrecognized value still renders; see this
   * module's doc comment. */
  status: "ok" | "findings" | "skipped" | "error" | (string & {});
  /** Only set for `skipped`/`error`. */
  reason: string | null;
  /** Path under `out/`, when this run produced one. */
  artifact: string | null;
  summary: string | null;
}

/** Mirrors `routines::commands::RoutineSummaryDto` - one row of
 * `RoutinesStatusDto.routines`. */
export interface RoutineSummaryDto {
  name: string;
  /** Whether `installed.txt` names a timer/unit file for this routine. */
  installed: boolean;
  /** `null` = never run (no `status/<name>.json` on disk yet) - a normal,
   * honest state, not an error. */
  latest: RoutineRunDto | null;
  /** Set INSTEAD of `latest` when the status file exists but could not be
   * read/parsed - a per-routine note, never a whole-tab failure. */
  latest_error: string | null;
}

/** Mirrors `routines::commands::RoutinesStatusDto` - `routines_status`'s
 * result. Never fails: a missing routines directory renders as
 * `routines_dir_exists: false` with every routine reporting "never run, not
 * installed", not an error. */
export interface RoutinesStatusDto {
  routines_dir: string;
  routines_dir_exists: boolean;
  /** One entry per `ROUTINE_NAMES` below, always all five. */
  routines: RoutineSummaryDto[];
}

/** Mirrors `routines::commands::RoutinesHistoryDto` - `routines_history`'s
 * result. */
export interface RoutinesHistoryDto {
  /** Newest first (the on-disk file is append-only, newest LAST). */
  records: RoutineRunDto[];
  /** Lines in `history.ndjson` that were not valid JSON / did not match the
   * record shape - truncation/corruption is reported here, never silent. */
  skipped_lines: number;
  routines_dir: string;
  history_file_exists: boolean;
}

/** The five routines `routines.sh` knows about, exact spelling and order as
 * its own `ROUTINE_NAMES` array (and
 * `crate::routines::commands::ROUTINE_NAMES`, which this mirrors). */
export const ROUTINE_NAMES: readonly string[] = [
  "focus-export",
  "qryx-trend",
  "verdryx-drift",
  "idryx-detect",
  "mockryx-drill",
];
