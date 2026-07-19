# Phase 6 · C1 — Felyx triage + explanation

Build contract for C1, extending [PHASE6.md](PHASE6.md) / [C0 results](PHASE6-C0-RESULTS.md).
Architecture: itrat-console/13 D13.7 C1 — "incident 'explain' cards, cross-plane root-cause
chains, Engram recall integration, false-alarm memory loop".

C0 shipped the read path with 10 async connector tools, all parameterless, and the shells
passed `Clients::default()` (no tools in-shell). C1 does three things:

1. **A sync-tool bridge** so the remaining connectors (Qryx CLI, Verdryx SQLite, Engram
   MCP-stdio) can back tools — they are synchronous and, for Engram, `&mut self`.
2. **New tools + the explain flow** — memory recall, quality, crypto posture, and a
   cross-plane `explain_incident` that chains the money / policy / identity planes into a
   root-cause with evidence refs.
3. **Wire real `Clients` in both shells** (deferred from C0) so the copilot actually has
   tools against the live planes, plus an "Explain with Felyx" affordance on incidents.

## C1-W1 — crate (orchestrator-owned)

### The sync-tool bridge
The `Tool::run` signature stays `async`. A sync-backed tool does its blocking work inside
`tokio::task::spawn_blocking`, moving a cheap, `Send + 'static` handle into the closure:
- **Qryx**: `QryxClient` (holds a `PathBuf`, shells out per call) — `Clone`, move a clone in.
- **Verdryx**: hold the **db path** (`PathBuf`), open `VerdryxClient::open(path)` fresh
  inside the closure (a `rusqlite::Connection` is `!Sync`, so it is never shared).
- **Engram**: `Arc<Mutex<EngramClient>>` (the MCP client is `&mut self` + a long-lived stdio
  child), lock inside the closure so calls serialize.

`Clients` grows: `engram: Option<Arc<Mutex<EngramClient>>>`, `qryx: Option<QryxClient>`,
`verdryx_db: Option<PathBuf>`.

### New tools
| Tool | Args | Backing | Notes |
|---|---|---|---|
| `memory_recall` | `{query: string, limit?: int}` | `EngramClient::recall` (hybrid mode) | past incidents / rulings relevant to a query |
| `memory_why` | `{memory_id: string}` | `EngramClient::why` | provenance of one memory |
| `quality_latest` | none | `VerdryxClient::latest_run` + `run_summary` | the newest eval run + its rollup |
| `crypto_scan` | `{path: string}` | `QryxClient::scan_ncsc` | the NCSC PQC-readiness posture of a path — the **first parameterized tool** (exercises the loop's argument path) |

`crypto_scan` validates its `path` arg (non-empty, exists) and returns an error-as-data on a
bad path rather than failing the answer.

### The explain flow
`CopilotService::explain_incident(incident_id) -> Answer`: a thin wrapper over `ask` that
seeds the loop with an incident-focused instruction — "explain incident X: pull the
incident, the run's spend trajectory (alerts/runs), the agent's identity posture
(identity_alerts), any governing policy (policies), and recall past rulings (memory_recall);
give a root-cause chain and cite the row ids." The chain the D13.4 example shows ("spend
spike → wardryx hold → idryx unattested") is entirely connector-backed, so C1 delivers it
without the core-`Store` event/graph tools (those need an `Arc<Mutex<Store>>` shared from the
shell's ingest — a deferred enhancement, noted below).

### The false-alarm memory loop (human-gated write)
`memory_remember` is an APPEND, so it is NOT a free model tool. It is a
`CopilotService::remember_ruling(text, tags)` method the shell calls only after a HUMAN
records a ruling ("this pattern was a false alarm on 2026-07-02"), via `EngramClient`'s
generic `call_tool("remember", …)`. So memory reflects human judgment, not model
self-reinforcement (D13.3).

### Tests
- Deterministic (MockProvider): a scripted `explain_incident` that calls several tools then
  answers; a `crypto_scan` call with a `{path}` arg proving the loop parses + forwards tool
  arguments (untested in C0 — all C0 tools were parameterless).
- Live skip-graceful: `crypto_scan` against the local `qryx` binary on a real path (qryx
  needs no seeded data); `memory_*` and `quality_latest` skip gracefully when the Engram
  binary / `.engram` store / `verdryx.db` are absent (this box).

## C1-W2 — both shells (two Sonnet tracks)

- **Wire real `Clients`** at copilot bootstrap by REUSING each shell's existing env
  discovery (the same resolution the Money / Identity / Policy panels use for Cloud / Idryx
  / Wardryx; and Memory / Drills for Engram / Qryx where present). Build
  `genaryx_copilot::Clients` with those real clients instead of `Clients::default()`. A
  plane that does not resolve is simply left `None` (its tools are not advertised).
- **Explain affordance**: an "Explain with Felyx" control on the Incidents (and/or Money)
  panel that calls a new `copilot_explain(incident_id)` command / FFI method and opens the
  Copilot pane focused on that answer.

## Deferred (not C1)

- Core-`Store` event-timeline + delegation-graph tools (`run_timeline`, `delegation_slice`)
  need an `Arc<Mutex<genaryx_core::Store>>` shared from the shell's live ingest — a deeper
  plumbing pass. C1's root-cause is connector + memory backed, which already spans the money,
  policy, and identity planes.
- C2 (propose-and-confirm) and C3 (the relay triage tie) are unchanged from PHASE6.md.

## Gates (unchanged)
`cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`,
`cargo test --workspace`, Tauri `tsc` + `pnpm build` + `src-tauri cargo build`, SwiftUI
`build-ffi.sh` + `swift build`.
