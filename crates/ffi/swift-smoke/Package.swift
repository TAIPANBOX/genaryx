// swift-tools-version: 6.0
// Phase-0 spike 1 smoke package. Not the app: apps/macos stays untouched by
// the spike; this package exists to prove the Swift <- UniFFI <- genaryx-core
// path end to end. Build and run it via ../build-smoke.sh, which generates
// Sources/GenaryxCoreFFI/Generated/ and Binary/GenaryxFFI.xcframework first
// (both are gitignored; a bare `swift run` on a fresh checkout will fail
// until the script has produced them).
import PackageDescription

let package = Package(
    name: "GenaryxFFISmoke",
    platforms: [
        .macOS(.v14)
    ],
    targets: [
        .executableTarget(
            name: "Smoke",
            dependencies: ["GenaryxCoreFFI"]
        ),
        // The generated bindings, shaped exactly as apps/macos would consume
        // them in the follow-up: one Swift target over one binary target.
        .target(
            name: "GenaryxCoreFFI",
            dependencies: ["genaryx_ffiFFI"]
        ),
        // Static libgenaryx_ffi.a + genaryx_ffiFFI.h + module.modulemap,
        // packaged by xcodebuild -create-xcframework in build-smoke.sh.
        .binaryTarget(
            name: "genaryx_ffiFFI",
            path: "Binary/GenaryxFFI.xcframework"
        ),
    ]
)
