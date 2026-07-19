# Phase 6 · C1 (Felyx triage + explanation) — RESULTS

Status: **DONE (both shells), all gates green**, 2026-07-19. Contract:
[PHASE6-C1.md](PHASE6-C1.md). Builds on [C0](PHASE6-C0-RESULTS.md).

C1 = "triage + explanation": the sync-connector tool bridge, memory recall, a cross-plane
`explain_incident` flow, and (deferred from C0) wiring the real connector `Clients` into
both shells so Felyx has tools against the live planes.

## W1 — crate (orchestrator-owned)

- **Sync-tool bridge**: `Tool::run` stays async; sync-backed tools do their blocking work in
  `tokio::task::spawn_blocking`. Engram lives behind `Arc<Mutex<EngramClient>>` (it is
  `&mut self` + a long-lived stdio child); Verdryx opens its `rusqlite` connection fresh per
  call (the connection is `!Sync`); Qryx shells its CLI fresh per call. `Clients` grew
  `engram`, `qryx_bin`, `verdryx_db`.
- **New tools**: `memory_recall(query, limit?)` + `memory_why(memory_id)` (Engram),
  `quality_latest` (Verdryx latest run + rollup), and `crypto_scan(path)` (Qryx NCSC
  PQC-readiness) — the **first parameterized tool**, which exercised the loop's
  argument-parsing/forwarding path (untested in C0, where every tool was parameterless). A
  bad `path` returns error-as-data so the model can correct it, not a failed answer.
- **`CopilotService::explain_incident(incident_id)`**: a focused `ask` that seeds the loop to
  gather the money (alerts/runs), identity (identity_alerts), and policy evidence plus a
  memory recall, and produce a root-cause chain with cited row ids — the D13.4 example chain
  ("spend spike → wardryx hold → idryx unattested"), which is entirely connector-backed.
- **Memory is read-only in C1**: recall + why (the autonomous intelligence). The write-back
  (recording a human "false alarm" ruling) is **deferred** — Engram's `remember` is not
  wrapped on the client and its exact tool schema isn't grounded here; it is a human-gated
  action that fits naturally alongside C2's human-in-loop work. Noted honestly, not silently
  dropped.

### W1 tests (deterministic + live)
- 35 crate unit tests + the `no_signer` guard. New for C1: the sync-tool registry
  registration, `crypto_scan` bad-args / error-as-data / unavailable paths, the loop
  forwarding a parameterized `{path}` argument end to end, and `explain_incident` inheriting
  the disabled-service `NoProvider` behavior.
- **Live**: `crypto_scan` ran against the real `qryx` binary (`~/.taipan/bin/qryx`) and
  scanned the crate dir successfully — the sync-tool bridge works end to end with a real
  sync connector, not just a mock. (Engram / Verdryx tools skip gracefully where their
  binary / db is absent.)

## W2 — both shells wire real `Clients` + an Explain affordance (two Sonnet tracks)

Deferred from C0 (which passed `Clients::default()`): each shell now resolves the real
connector clients at copilot bootstrap by REUSING that plane's existing `env::discover()`,
fail-soft per plane (a plane that does not resolve, or whose client errors, just drops that
plane's tools; it never fails bootstrap).

- **Tauri** (`apps/desktop`): `resolve_clients()` wires cloud/idryx/wardryx (from
  money/identity/policy `env`) + qryx_bin + verdryx_db + **engram** (a second `engram-mcp`
  child via `spawn_blocking`, reusing the memory panel's discovery). `copilot_explain`
  command; an "Explain" button on the Money incidents feed that deep-links into the Copilot
  pane (the same closure idiom as the existing Replay deep-link).
- **SwiftUI** (`crates/ffi` + `apps/macos`): `build_clients()` wires cloud/idryx/wardryx +
  qryx_bin + verdryx_db; **engram left `None`** deliberately (its only constructor spawns a
  child, wasteful to do eagerly at launch for a still-disabled service — a lazy-construction
  follow-up is noted). `CopilotHandle::explain()` + an "Explain with Felyx" button on the
  Money incidents rail deep-linking into the Copilot tab.

The one asymmetry (Tauri spawns Engram, SwiftUI defers it) is a deliberate, documented
per-shell judgment; both are valid under the contract's "engram is optional".

## Exit gate (C1)

All gates green, re-run by the orchestrator on the integrated tree: `cargo fmt --check`,
`cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace` (25
test-suites), Tauri `tsc` + `pnpm build` + `src-tauri cargo build`, SwiftUI `build-ffi.sh` +
`swift build`. The sync bridge is proven live (real qryx scan); the parameterized-tool
argument path and the explain flow are proven deterministically.

The copilot still defaults to `provider = "none"` on this box (no local model), so the panes
show the honest disabled state; the C1 tools are now WIRED (real connectors resolved in both
shells) and will light up the moment a provider is configured. A live LLM demo needs either
a local Ollama install or a BYO cloud key (a spend opt-in).

## Deferred (post-C1)

- The false-alarm memory WRITE-back (`memory_remember`) — needs Engram's `remember` schema +
  a human-gated action; fits with C2.
- Core-`Store` event-timeline / delegation-graph tools — need an `Arc<Mutex<Store>>` shared
  from the shell's live ingest.
- SwiftUI Engram wiring via lazy/shared construction (today `None`).
- C2 (propose-and-confirm) and C3 (the relay triage tie), unchanged from PHASE6.md.
