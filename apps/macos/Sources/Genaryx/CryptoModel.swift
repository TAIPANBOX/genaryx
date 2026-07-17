import Foundation
import GenaryxCoreFFI
import Observation

/// Whole-panel connection state for the Crypto surface - honest, distinct
/// states the views render directly rather than inferring from a read's
/// error shape. Mirrors `QualityConnection`/`IdryxConnection`: no network
/// call anywhere in this handle either (Qryx is a local subprocess -
/// `crates/ffi/src/crypto/mod.rs`'s module doc), so `.connectFailed` here can
/// only mean a local binary/spawn problem.
enum CryptoConnection: Equatable {
    case connecting
    case noEnvironment
    case connectFailed(reason: String)
    case ready(source: CryptoEnvSource, qryxBin: String)

    var isReady: Bool {
        if case .ready = self { return true }
        return false
    }
}

/// Which evidence bundle the Evidence section's scope toggle requests:
/// `scan_evidence` (the whole scanned tree) or `agents_evidence` (scoped to
/// the agent-governance stack's own trust surface - docs/PHASE4.md W1
/// grounding, `qryx agents --format evidence`). Both return the exact same
/// `EvidenceReportRecord` shape, so the view renders identically either way.
enum EvidenceScope: String, CaseIterable, Identifiable {
    case repository
    case agentStack

    var id: String { rawValue }

    var label: String {
        switch self {
        case .repository: return "repository"
        case .agentStack: return "agent stack"
        }
    }
}

/// Live Crypto state for the SwiftUI shell: owns a `CryptoHandle`
/// (constructed once at `connect()`) and runs on-demand scans through it.
/// Read-only in the mutation sense (no state this console owns is ever
/// changed - a scan only reads the target filesystem), but unlike
/// Quality/Identity, EVERY read here is an explicit operator action, never
/// auto-refreshed: `crates/ffi/src/crypto/mod.rs`'s own doc explains why
/// (qryx walks a filesystem tree - genuinely expensive). So, unlike
/// `QualityView`/`IdentityView`, `CryptoView` runs no periodic `.task`
/// refresh loop at all.
///
/// `CryptoHandle`'s exported methods are synchronous and can block for a
/// real qryx subprocess run. Every call into the handle below therefore runs
/// inside `Task.detached`, off this model's `@MainActor` isolation, exactly
/// like `QualityModel`/`IdentityModel` - see `PolicyModel.swift`'s own doc
/// for the full rationale. `CryptoHandle` is generated as `@unchecked
/// Sendable`, so capturing it into a detached task's closure is safe.
@MainActor
@Observable
final class CryptoModel {
    private(set) var connection: CryptoConnection = .connecting

    /// The operator-editable scan target - pre-filled from
    /// `CryptoHandle.defaultScanTarget()` once connected, but never enforced
    /// (docs/PHASE4.md W1: "operator can see/set it").
    var scanTarget: String = ""
    /// The Evidence section's scope toggle - see `EvidenceScope`'s own doc.
    /// `CryptoView` reacts to a change here with `.onChange` (calling
    /// `refreshEvidence()`), rather than a `didSet` observer on this
    /// `@Observable` property - keeps this class's state changes plain
    /// assignments throughout, with no side-effecting property observer
    /// tucked inside the Observation macro's synthesized storage.
    var evidenceScope: EvidenceScope = .repository
    /// The operator-entered path to an evidence report file for the Verify
    /// action - never pre-filled (there is no honest default: it names a
    /// file the operator or a later Evidence Center build produced
    /// elsewhere).
    var verifyFilePath: String = ""

    private(set) var ncscReport: NcscReportRecord?
    /// Raw CBOM JSON from `scan_cbom` - `CbomParser` (`CryptoComponents.swift`)
    /// decodes it for the inventory table. Kept as the raw string here
    /// (matches `dto`'s own module doc on why CBOM crosses FFI untyped),
    /// not a partially-typed Swift model.
    private(set) var cbomJson: String?
    private(set) var evidenceReport: EvidenceReportRecord?
    private(set) var verifyOutcome: VerifyOutcomeRecord?

    /// When the most recent scan completed - the panel's "as of last scan"
    /// label (docs/PHASE4.md W1: Qryx is on-demand, never implied live).
    /// Updated when ANY of the scan's three reads succeeds (docs/PHASE4.md
    /// W1 phrases this as one scan action, not three independently-timed
    /// ones); a field that itself failed keeps showing its last known-good
    /// value rather than going blank, mirroring `MenuBarLabel.burnText`'s own
    /// "absent beats wrong, but last-known-good beats absent" precedent.
    private(set) var lastScanAt: Date?

    private(set) var bannerMessage: String?
    private(set) var isScanning = false
    private(set) var isVerifying = false

    private var handle: CryptoHandle?

    init() {
        Task { await self.connect() }
    }

    // MARK: - connect

    /// (Re)resolve an environment and build a fresh handle. Called once from
    /// `init()`; also reachable from a "retry" affordance in the empty
    /// state. Deliberately does NOT run a scan itself - see the type doc.
    func connect() async {
        connection = .connecting
        bannerMessage = nil
        handle = nil

        do {
            let newHandle = try await Task.detached { try CryptoHandle.discover() }.value
            handle = newHandle
            connection = .ready(source: newHandle.source(), qryxBin: newHandle.qryxBin())
            if scanTarget.trimmingCharacters(in: .whitespaces).isEmpty {
                scanTarget = await Task.detached { newHandle.defaultScanTarget() }.value
            }
        } catch {
            handle = nil
            connection = Self.connectionFailure(from: error)
        }
    }

    private static func connectionFailure(from error: Error) -> CryptoConnection {
        guard let cryptoError = error as? CryptoError else {
            return .connectFailed(reason: String(describing: error))
        }
        switch cryptoError {
        case .NoEnvironment:
            return .noEnvironment
        case .Spawn(let bin, let reason):
            return .connectFailed(reason: "\(bin): \(reason)")
        case .Cli(let code, let stderr):
            // Not expected during connect (no scan has happened yet -
            // `CryptoHandle.discover`/`connect` never run qryx at
            // construction), but handled honestly rather than assumed
            // impossible - mirrors `PolicyModel.connectionFailure`'s own
            // defensive default arm.
            return .connectFailed(reason: "qryx exited \(code): \(stderr)")
        case .Json(let reason):
            return .connectFailed(reason: reason)
        }
    }

    // MARK: - on-demand scan (never auto-refreshed - see the type doc)

    /// Run all three scan reads (NCSC timeline, CBOM inventory, evidence)
    /// against `scanTarget` concurrently. Only ever called in direct
    /// response to the operator pressing Scan. Each read's success/failure
    /// is independent: one failing does not roll back the other two, so a
    /// partial result still renders whatever DID come back.
    @discardableResult
    func runScan() async -> Bool {
        guard let handle else { return false }
        let target = scanTarget.trimmingCharacters(in: .whitespaces)
        guard !target.isEmpty else {
            bannerMessage = "Enter a scan target path first."
            return false
        }
        isScanning = true
        defer { isScanning = false }
        bannerMessage = nil

        async let ncscLoad = Task.detached { try handle.scanNcsc(target: target) }.value
        async let cbomLoad = Task.detached { try handle.scanCbom(target: target) }.value
        async let evidenceLoad = Self.loadEvidence(handle: handle, target: target, scope: evidenceScope)

        var anySucceeded = false

        do {
            ncscReport = try await ncscLoad
            anySucceeded = true
        } catch {
            present(error)
        }
        do {
            cbomJson = try await cbomLoad
            anySucceeded = true
        } catch {
            present(error)
        }
        do {
            evidenceReport = try await evidenceLoad
            anySucceeded = true
        } catch {
            present(error)
        }

        if anySucceeded {
            lastScanAt = Date()
        }
        return anySucceeded
    }

    /// Re-run just the evidence read for the current `evidenceScope` (fired
    /// automatically when the scope toggle changes, after an initial scan
    /// has already run) without re-running the NCSC/CBOM scans too.
    @discardableResult
    func refreshEvidence() async -> Bool {
        guard let handle else { return false }
        let target = scanTarget.trimmingCharacters(in: .whitespaces)
        guard !target.isEmpty else { return false }
        do {
            evidenceReport = try await Self.loadEvidence(handle: handle, target: target, scope: evidenceScope)
            lastScanAt = Date()
            return true
        } catch {
            present(error)
            return false
        }
    }

    private nonisolated static func loadEvidence(
        handle: CryptoHandle, target: String, scope: EvidenceScope
    ) async throws -> EvidenceReportRecord {
        try await Task.detached {
            switch scope {
            case .repository:
                return try handle.scanEvidence(target: target, signKey: nil)
            case .agentStack:
                return try handle.agentsEvidence(target: target)
            }
        }.value
    }

    // MARK: - verify

    /// `qryx verify-evidence <file>` against `verifyFilePath`. `verified ==
    /// false` is a real, successfully-obtained answer (see
    /// `VerifyOutcomeRecord`'s own doc) - only a spawn/parse failure sets
    /// `bannerMessage` instead of `verifyOutcome`.
    @discardableResult
    func verifyEvidence() async -> Bool {
        guard let handle else { return false }
        let file = verifyFilePath.trimmingCharacters(in: .whitespaces)
        guard !file.isEmpty else {
            bannerMessage = "Enter a path to an evidence report file first."
            return false
        }
        isVerifying = true
        defer { isVerifying = false }
        do {
            verifyOutcome = try await Task.detached { try handle.verifyEvidence(file: file) }.value
            return true
        } catch {
            present(error)
            return false
        }
    }

    // MARK: - error presentation

    /// Fold any thrown error into the plain banner. Mirrors
    /// `QualityModel.present`.
    private func present(_ error: Error) {
        guard let cryptoError = error as? CryptoError else {
            bannerMessage = String(describing: error)
            return
        }
        switch cryptoError {
        case .NoEnvironment:
            connection = .noEnvironment
        case .Spawn(let bin, let reason):
            bannerMessage = "Could not run qryx at \(bin): \(reason)"
        case .Cli(let code, let stderr):
            bannerMessage = "qryx exited \(code): \(stderr)"
        case .Json(let reason):
            bannerMessage = "Could not parse qryx output: \(reason)"
        }
    }
}
