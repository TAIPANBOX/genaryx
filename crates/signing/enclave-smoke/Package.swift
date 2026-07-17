// swift-tools-version:5.9
// Phase-0 spike #2 harness (06 §7): the SwiftUI-shell twin of
// crates/signing/src/enclave.rs - CryptoKit SecureEnclave.P256 signing the
// same device-pairing protocol. Deliberately NOT part of apps/macos: this is
// spike evidence tooling, not shipped shell code.
//
// Build:  DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer swift build
// Run:    .build/debug/enclave-smoke [--vector | --emit-json | --cloud URL]
import PackageDescription

let package = Package(
    name: "enclave-smoke",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(name: "enclave-smoke", path: "Sources/EnclaveSmoke")
    ]
)
