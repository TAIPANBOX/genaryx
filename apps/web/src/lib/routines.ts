/**
 * Routines wire fetchers + pure worst-first ranking / status-to-tone mapping
 * (I7b "Routines tab"). Types live in `../routinesTypes.ts` (mirrors
 * `identityTypes.ts` + `lib/identity.ts`'s split - see this plane's own
 * doc comment on the choice, since `lib/credentials.ts` combines both in
 * one file instead); fetchers and the pure ranking/tone logic live here.
 *
 * Read-only, like `lib/onboard.ts`: neither Rust command behind this module
 * can fail in a way the frontend needs to distinguish (`routines_status`/
 * `routines_history` are both `Result<_, ()>` - see
 * `crates/api/src/routines/commands.rs`'s doc comment), so there is no
 * `RoutinesError` tagged union here. The one way either fetcher throws at
 * all is "no backend to talk to" (`hasBackend()` false), mirrored as a plain
 * `Error`.
 */
import { hasBackend, invokeBackend } from "./transport";
import { humanizeAge } from "./credentials";
import type { RoutineRunDto, RoutineSummaryDto, RoutinesHistoryDto, RoutinesStatusDto } from "../routinesTypes";

export type { RoutineRunDto, RoutineSummaryDto, RoutinesHistoryDto, RoutinesStatusDto } from "../routinesTypes";
export { ROUTINE_NAMES } from "../routinesTypes";

/** Thrown by the fetchers below when there is no backend to talk to at all -
 * mirrors `lib/credentials.ts`'s identical no-backend guard, just a plain
 * `Error` (not a tagged union) since this plane has none of its own - see
 * this module's doc comment. */
const NO_BACKEND_MESSAGE = "no backend: cannot reach the routines plane (no Tauri runtime and no VITE_GENARYX_API)";

// ============================================================================
// Fetchers
// ============================================================================

/** `routines_status` - the resolved routines dir (+ exists flag) and one row
 * per known routine. Never throws once a backend exists (the Rust command is
 * infallible); throws only outside any backend at all. */
export async function fetchRoutinesStatus(): Promise<RoutinesStatusDto> {
  if (!hasBackend()) throw new Error(NO_BACKEND_MESSAGE);
  return invokeBackend<RoutinesStatusDto>("routines_status");
}

/** `routines_history` - optionally filtered to one routine, capped at
 * `limit` (the backend applies its own default/hard-cap when omitted, see
 * `crates/api/src/routines/commands.rs`'s `DEFAULT_HISTORY_LIMIT`/
 * `MAX_HISTORY_LIMIT`). */
export async function fetchRoutinesHistory(routine?: string, limit?: number): Promise<RoutinesHistoryDto> {
  if (!hasBackend()) throw new Error(NO_BACKEND_MESSAGE);
  return invokeBackend<RoutinesHistoryDto>("routines_history", { routine, limit });
}

// ============================================================================
// Worst-first ranking + status-to-tone mapping (pure, unit tested in
// routines.test.ts - mirrors `lib/credentials.ts`'s `deriveKeyStatus`/
// `KEY_STATUS_ORDER` convention)
// ============================================================================

/** A routine row's overall UI status: the four real backend `status` values
 * PLUS two console-side pseudo-states that are not part of the wire
 * contract at all:
 *
 * - `"never"` - no `status/<name>.json` on disk yet (`latest === null` and
 *   `latest_error === null`). Not a backend `status` value - it is the
 *   ABSENCE of a recorded run, not a recorded outcome.
 * - `"unreadable"` - the status file exists but this console could not read
 *   or parse it (`latest_error !== null`). A console-side read problem,
 *   distinct from anything `routines.sh` itself ever recorded.
 * - `"unknown"` - a real record parsed fine, but its `status` string is
 *   none of the four the contract names today (forward-compatible with a
 *   future fifth value - see `routinesTypes.ts`'s doc comment). */
export type RoutineUiStatus = "unreadable" | "error" | "findings" | "unknown" | "skipped" | "never" | "ok";

/**
 * Worst-first order, reused both by {@link routineStatusRank} (the summary
 * table's sort) and {@link ROUTINE_STATUS_TONE}/{@link ROUTINE_STATUS_LABEL}
 * below - one array, so ranking and display can never quietly disagree,
 * mirroring `lib/credentials.ts`'s `KEY_STATUS_ORDER` convention exactly.
 *
 * Precedence, worst first:
 *
 * 1. `unreadable` - this console could not even read what `routines.sh`
 *    recorded; ranked above `error` because there is LESS signal here, not
 *    more (an error at least says what went wrong).
 * 2. `error` - a real tool/usage failure (stack-up README: "a real
 *    usage/tool failure").
 * 3. `findings` - mockryx-drill found a gap. Notable (it is what a drill is
 *    for), but the README is explicit this is not a failure.
 * 4. `unknown` - a real record, but a `status` value this console does not
 *    recognize (a future stack-up version) - cautious middle ground: not
 *    confirmed fine, but not one of the two clearly-urgent states either.
 * 5. `skipped` / 6. `never` - both "nothing to act on yet", per the
 *    README's OWN framing ("a precondition not met... is the expected state
 *    right after a fresh install, not a broken one" - equally true of a
 *    routine that has simply never fired). `skipped` sits one slot ahead
 *    only to give the array a total order, not because it is claimed to be
 *    genuinely worse than `never`.
 * 7. `ok` - ran, nothing wrong.
 */
export const ROUTINE_STATUS_ORDER: readonly RoutineUiStatus[] = [
  "unreadable",
  "error",
  "findings",
  "unknown",
  "skipped",
  "never",
  "ok",
];

const ROUTINE_STATUS_RANK: Readonly<Record<RoutineUiStatus, number>> = Object.fromEntries(
  ROUTINE_STATUS_ORDER.map((s, i) => [s, i]),
) as Record<RoutineUiStatus, number>;

/** Sort comparator input: worst-first, by {@link ROUTINE_STATUS_ORDER}. */
export function routineStatusRank(status: RoutineUiStatus): number {
  return ROUTINE_STATUS_RANK[status];
}

/** One dash-kit CSS variable per {@link RoutineUiStatus} - mirrors
 * `CredentialsKeysTable.tsx`'s `STATUS_TONE` convention: `ok` green
 * (`--mint`), `findings` amber (`--sev-medium`), `skipped`/`never`/`unknown`
 * neutral/muted (`--faint`), `error`/`unreadable` red (`--sev-high`). No new
 * palette entry - every value here is a token this codebase already uses
 * for the same meaning elsewhere. */
export const ROUTINE_STATUS_TONE: Readonly<Record<RoutineUiStatus, string>> = {
  unreadable: "var(--sev-high)",
  error: "var(--sev-high)",
  findings: "var(--sev-medium)",
  unknown: "var(--faint)",
  skipped: "var(--faint)",
  never: "var(--faint)",
  ok: "var(--mint)",
};

export const ROUTINE_STATUS_LABEL: Readonly<Record<RoutineUiStatus, string>> = {
  unreadable: "unreadable",
  error: "error",
  findings: "findings",
  unknown: "unknown",
  skipped: "skipped",
  never: "never run",
  ok: "ok",
};

/**
 * Derive one routine's overall UI status from its `RoutinesStatusDto` row -
 * see {@link RoutineUiStatus}'s doc comment for what each value means and
 * the exact precedence: `latest_error` (unreadable) wins over anything else
 * whenever it is set, `latest === null` (never run) is checked next, and
 * only then does a real record's own `status` string get classified.
 */
export function toUiStatus(row: Pick<RoutineSummaryDto, "latest" | "latest_error">): RoutineUiStatus {
  if (row.latest_error !== null) return "unreadable";
  if (row.latest === null) return "never";
  // Explicit literal returns rather than `return row.latest.status` - the
  // wire type's `(string & {})` fallback branch (see `routinesTypes.ts`)
  // prevents TypeScript from narrowing `status` to just the four matched
  // cases here, so returning the matched value itself would widen back to
  // the full open union instead of `RoutineUiStatus`.
  switch (row.latest.status) {
    case "ok":
      return "ok";
    case "findings":
      return "findings";
    case "skipped":
      return "skipped";
    case "error":
      return "error";
    default:
      return "unknown";
  }
}

/** Map a raw, already-parsed record's `status` string to the same tone
 * {@link ROUTINE_STATUS_TONE} uses - for the per-run history table, where
 * every row is a real parsed record and neither of the two console-side
 * pseudo-states (`never`/`unreadable`) ever applies (those describe the
 * ABSENCE of a record or a record this console could not read at all, never
 * one of the rows in a successfully-parsed history list). */
export function recordStatusTone(status: string): string {
  switch (status) {
    case "ok":
      return ROUTINE_STATUS_TONE.ok;
    case "findings":
      return ROUTINE_STATUS_TONE.findings;
    case "skipped":
      return ROUTINE_STATUS_TONE.skipped;
    case "error":
      return ROUTINE_STATUS_TONE.error;
    default:
      return ROUTINE_STATUS_TONE.unknown;
  }
}

/** Sort a COPY of `routines` worst-first by {@link routineStatusRank}, tying
 * on routine name for a fully deterministic order - mirrors
 * `CredentialsKeysTable`'s identical `keyStatusRank(...) || localeCompare`
 * pattern. Never mutates its input. */
export function sortRoutinesWorstFirst(routines: readonly RoutineSummaryDto[]): RoutineSummaryDto[] {
  return [...routines].sort((a, b) => {
    const rankDiff = routineStatusRank(toUiStatus(a)) - routineStatusRank(toUiStatus(b));
    return rankDiff !== 0 ? rankDiff : a.name.localeCompare(b.name);
  });
}

/**
 * The one-line detail to show for a routine's latest run: `reason` when the
 * status is `skipped`/`error` AND a reason was actually recorded, else
 * `summary`, else an honest placeholder. Mirrors `routines.sh`'s own
 * `last_status_line` precedence exactly (`detail = reason if status in
 * ("skipped", "error") and reason else summary`), so this console's one-line
 * reading matches what `./routines.sh status` on the box itself would print.
 */
export function latestDetailLine(latest: RoutineRunDto): string {
  const preferReason = (latest.status === "skipped" || latest.status === "error") && Boolean(latest.reason);
  const detail = preferReason ? latest.reason : latest.summary;
  return detail && detail.length > 0 ? detail : "(no detail recorded)";
}

/** "5m ago"/"2d ago"/"never"/"unknown" for a routine's latest run, parsed
 * off `finished_at` (RFC3339 UTC - `Date.parse` handles it natively) -
 * reuses `lib/credentials.ts`'s {@link humanizeAge} so every "relative time"
 * reading in this console is phrased identically, rather than a second,
 * parallel formatter. */
export function latestRelativeTime(latest: RoutineRunDto | null, nowMillis: number): string {
  if (latest === null) return "never";
  const parsed = Date.parse(latest.finished_at);
  return Number.isFinite(parsed) ? humanizeAge(nowMillis - parsed) : "unknown";
}
