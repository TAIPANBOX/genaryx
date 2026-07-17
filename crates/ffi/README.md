# genaryx-ffi

The UniFFI boundary between `genaryx-core` and the SwiftUI shell. Phase-0
spike 1; verdict and evidence live in `docs/PHASE0.md` (spike row 1, finding
F-04).

## Surface

- `UiEvent` (Record): flat mirror of the UI-relevant `StoredEvent` fields.
  Swift sees camelCase (`eventType`, `agentId`, `onBehalfOf`). `id` is the
  SQLite rowid for stored rows and 0 for live-pushed rows (the broadcast path
  precedes insert), so view models key live rows themselves.
- `FleetHandle` (Object): the constructor seeds a throwaway demo world in a
  temp dir (`demo::generate`, six tailed NDJSON files, WAL store), primes the
  full campaign synchronously, then runs two plain threads: ingest (sole
  owner of the `Send`-not-`Sync` `IngestService`) and a feeder appending one
  conforming line per second. `recent_events(limit)` and `event_count()` read
  through a second WAL connection; `subscribe(listener)` registers a live
  callback.
- `EventListener` (callback interface): `on_event(UiEvent)`, invoked from the
  Rust ingest thread after each poll cycle drains the core broadcast channel
  via synchronous `try_recv`. No async runtime exists on either side of the
  boundary.
- `FfiError` (Error): one message-carrying `Core` variant; nothing panics
  across the FFI.

## Smoke (proves the boundary end to end)

```sh
bash crates/ffi/build-smoke.sh
```

Chain: `cargo build --release` (staticlib + cdylib, with
`MACOSX_DEPLOYMENT_TARGET=14.0` to match the shell's minimum) -> project-
pinned `uniffi-bindgen` in library mode (Swift) -> `xcodebuild
-create-xcframework` (static lib + header + `module.modulemap`) ->
`swift run Smoke` in `swift-smoke/`. The smoke exits 0 only if it read 5
recent stored events, received at least one live push through the callback,
and observed the store grow while watching. Generated Swift, the xcframework,
and `.build/` are gitignored; the script regenerates everything.

## Wiring apps/macos (the follow-up, not this spike)

1. Run the same build/bindgen/xcframework steps (scripted, or a small
   Makefile target) with the output staged under `apps/macos`.
2. Add the two targets from `swift-smoke/Package.swift` to
   `apps/macos/Package.swift`: a `GenaryxCoreFFI` Swift target over a
   `genaryx_ffiFFI` binaryTarget, and make the app target depend on it.
3. Delete `MockData.swift` and `UiEvent.swift`; the generated `UiEvent`
   record replaces the hand-written struct (rename `type_` uses to
   `eventType`, `agentId` etc. are already aligned). `BusExplorerView` keeps
   its shape: seed with `recent_events`, then apply `EventListener` pushes,
   hopping to `@MainActor` before touching state.

## Known trade-off (accepted for the spike)

The `uniffi-bindgen` bin lives in this crate (`features = ["cli"]`) so the
generator can never version-skew from the runtime; the cost is that cli-only
dependencies join the lib's dependency graph and fatten `libgenaryx_ffi.a`
(~139 MB; the final linked smoke binary dead-strips to ~20 MB). If that ever
hurts, split a `crates/uniffi-bindgen` bin crate (one more workspace member)
and drop the `cli` feature here.
