# Phase 0 — skeleton and spikes

Source: `itrat-console/09-roadmap-and-process.md` (Ф0). Estimate: 2 sessions.

**Exit gate.** Both apps open and show the same live event stream from the shared
core; all six spikes have written verdicts; parity checklist v0 is enforced in CI;
golden NDJSON fixtures + an ingest bench report are committed.

## Scope

- [x] Monorepo skeleton: Rust workspace (`genaryx-core`, `connectors`, `signing`).
      `apps/*` dirs are reserved; shells land with the delegated tracks.
- [x] `genaryx-core` heart: agent-event envelope types + conform validation
      (draft 2020-12, embedded byte-exact v0.1/v0.2 schemas) + golden tests.
      *(done this session: 12 tests green, fmt clean, clippy `-D warnings` clean.)*
- [ ] Store (SQLite WAL) + batched writer. → Sonnet
- [ ] IngestService: FileTail → conform → Store → live broadcast. → Sonnet
- [ ] `taipan demo` generator (real campaign shapes). → Sonnet
- [ ] Tauri shell: virtualized Bus Explorer live list. → Sonnet (Web track)
- [ ] SwiftUI shell: UniFFI bridge + live list + menu-bar stub. → Sonnet (SwiftUI track)
- [ ] CI: fmt/clippy/test + both-shell smoke + parity checklist v0.

## Spike log (06 §7) — verdicts required before scale work

| # | Spike | Status | Verdict |
|---|-------|--------|---------|
| 1 | UniFFI boundary: Swift bindings, async event streams, XCFramework packaging | TODO | — |
| 2 | Secure Enclave two ways (SwiftUI CryptoKit + Tauri security-framework), full pair → signed-ack vs local `tokenfuse-cloud` | TODO | — |
| 3 | SQLite ingest bench ≥ 50k NDJSON lines/min on M-series | TODO | — |
| 4 | ML-DSA verify in Rust (crate choice vs `qryx verify-evidence` bridge) | TODO | — |
| 5 | Both-shell headless smoke in CI (tauri-driver + xcodebuild/XCUITest) | TODO | — |
| 6 | SSE client vs Cloud `/v1/stream` under reconnect / chunk splits | TODO | — |

Verdict = one of {GO as-planned, GO with change, FALLBACK to <plan B>}, with the
evidence (bench numbers, a working signed ack, a passing smoke run) linked.

## Findings (real, from building against live data)

- **F-01 (2026-07-16).** The `aws-comparable-176` benchmark campaign emitted all
  12 events with `agent_id: "aws-comparable-agent"` — no `agent://` prefix — so
  every line is non-conforming to the envelope. Cause: the bus emission path is
  fail-open (07 §3), so a benchmark harness with loose ids produced invalid events
  that no service rejected. The conformer catches all 12; this is precisely the
  Posture "schema conformance" check (08 §2). Kept as a regression fixture
  (`campaign-aws-176.ndjson`) with a test asserting it is caught. No stack change
  needed; it validates the console's value on real data.

## Toolchain facts (verified 2026-07-16, box "factory")

- Rust 1.96.1 (aarch64-apple-darwin only; add windows/linux targets in later phases).
- Node 26.5 + npm 11.17 + pnpm 11.12.
- Xcode 26.6 present at `/Applications/Xcode.app` (macOS 26.5 SDK, license accepted,
  first-launch OK). Active dir is CLT; build the SwiftUI shell with
  `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer` (no sudo needed).
- Missing, install when the shells land: `tauri-cli` (or `@tauri-apps/cli` devDep),
  UniFFI via a project `uniffi-bindgen` bin (version-matched to the crate).
- `gh` authenticated as TAIPANBOX (repo + workflow scope). Repo `TAIPANBOX/genaryx`
  is private and was empty; `TAIPANBOX/taipan` does not exist yet (created in F1).

## Delegation plan (implementation = Sonnet 5, explicit `model: sonnet`)

The core skeleton compiles first (this session). Then, minimizing write conflicts:

1. **Store** (Sonnet) — self-contained in `core/src/store.rs`, owns its tables.
2. **Ingest** (Sonnet) — `core/src/ingest.rs`, depends on Store + conform.
3. **demo** (Sonnet) — `core/src/demo.rs`, writes NDJSON the console tails.
4. Then two **parallel** shell tracks (Sonnet Web + Sonnet SwiftUI) from one core spec.

Each spec is self-contained: repo conventions, file:line insertion points, exact
verification commands, and "do not push". Verification must match CI.
