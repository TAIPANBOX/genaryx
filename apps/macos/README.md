# Genaryx (macOS shell)

The native macOS SwiftUI shell for Genaryx (decision D2): a SwiftPM package with an
executable target, no Xcode project required for local builds.

## Build and run

Full Xcode is required (the active command line tools alone are not enough for the
macOS SDK / SwiftUI toolchain used here):

```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
cd apps/macos
swift build
swift run
```

`swift build` compiles headlessly and is what CI should run. `swift run` additionally
launches the app (a window plus a menu-bar item); only do that locally, not in a
headless environment.

## Status: mock data

`BusExplorerView` currently renders `MockData.events`, ~40 hand-written `UiEvent`
values shaped like a real `taipan demo` campaign (see `crates/core/src/demo.rs`),
not live data. There is no Rust, Cargo, or UniFFI dependency yet.

The follow-up task wires this shell to the real core: generate a UniFFI binding for
`genaryx-core`, replace `UiEvent`/`MockData` with the generated `StoredEvent` type,
and feed `BusExplorerView` from `IngestService::subscribe()`'s broadcast stream
instead of the static mock array. See the doc comment on `UiEvent` in
`Sources/Genaryx/UiEvent.swift` for the exact bridge point.

## Design language

Colors, type, and corner-radius tokens are pulled from the it-rat2 web shell's
design language (`Sources/Genaryx/Theme.swift` has the full mapping back to
`site.css`'s custom properties). Native macOS patterns take priority over pixel
parity with the web shell: this is a native list plus menu-bar app that carries the
same color language and information architecture, not an embedded web view.
