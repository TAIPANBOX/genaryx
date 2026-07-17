// swift-tools-version: 6.0
// The SwiftUI shell over genaryx-core, wired through the UniFFI bridge in
// crates/ffi. `GenaryxCoreFFI` (the generated Swift bindings) and the
// `genaryx_ffiFFI` binary target (the xcframework) are both produced by
// build-ffi.sh, which must run at least once before `swift build`; see
// README.md and crates/ffi/README.md ("Wiring apps/macos").
import PackageDescription

let package = Package(
    name: "Genaryx",
    platforms: [
        .macOS(.v14)
    ],
    targets: [
        .executableTarget(
            name: "Genaryx",
            dependencies: ["GenaryxCoreFFI"]
        ),
        // The generated bindings, shaped exactly as crates/ffi/swift-smoke
        // proved out: one Swift target over one binary target. Regenerated
        // by build-ffi.sh into Sources/GenaryxCoreFFI/Generated/ (gitignored).
        .target(
            name: "GenaryxCoreFFI",
            dependencies: ["genaryx_ffiFFI"]
        ),
        // Static libgenaryx_ffi.a + genaryx_ffiFFI.h + module.modulemap,
        // packaged into Binary/GenaryxFFI.xcframework by build-ffi.sh
        // (gitignored).
        .binaryTarget(
            name: "genaryx_ffiFFI",
            path: "Binary/GenaryxFFI.xcframework"
        ),
    ]
)
