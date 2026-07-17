# Genaryx (macOS shell)

The native macOS SwiftUI shell for Genaryx (decision D2): a SwiftPM package with an
executable target, no Xcode project required for local builds.

## Build and run

Full Xcode is required (the active command line tools alone are not enough for the
macOS SDK / SwiftUI toolchain used here):

```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
cd apps/macos
bash build-ffi.sh
swift build
swift run
```

`build-ffi.sh` builds `genaryx-ffi` (release, pinned to `MACOSX_DEPLOYMENT_TARGET=14.0`),
runs the project-pinned `uniffi-bindgen` in library mode, and packages the result into
an xcframework, staging the generated Swift under
`Sources/GenaryxCoreFFI/Generated/` and the xcframework under `Binary/`. Both are
gitignored; run the script again any time the Rust side changes (idempotent, wipes and
regenerates each time). `swift build` will fail until it has run at least once on a
fresh checkout.

`swift build` compiles headlessly and is what CI should run (after `build-ffi.sh`).
`swift run` additionally launches the app (a window plus a menu-bar item); only do
that locally, not in a headless environment.

## Status: live core

`BusExplorerView` renders `FleetModel.events`, fed from the real `genaryx-core`
through the UniFFI bridge in `crates/ffi` (see `crates/ffi/README.md`). The mock data
(`MockData.swift`, `UiEvent.swift`) is gone: `UiEvent` is now the generated type from
`GenaryxCoreFFI`.

`FleetModel` (`Sources/Genaryx/FleetModel.swift`) constructs a `FleetHandle`, which
seeds a throwaway demo world and runs its own ingest + feeder threads (~179 events
primed, then roughly one live line per second); the model seeds `events` from
`recentEvents(limit:)` and grows it from `EventListener` pushes, hopping to
`@MainActor` before touching state. If `FleetHandle()` fails to construct, the shell
still launches with an empty list and a visible "core unavailable" note instead of
crashing (fail-closed).

## Design language

Colors, type, and corner-radius tokens are pulled from the it-rat2 web shell's
design language (`Sources/Genaryx/Theme.swift` has the full mapping back to
`site.css`'s custom properties). Native macOS patterns take priority over pixel
parity with the web shell: this is a native list plus menu-bar app that carries the
same color language and information architecture, not an embedded web view.
