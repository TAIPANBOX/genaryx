# Routines: the "Routines tab" (I7b)

Status: built on branch `feat/routines-tab` (2026-07-23). Design record and
the exact command contract. Surfaces what stack-up's `routines.sh` (already
merged, I7a) records under `$STACK_UP_HOME/routines/` - the stable
`stackup.routine-run/v1` contract documented in
`~/Development/stack-up/README.md`, section "Scheduled governance runs" /
"The record".

Defensive intent, as everywhere in this stack: an operator who schedules
governance work (a FinOps export, a crypto-inventory trend, a quality-drift
check, an identity-anomaly sweep, a fire drill) only benefits from it if
someone actually looks at whether it ran and what it found. This tab is that
look.

## What it is

`routines.sh` installs OS-native timers (systemd on Linux, launchd on macOS)
for five routines and is also the thing those timers invoke. Every run is
recorded twice: appended to `history.ndjson` (the full history) and written
atomically to `status/<name>.json` (just the latest). This tab lists the
five routines, each showing whether it is installed as a scheduled timer and
the history of its runs, worst-first.

**Read-only, stated plainly**: this console does NOT install, uninstall, or
run a routine. That remains the operator's own `routines.sh` on the box
(`./routines.sh install [--with-drill]`, `run <name>`, `uninstall`). This
tab only SURFACES what `routines.sh` already recorded - a future
"install/run from the console" is an explicit, named follow-up below, not
something this build does.

## The five routines

| Routine | Cadence | What a `findings`/`skipped`/`error` outcome means |
|---|---|---|
| `focus-export` | daily 06:07 | `error`: the gateway binary or export command failed. `skipped`: no traces recorded yet (fresh install). |
| `qryx-trend` | daily 06:17 | `error`: `qryx scan`/`trend` failed. `skipped`: missing binary or scan path. |
| `verdryx-drift` | daily 06:27 | `error`: a real `verdryx drift` failure. `skipped`: no baseline configured (`ROUTINE_VERDRYX_BASELINE` unset) - the common state until an operator sets one. |
| `idryx-detect` | daily 06:37 | `error`: `idryx detect` failed. `skipped`: missing binary or events file. Never `findings` - idryx always exits 0 regardless of alert count, so a real sweep with alerts is `ok`. |
| `mockryx-drill` | weekly, Monday 06:47, **opt-in only** | `findings`: the drill found a guardrail gap - the point of running it, not a failure. `error`: a real infrastructure failure (e.g. no reachable gateway). Never installed by a plain `routines.sh install`; needs `--with-drill`. |

`status` is exactly one of `ok | findings | skipped | error` per the
contract (`ok` includes "found something, that's the point" - e.g. an
`idryx-detect` sweep that reports alerts is still `ok`). This console
carries an unrecognized fifth value through as a plain string rather than
rejecting it - see "Command contract" below.

## Where it lives

- `crates/api/src/routines/` - the plane (`env`/`commands`, no `state`: every
  call re-reads a handful of small local files fresh - see `mod.rs`'s own
  doc comment for why this mirrors `crate::onboard`'s "no env/state pair"
  shape for `state` specifically, while still keeping its own `env` module
  since the routines-dir resolution rule is non-trivial enough to deserve
  one).
- Web shell: two `POST /api/command/routines_*` arms in
  `crates/web/src/dispatch.rs`; both classified `viewer` in
  `crates/web/src/roles.rs`. Stateless like Onboard/Pocket - nothing is
  held in `AppState` for this plane.
- UI: `components/RoutinesView.tsx` + `lib/routines.ts` +
  `routinesTypes.ts`, a new "Routines" view in the shell (`lib/views.ts`,
  `AppShell.tsx`), positioned next to Posture/Bus Explorer as an
  ops/observability surface.
- The Tauri desktop shell and the SwiftUI shell this doc's history once
  named were removed from this repo with the 2026-07-21 web-only pivot; the
  web dispatcher above is the only shell-side wiring this plane has today.

## Environment resolution

`crates/api/src/routines/env.rs` resolves the routines directory, honoring
the SAME `STACK_UP_HOME` variable `routines.sh` itself reads:

1. `$STACK_UP_HOME/routines`, when `STACK_UP_HOME` is set.
2. `~/.stack-up/routines` otherwise (`routines.sh`'s own default).

This is deliberately NOT `genaryx_core::taipan_home` and consults no
`taipan up` descriptor: routines is a stack-up concept (`$STACK_UP_HOME`),
not a taipan-up plane (`$TAIPAN_HOME`) - the two home directories are
siblings on disk, not the same thing. Resolution never fails: a directory
that does not exist yet is reported honestly (`routines_dir_exists: false`),
not an error - the expected state right after a fresh `stack-up` clone,
before `routines.sh` has ever run.

## Command contract (exact)

Both DTOs `Serialize` (+ `Deserialize` for the one request), snake_case wire
names. Neither command has a tagged error enum: both are `Result<_, ()>`,
mirroring `credentials_status`/`quality_status`/`admission_status` - every
failure mode this plane has (a missing directory, an unparseable status
file, a malformed history line) is modeled as an honest field on the
response, never a command-level error.

### `routines_status() -> RoutinesStatusDto`

```rust
pub struct RoutinesStatusDto {
    pub routines_dir: String,
    pub routines_dir_exists: bool,
    pub routines: Vec<RoutineSummaryDto>,   // always all five, see ROUTINE_NAMES
}

pub struct RoutineSummaryDto {
    pub name: String,
    pub installed: bool,                     // installed.txt names this routine's timer/unit file
    pub latest: Option<RoutineRunDto>,        // None = never run (no status/<name>.json yet)
    pub latest_error: Option<String>,         // Some = status file exists but could not be read/parsed
}
```

`installed` is derived by reading `installed.txt` and checking whether any
line contains the literal substring `routine-<name>` -
`install_systemd_unit`/`install_launchd_unit` write
`stack-up-routine-<name>.{service,timer}` (systemd) or
`dev.taipanbox.stack-up.routine-<name>.plist` (launchd), and no name in the
fixed five-routine list is a prefix of another, so the substring check is
exact.

`latest`/`latest_error` are mutually exclusive by construction (never both
set): a missing status file is `(None, None)` ("never run" - genuinely
distinct from `installed: false`, since a timer can be installed and simply
not have fired yet), an unreadable/unparseable one is `(None,
Some(message))`, and a good one is `(Some(record), None)`. Never fails the
whole command: one routine's bad status file is that ONE routine's problem.

### `routines_history(args: { routine?: string, limit?: u32 }) -> RoutinesHistoryDto`

```rust
pub struct RoutinesHistoryDto {
    pub records: Vec<RoutineRunDto>,   // newest first
    pub skipped_lines: u32,            // history.ndjson lines that were not valid records
    pub routines_dir: String,
    pub history_file_exists: bool,
}
```

Pipeline: parse every line of `history.ndjson` (a non-JSON/non-matching line
increments `skipped_lines` and is dropped, never fatal) -> reverse (the file
is append-only, newest LAST) -> filter to `routine` when given -> cap at
`limit` (default 200, hard-capped at 1000 regardless of what is asked for -
`history.ndjson` is never rotated, so an unbounded ask must not become an
unbounded read). `skipped_lines` counts malformed lines across the WHOLE
file, before the per-routine filter - it is a fact about the file, not about
one routine's slice of it.

### `RoutineRunDto` - the stable v1 record, verbatim

```rust
pub struct RoutineRunDto {
    pub schema: String,          // "stackup.routine-run/v1"
    pub routine: String,
    pub started_at: String,      // RFC3339 UTC
    pub finished_at: String,     // RFC3339 UTC
    pub exit_code: i64,
    pub status: String,          // "ok" | "findings" | "skipped" | "error", open-ended (see below)
    pub reason: Option<String>,  // set for skipped/error
    pub artifact: Option<String>,// path under out/, when produced
    pub summary: Option<String>,
}
```

`status` is deliberately a plain `String`, not a closed Rust enum or a
closed TypeScript union: the contract names four values today, but this
console does not own that contract (stack-up does) and must not reject a
future fifth value outright. The frontend types it as
`"ok" | "findings" | "skipped" | "error" | (string & {})` - the same
open-string tolerance `GatewayKeysReport.strict_mode`/`IdryxAlert.severity`
already keep for their own wire strings - and
`lib/routines.ts::toUiStatus` maps an unrecognized value to its own
`"unknown"` UI status rather than crashing or silently dropping the row.

## Worst-first ranking (frontend, `lib/routines.ts`)

The summary table sorts by a console-side `RoutineUiStatus`, worst first:

`unreadable` (the console could not read/parse the status file - less
signal than a real error, ranked above it) > `error` > `findings` >
`unknown` (a real record, but a status value this console does not
recognize) > `skipped` / `never` (both "nothing to act on yet", per the
README's own framing that an unmet precondition is the expected state right
after a fresh install, not a broken one - `skipped` is placed one slot
ahead only to give the array a total order) > `ok`.

Clicking a routine loads its `routines_history` (filtered to that one name)
into a compact history panel below, auto-selecting the worst-ranked routine
on load so the operator lands on the thing most worth looking at first.

## Non-goals (explicit)

- No install/uninstall/run from the console, ever, in this build. Every
  write this plane could plausibly make (arming a timer, invoking
  `routines.sh run <name>`) stays the operator's own action on the box.
- No env mutation, no config file editing (`$STACK_UP_HOME/routines/config`
  is read only by `routines.sh` itself; this console does not read or write
  it either).
- No new connector: this plane reads local files directly
  (`std::fs`), no network, no descriptor dependency, no sibling binary to
  shell out to - simpler than every other plane in `crates/api/src`, per
  the brief that named it "simplest of all".
- No SwiftUI/macOS work (2026-07-21 web-first pivot).

## Follow-ups (named, not silent)

- "Install/run from the console": a genuine future capability (arm a
  timer, trigger `routines.sh run <name>` remotely) that this build
  deliberately does not build. Would need its own admin-gated command(s),
  a decision on how the console reaches the box's own `routines.sh` (SSH,
  per `crate::remote`?), and a fresh look at whether the read-only/mutating
  split this doc states should change.
- A live/auto-refreshing variant (poll on an interval, like
  `QualityView.tsx`'s 60s poll over `verdryx.db`) was considered and not
  built: `history.ndjson`/`status/*.json` change only once a day per
  routine at most (weekly for the drill), so a `snapshot`-style explicit
  Refresh (this build's choice) costs nothing an operator would notice
  versus a timer-driven poll, and adds no new interval to reason about.
- Reading `$STACK_UP_HOME/routines/config` to show which env vars a
  `skipped` routine is missing (beyond the `reason` string `routines.sh`
  itself already records) was considered out of scope - the recorded
  `reason` already names the exact missing variable/path in every skip case
  this build's own fixtures exercise.
