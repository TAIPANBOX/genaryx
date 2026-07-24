# Admission: the "verify" step (I6)

Status: built on branch `feat/admission-gate` (2026-07-23). Design record and
the exact command contract. Follows I6, sits directly after the B2 onboard
wizard (`docs/ONBOARD.md`) and reuses the I15 gateway connection
(`crates/api/src/credentials/`) and Drills plane (`crates/api/src/drills/`)
this build's own brief named as its closest templates.

Defensive intent, as everywhere in this stack: the operator uses this to
PROVE a newcomer agent is correctly admitted before turning on stricter
enforcement, using the newcomer's own key against the operator's own
gateway. Nothing here reaches outside the perimeter the operator already
runs.

## What it is

After the onboard wizard generates a newcomer's artifacts (passport, client
key, identity-map fragment, Wardryx policy stub), nothing has actually
talked to the stack yet - the wizard is offline by design. Admission is the
new plane that closes that gap: it proves the key is known and bound on the
live gateway, that first traffic has flowed, rehearses the guardrails with a
mockryx drill AS the newcomer's own key, optionally establishes a Verdryx
quality baseline for the newcomer through the gateway, then hands back a
copy-paste proposal for enabling `TOKENFUSE_IDENTITY_STRICT`.

**Propose, never mutate** (the same rule `docs/ONBOARD.md` states for its own
wizard): this plane never edits an env var, a config file, or the identity
map. "Enable strict" is a text block the operator copies into their own
gateway's environment and applies themselves, including restarting the
gateway process.

## Where it lives

- `crates/api/src/admission/` - the plane (`env`/`state`/`commands`, mirrors
  `crate::credentials`'s module shape for the ONE piece that benefits from a
  held connection, the gateway; see `env.rs`'s own module doc, "Honest
  per-piece resolution states", for why the verdryx binary/db legs are
  deliberately NOT folded into that same state machine).
- Web shell: three `POST /api/command/admission_*` arms in
  `crates/web/src/dispatch.rs`; role classification in
  `crates/web/src/roles.rs` (`admission_status`/`admission_check` are
  viewer reads, `admission_baseline` is admin-only, the same floor
  `drills_run` sits at).
- UI: a new "Verify (admission gate)" section in
  `components/OnboardView.tsx`, its guts factored into
  `components/AdmissionVerify.tsx` + `lib/admission.ts` + `admissionTypes.ts`
  (mirrors `onboardTypes.ts`'s file split), reachable either right after a
  fresh Generate or via a per-row "Verify" action on an existing provisioned
  passport.
- The Tauri desktop shell and the SwiftUI shell this doc's history once
  named were removed from this repo with the 2026-07-21 web-only pivot; the
  web shell above is the only shell-side wiring this plane has today.

## Command contract (exact)

All DTOs `Serialize` (+ `Deserialize` for requests), snake_case wire names.
Errors are the plane's own tagged enum, `AdmissionError { kind, ... }`,
serialized like every other plane's error (web = 422 body).

### `admission_status() -> AdmissionStatusDto`

```rust
pub struct AdmissionStatusDto {
    pub gateway: GatewayStatusDto,       // Bootstrapping | NoEnvironment | Unreachable{..} | Ready{..}
    pub verdryx_bin: String,             // the one candidate path, always named
    pub verdryx_bin_present: bool,
    pub verdryx_db: Option<VerdryxDbStatusDto>,  // { source, path }
    pub drills_scenario_dir: Option<String>,     // crate::drills::env's own well-known dir, if it exists
}
```

Never fails. The gateway's own connection state is the SAME
Bootstrapping/NoEnvironment/Unreachable/Ready shape `credentials_status`
reports (same `GET /v1/keys` reachability probe, same `GatewayClient`); the
verdryx binary/db legs and the drills scenario dir are independent facts,
re-resolved fresh on every call, never gated together with the gateway leg -
a design deviation from the single-flat-tagged-enum shape every other
plane's status DTO uses, made deliberately: the alternative (folding
"verdryx not installed yet" into the same tag as "no gateway found at all")
would conflate two genuinely different, independently-actionable facts.

### `admission_check(args: { key_id: string, agent_id: string }) -> AdmissionCheckDto`

```rust
pub struct AdmissionCheckDto {
    pub key_id: String,
    pub agent_id: String,
    pub strict_mode: String,             // "off" | "warn" | "enforce", straight off the report
    pub identity_map_configured: bool,
    pub key: Option<GatewayKeyEntry>,    // None = key unknown to the gateway; Some = the whole entry, unchanged
    pub in_map: bool,                    // agent_id matches ANY keys[].agents pattern in the report
}
```

Viewer-safe (a plain read, no side effects beyond the one `GET /v1/keys`
call). `key` carries the gateway's own `GatewayKeyEntry` straight through
(already `Serialize`, no UI-facing mirror struct - the same
`credentials_keys`/`quality_*` precedent their own module docs name), so the
frontend's existing `lib/credentials.ts` helpers (`totalCalls`,
`maxLastSeenMillis`, `lastSeenLabel`) work on it unchanged. `in_map` reuses
the docs/20 pattern grammar (a literal, or a single trailing `*`)
`crate::onboard::commands` already implements for its own `in_map` field -
see `commands.rs`'s own module doc for why this is a faithful COPY of that
grammar rather than a shared import (`crate::onboard` stays untouched by
this branch, so its private `valid_pattern`/`pattern_matches` could not be
bumped to `pub(crate)` and shared).

### `admission_baseline(args: { evalset_path: string, model: string, agent_id: string, api_key: string }) -> AdmissionBaselineDto`

```rust
pub struct AdmissionBaselineDto {
    pub run_id: String,
    pub case_count: u64,
    pub mean_score: Option<f64>,         // None when the run scored zero cases, never a fabricated 0.0
    pub total_cost_usd: f64,
    pub baseline_id_or_label: String,    // the parsed baseline id, or the requested --label as a fallback
}
```

Admin-only (real provider spend under the newcomer's own key). Shells the
`verdryx` binary twice, off the async executor thread
(`tokio::task::spawn_blocking`, mirroring `crate::drills::commands`'s own
off-thread runner):

1. `verdryx eval <evalset_path> --model <model> --agent-id <agent_id> --db <resolved db path>`,
   with `ANTHROPIC_BASE_URL=<gateway base url>` and
   `ANTHROPIC_API_KEY=<api_key>` set on the CHILD PROCESS ONLY
   (`std::process::Command::env`). The eval run id is parsed defensively off
   stdout (`verdryx/cli.py::_cmd_eval`'s exact
   `"Eval run <uuid>  (model=..., db=...)"` line); an unparseable stdout is
   an honest `AdmissionError::UnparseableOutput`, never a guess.
2. `verdryx baseline <run_id> --db <db> --label admission-<agent_id>` - the
   baseline id is parsed the same way, best-effort: if it cannot be read,
   the call still succeeds and reports the `--label` it requested instead
   (a genuine, queryable handle either way).
3. Reads the result back through the EXISTING, unmodified read-only
   `genaryx_connectors::VerdryxClient::run_summary` - no new connector.

### The drill leg needs no new command

The UI's "Run drill as this key" action calls the EXISTING
`drills_run` (`crate::drills::commands::drills_run`) with the newcomer's own
`api_key`, unmodified. Nothing in this plane changes Drills.

## Secret hygiene (critical)

`admission_baseline`'s `api_key` argument:

- Is used exactly twice, both times as a child-process-only environment
  variable (`ANTHROPIC_API_KEY`/`ANTHROPIC_BASE_URL`) - never appended to
  the child's `args` (which would put it in that process's own argv,
  visible to anything inspecting the OS process table on the host), and
  never written via `std::env::set_var` on the console's own process.
- Never appears in any DTO field this plane returns.
- Is defensively redacted (`redact_secret`, a literal find-and-replace) out
  of any subprocess stderr this plane captures before it can reach an
  `AdmissionError` - the last line of defense against an underlying SDK
  error message that happens to echo the credential back.
- Is never journaled: this plane never calls `genaryx_core::command::record`
  (no `CommandRecord`, no `commands_journal` row, no bus event) for ANY of
  its three commands - the same "no journal" contract
  `crate::drills`/`crate::credentials`/`crate::quality` already keep for
  their own read/on-demand commands (a drill or a baseline eval has real
  side effects OUTSIDE the console, but neither mutates any TAIPANBOX
  plane's governance state the way Money's kill/set-budget do).
- On the frontend, the api key field is `type="password"`, held only in
  local component state, never sent anywhere except the two `invoke`
  calls it is for, and cleared on unmount.

**Fact found about `drills_run`'s own pre-existing `api_key` handling**
(reported per this branch's brief, not fixed - `crate::drills` is out of
scope here): `drills_run` also never journals its `api_key` (same "no
`command::record` call at all" fact, confirmed by grep across
`crates/api/src`), and neither the web dispatch route nor the Tauri command
layer logs command arguments anywhere (`crates/web/src/main.rs`'s `command`
handler only logs the command NAME on a role-gate refusal, never the body).
Where `drills_run` genuinely differs from `admission_baseline`: mockryx
receives the key as a `--api-key` CLI ARGUMENT
(`crates/connectors/src/mockryx.rs::MockryxClient::run`), which puts it in
the mockryx child process's own argv - visible to anything with OS
process-table access on that host (`ps`, `/proc/<pid>/cmdline`) for as long
as that process is alive. `admission_baseline` deliberately does NOT do
this: both env vars it sets are child-process-only environment, never argv.

## Strict-mode proposal semantics

Shown only when key bound + `in_map` + first traffic seen + the last drill
attempt ran without an infrastructure error (a report came back at all - per
tokenfuse docs/20, GAPS inside that report are informative, not blocking: a
guardrail gap is exactly the reason to fix the map/policy before flipping
strict, not a reason to hide the proposal; the UI states this in one
sentence next to the block whenever the last drill found any).

The proposal text (copy-paste, changes nothing on its own):

1. `TOKENFUSE_IDENTITY_STRICT=warn` first - a mismatched call still
   proceeds; the response carries `x-fuse-identity: would-block=<reason>`
   and the trace keeps the resolved unit (tokenfuse docs/20 section 3).
2. `TOKENFUSE_IDENTITY_STRICT=enforce` once warn shows no unexpected
   would-block - a mismatched call gets `403` with the `identity_mismatch`
   error contract; the call never reaches the provider.
3. Either step requires restarting the gateway process with the new
   environment variable - this console does not restart it or edit any env
   file for the operator.

## Non-goals (explicit)

- No env mutation, ever - "enable strict" is a copy-paste text block only.
- No auto-strict: nothing in this plane sets `TOKENFUSE_IDENTITY_STRICT`
  itself, and nothing auto-runs a drill or a baseline (every leg is an
  explicit operator click, same "never auto-run" rule `crate::drills`
  states for its own `drills_run`).
- The wizard (`crate::onboard`) stays offline and untouched by this branch.
- No new connector: this plane calls only the EXISTING
  `genaryx_connectors::GatewayClient` and `VerdryxClient`, and shells the
  `verdryx` binary directly from `crates/api/src/admission/commands.rs`
  (mirrors how `crate::drills::commands` shells `mockryx` off-thread; no new
  `crates/connectors` module was added for this).
- No SwiftUI/macOS work (2026-07-21 web-first pivot).

## Follow-ups (named, not silent)

- The per-row "Verify" action on a provisioned passport (`OnboardView.tsx`)
  cannot be exercised under `pnpm dev:mock` today, because
  `apps/web/src/lib/mockPreview.ts` has NO fixture arm for
  `onboard_status` at all (a pre-existing gap in onboard's own mock
  fidelity, found while building this feature, not introduced by it - see
  that file's `mockInvoke` switch, which falls through `onboard_status` to
  its generic `default` case and returns `null`). The Verify section itself
  is fully exercisable under mock regardless, via its own manual key
  id/agent id fields; only that ONE entry point is affected. Fixing
  onboard's own mock arm is out of scope for this branch.
- `drills_run`'s `api_key`-as-argv choice (see "Secret hygiene" above) is a
  materially different, less secret-hygienic pattern than this plane's
  child-process-env-only handling. Reported as a fact for the architect's
  attention; not changed here.
- Verdryx's own `--model stub` (a deterministic, network-free adapter -
  `verdryx/cli.py`'s own module doc) is not specially surfaced anywhere in
  this plane's UI. An operator who wants to dry-run the baseline leg without
  spending real provider money can already type `stub` into the model
  field; a dedicated affordance for that was not built, since the brief
  calls for "no silent default that spends", not a specific no-spend mode.
