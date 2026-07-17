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
| 1 | UniFFI boundary: Swift bindings, async event streams, XCFramework packaging | DONE | GO with change: the boundary is proven end to end, but live events cross it via a uniffi `callback_interface` (`EventListener`) pushed from a plain Rust thread that solely owns the non-`Sync` `IngestService` and drains its broadcast receiver with the synchronous `try_recv`, not via uniffi async streams; no async runtime exists on either side of the FFI. Everything else as planned: uniffi 0.32 proc-macro scaffolding (no UDL), project-pinned `uniffi-bindgen` bin in library mode, staticlib packaged by `xcodebuild -create-xcframework`, consumed as a SwiftPM binaryTarget. Evidence (`crates/ffi`, smoke `bash crates/ffi/build-smoke.sh`, run twice, exit 0): Swift constructed `FleetHandle`; `eventCount()` = 179 (full demo campaign, primed synchronously in the constructor); `recentEvents(limit: 5)` returned the 5 newest stored rows (ids 179..175, real qryx events); 3 live feeder events arrived through the callback within ~3s (live-run-001..003); `eventCount()` grew to 182 while Swift watched (second WAL reader connection sees the writer's commits). The same path runs as a Rust E2E test in CI (`cargo test -p genaryx-ffi`, ~2.4s, Linux-safe). Packaging gotchas and the id-on-push contract: see F-04. |
| 2 | Secure Enclave two ways (SwiftUI CryptoKit + Tauri security-framework), full pair → signed-ack vs local `tokenfuse-cloud` | TODO | — |
| 3 | SQLite ingest bench ≥ 50k NDJSON lines/min on M-series | DONE | GO: measured 6.8M-7.2M lines/min end-to-end (conform + Store insert), 25M-27M lines/min conform-only, target 50k/min; corpus 200,122 lines; see `crates/core/examples/ingest_bench.rs` |
| 4 | ML-DSA verify in Rust (crate choice vs `qryx verify-evidence` bridge) | DONE | GO: crate `ml-dsa` v0.1.1 (RustCrypto `signatures` monorepo, FIPS-204 final). Covers ML-DSA-44/65/87 via one generic `verify(param_set, public_key, message, signature) -> Result<bool, String>`; SPKI/PKCS8 parsing is built into the crate (matches Qryx's embedded-key format, 07 §4.5) with a raw-key fallback for bare offline-license keys. 10 tests green: round-trip KAT + tampered-message + tampered-signature + wrong-key + malformed-input for all three param sets, see `crates/signing/src/mldsa.rs`. `qryx` is not on this box's PATH so the qryx-signed-evidence bonus was skipped as instructed; ran an adjacent check instead since this box's OpenSSL 3.6.3 signs ML-DSA-65 natively: OpenSSL-signed message/SPKI verified `true` through our code, tampered message verified `false`, real cross-implementation evidence beyond a same-crate round trip. `fips204` (single-maintainer) was the other candidate; rejected for lacking any SPKI/PKCS8 support, which would have meant hand-rolling ASN.1 parsing ourselves. |
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

- **F-02 (2026-07-17).** `crates/core/examples/ingest_bench.rs`: conform-only
  runs 25M-27M lines/min; end-to-end (conform + build `ConsoleEvent` + `Store::insert_batch`,
  200,122-line corpus repeated from the 179-line demo output, 1000-line insert
  batches) runs 6.8M-7.2M lines/min on this box (Apple M1 Pro). Both clear the
  50k/min Phase-0 target by two to three orders of magnitude. SQLite insert is
  the dominant cost relative to conform alone (roughly 3.5x-4x slower per
  line), but still nowhere near the target boundary, so no schema or batching
  change is needed at Phase-0 scale.

- **F-03 (2026-07-17).** ML-DSA signature encoding is not "any bytes decode to
  some signature": `ml-dsa`'s `Signature::decode` range-checks the packed hint
  and `z` fields and returns a decode error for out-of-range bit patterns.
  Flipping a byte in the tail of a real signature (the encoded hint) often
  produces a decode `Err`, not a cryptographic `Ok(false)`; flipping a byte in
  the leading `c_tilde` commitment hash always decodes fine and fails the
  equality check, giving a clean `Ok(false)`. Both are fail-closed (06 §0.5:
  never a panic, never a silent accept) and both are covered by
  `crates/signing/src/mldsa.rs` tests, but a caller checking only for `Ok(false)`
  on tamper would miss the decode-error case; the console's evidence/license
  verification call sites should treat `Err` and `Ok(false)` identically (both
  mean "reject"), never treat `Err` as "inconclusive, allow".

- **F-04 (2026-07-17).** UniFFI boundary notes, from getting the spike-1 smoke
  green (`crates/ffi`). (1) Async never crosses the FFI: `IngestService` is
  `Send` but not `Sync`, so one plain thread owns it outright and forwards
  broadcast events to Swift through a `callback_interface` after each 150ms
  poll; `tokio::sync::broadcast::Receiver::try_recv` is synchronous, so no
  runtime, executor, or uniffi-async machinery exists anywhere in the chain.
  Callbacks run on the Rust ingest thread; the shell hops to `@MainActor`
  itself before touching UI state. (2) Push-path events carry `id = 0`:
  `ConsoleEvent` is broadcast before its rowid exists, so `UiEvent.id` is
  meaningful only on `recent_events` rows; shells key live rows themselves (a
  later core change could broadcast post-insert `StoredEvent`s if rowids are
  ever needed live). (3) Pin `MACOSX_DEPLOYMENT_TARGET=14.0` when building the
  staticlib: the host default (macOS 26.5 SDK) draws one ld warning per object
  file (~700 of them) when linked into the macOS-14 SwiftPM target; pinned,
  the link is warning-free. (4) Keeping the `uniffi-bindgen` bin inside
  `genaryx-ffi` (uniffi feature `cli`) guarantees generator/runtime
  version-lock but pulls bindgen-only deps (clap, askama, reqwest/rustls) into
  the lib's graph: `libgenaryx_ffi.a` lands at ~139 MB, dead-stripping to a
  ~20 MB linked smoke binary. Accepted for Phase 0; the remedy, if it ever
  hurts, is a separate `crates/uniffi-bindgen` bin crate so the lib drops the
  `cli` feature.

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
