#!/usr/bin/env bash
# Wires the SwiftUI shell to the real genaryx-core: builds the UniFFI chain
# and stages the output under apps/macos so `swift build` links against it.
#
#   Rust staticlib/cdylib -> uniffi-bindgen (library mode, Swift)
#   -> xcframework (static lib + header + modulemap) -> staged for SwiftPM
#
# Usage: bash apps/macos/build-ffi.sh    (self-anchored; run from anywhere)
#
# Mirrors crates/ffi/build-smoke.sh exactly (see crates/ffi/README.md,
# "Wiring apps/macos"), just staging the generated Swift and xcframework
# under apps/macos instead of crates/ffi/swift-smoke, and stopping short of
# building/running the app itself (that's `swift build` / `swift run`,
# separate steps). Idempotent: safe to re-run any time the Rust side
# changes; it wipes and regenerates both output directories each time.
set -euo pipefail

# Active developer dir on this box is the CLT; xcodebuild needs the full
# Xcode (docs/PHASE0.md toolchain facts). No sudo required this way.
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"

# Build the Rust objects for the same minimum macOS the Swift package
# declares, or every object in the staticlib draws an ld warning ("built for
# newer macOS version than being linked").
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"

APP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$APP_DIR/../.." && pwd)"
GEN_SWIFT="$APP_DIR/Sources/GenaryxCoreFFI/Generated"
STAGE="$APP_DIR/Binary"

echo "==> cargo build -p genaryx-ffi --release (staticlib + cdylib)"
cargo build -p genaryx-ffi --release --manifest-path "$ROOT/Cargo.toml"

echo "==> uniffi-bindgen generate --library (Swift)"
BINDGEN_OUT="$(mktemp -d "${TMPDIR:-/tmp}/genaryx-macos-bindgen.XXXXXX")"
trap 'rm -rf "$BINDGEN_OUT"' EXIT
cargo run -p genaryx-ffi --bin uniffi-bindgen --manifest-path "$ROOT/Cargo.toml" -- \
    generate --library "$ROOT/target/release/libgenaryx_ffi.dylib" \
    --language swift --out-dir "$BINDGEN_OUT"

echo "==> staging: .swift into the SwiftPM target, header+modulemap into the xcframework"
rm -rf "$GEN_SWIFT" "$STAGE"
mkdir -p "$GEN_SWIFT" "$STAGE/headers"
cp "$BINDGEN_OUT/genaryx_ffi.swift" "$GEN_SWIFT/genaryx_ffi.swift"
cp "$BINDGEN_OUT/genaryx_ffiFFI.h" "$STAGE/headers/"
# xcframeworks resolve the C module from a literal module.modulemap.
cp "$BINDGEN_OUT/genaryx_ffiFFI.modulemap" "$STAGE/headers/module.modulemap"

echo "==> xcodebuild -create-xcframework"
xcodebuild -create-xcframework \
    -library "$ROOT/target/release/libgenaryx_ffi.a" \
    -headers "$STAGE/headers" \
    -output "$STAGE/GenaryxFFI.xcframework"
rm -rf "$STAGE/headers"

echo "==> done: bindings in Sources/GenaryxCoreFFI/Generated, xcframework in Binary/GenaryxFFI.xcframework"
