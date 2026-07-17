import Foundation
import GenaryxCoreFFI
import Observation

/// Whole-panel connection state for the Drills surface - honest, distinct
/// states the views render directly rather than inferring from a read's
/// error shape. Mirrors `CryptoConnection`: no subprocess spawn anywhere in
/// this handle's construction either (`MockryxClient` only resolves a binary
/// path, exactly like `QryxClient` - `crates/ffi/src/drills/mod.rs`'s own
/// module doc), so `.connectFailed` here can only mean a local
/// runtime/resource problem, never a real mockryx run gone wrong (a bad
/// `run()` call surfaces through `bannerMessage` instead, never this
/// whole-panel state).
enum DrillsConnection: Equatable {
    case connecting
    case noEnvironment
    case connectFailed(reason: String)
    case ready(source: DrillsEnvSource, mockryxBin: String)

    var isReady: Bool {
        if case .ready = self { return true }
        return false
    }
}

/// Live Drills state for the SwiftUI shell: owns a `DrillsHandle`
/// (constructed once at `connect()`) and runs on-demand drills through it.
/// Every read here shells out to `mockryx run`, which sends real adversarial
/// traffic at a live TokenFuse gateway - genuinely consequential, so, exactly
/// like `CryptoModel`, this NEVER auto-refreshes on a timer; the Swift
/// `DrillsView` runs no periodic `.task` refresh loop for `run()` (only
/// `connect()` itself runs once).
///
/// `DrillsHandle`'s exported methods are synchronous and can block for a
/// real mockryx subprocess run. Every call into the handle below therefore
/// runs inside `Task.detached`, off this model's `@MainActor` isolation,
/// exactly like `CryptoModel`/`QualityModel` - see `PolicyModel.swift`'s own
/// doc for the full rationale. `DrillsHandle` is generated as `@unchecked
/// Sendable`, so capturing it into a detached task's closure is safe.
@MainActor
@Observable
final class DrillsModel {
    private(set) var connection: DrillsConnection = .connecting

    /// The operator-editable scenario directory - pre-filled from
    /// `DrillsHandle.defaultScenarioDir()` once connected, but never enforced
    /// (mirrors `CryptoModel.scanTarget`'s own "operator can see/set it").
    var scenarioDir: String = ""
    /// The TokenFuse gateway to rehearse against - pre-filled from
    /// `DrillsHandle.defaultGateway()` when one resolved, otherwise left
    /// blank (a real, honest "nothing configured" starting point - see
    /// `crates/ffi/src/drills/env.rs`'s own module doc).
    var gateway: String = ""
    var apiKey: String = ""
    /// docs/PHASE4.md W2: "a fail-on-skip toggle".
    var failOnSkip: Bool = false

    private(set) var report: DrillReportRecord?
    private(set) var bannerMessage: String?
    private(set) var isRunning = false

    private var handle: DrillsHandle?

    init() {
        Task { await self.connect() }
    }

    // MARK: - connect

    /// (Re)resolve an environment and build a fresh handle. Called once from
    /// `init()`; also reachable from a "retry" affordance in the empty
    /// state. Deliberately does NOT run a drill itself - see the type doc.
    /// Best-effort loads the last SAVED report (`DrillsHandle.loadReport`
    /// against `defaultSavePath()`) so a freshly-launched console still shows
    /// an "as of last run" view from a previous session - reading a past
    /// artifact off disk is not "running a drill", so this does not violate
    /// "never auto-run" (`crates/ffi/src/drills/mod.rs`'s own module doc).
    func connect() async {
        connection = .connecting
        bannerMessage = nil
        report = nil
        handle = nil

        do {
            let newHandle = try await Task.detached { try DrillsHandle.discover() }.value
            handle = newHandle
            connection = .ready(source: newHandle.source(), mockryxBin: newHandle.mockryxBin())
            if scenarioDir.trimmingCharacters(in: .whitespaces).isEmpty {
                scenarioDir = await Task.detached { newHandle.defaultScenarioDir() }.value
            }
            if gateway.trimmingCharacters(in: .whitespaces).isEmpty {
                gateway = await Task.detached { newHandle.defaultGateway() }.value ?? ""
            }
            if apiKey.isEmpty {
                apiKey = await Task.detached { newHandle.defaultApiKey() }.value ?? ""
            }
            await loadLastSavedReport()
        } catch {
            handle = nil
            connection = Self.connectionFailure(from: error)
        }
    }

    private static func connectionFailure(from error: Error) -> DrillsConnection {
        guard let drillsError = error as? DrillsError else {
            return .connectFailed(reason: String(describing: error))
        }
        switch drillsError {
        case .NoEnvironment:
            return .noEnvironment
        case .Spawn, .Cli, .Json, .Read:
            // Not expected during connect (no run/load has happened yet -
            // `DrillsHandle.discover`/`connect` never spawn a subprocess),
            // but handled honestly rather than assumed impossible - mirrors
            // `PolicyModel.connectionFailure`'s own defensive default arm.
            return .connectFailed(reason: String(describing: drillsError))
        }
    }

    /// Best-effort: a missing save file (the common case - no drill has ever
    /// been run/saved yet) is a normal empty state, never a banner error.
    private func loadLastSavedReport() async {
        guard let handle else { return }
        let path = await Task.detached { handle.defaultSavePath() }.value
        report = try? await Task.detached { try handle.loadReport(path: path) }.value
    }

    // MARK: - on-demand run (never auto-refreshed - see the type doc)

    /// `mockryx run ...` against the current `scenarioDir`/`gateway`/`apiKey`/
    /// `failOnSkip`, saved to `DrillsHandle.defaultSavePath()` so the "last
    /// run" view survives a future restart. Only ever called in direct
    /// response to the operator pressing "Run drills" (docs/PHASE4.md W2:
    /// "on demand... never auto-run").
    @discardableResult
    func run() async -> Bool {
        guard let handle else { return false }
        let scenarioDir = scenarioDir.trimmingCharacters(in: .whitespaces)
        let gateway = gateway.trimmingCharacters(in: .whitespaces)
        guard !scenarioDir.isEmpty else {
            bannerMessage = "Enter a scenario directory first."
            return false
        }
        guard !gateway.isEmpty else {
            bannerMessage = "Enter a TokenFuse gateway URL first."
            return false
        }
        let apiKeyValue = apiKey.trimmingCharacters(in: .whitespaces)
        // Captured as a local `let` (like `scenarioDir`/`gateway`/`apiKeyValue`
        // above) BEFORE entering `Task.detached`: `self` is `@MainActor`-isolated,
        // so `self.failOnSkip` cannot be read from inside the detached closure.
        let failOnSkip = self.failOnSkip
        isRunning = true
        defer { isRunning = false }
        bannerMessage = nil
        do {
            let savePath = await Task.detached { handle.defaultSavePath() }.value
            report = try await Task.detached {
                try handle.run(
                    scenarioDir: scenarioDir, gateway: gateway, apiKey: apiKeyValue.isEmpty ? nil : apiKeyValue,
                    failOnSkip: failOnSkip, savePath: savePath)
            }.value
            return true
        } catch {
            present(error)
            return false
        }
    }

    // MARK: - error presentation

    /// Fold any thrown error into the plain banner. Mirrors
    /// `CryptoModel.present`. A gap (mockryx exit 1) is NEVER routed through
    /// here - it is a normal `Ok(DrillReportRecord)` with `hasGaps == true`,
    /// never an error (see `DrillsError`'s own doc, the one bug class this
    /// whole wave's review discipline calls out by name).
    private func present(_ error: Error) {
        guard let drillsError = error as? DrillsError else {
            bannerMessage = String(describing: error)
            return
        }
        switch drillsError {
        case .NoEnvironment:
            connection = .noEnvironment
        case .Spawn(let bin, let reason):
            bannerMessage = "Could not run mockryx at \(bin): \(reason)"
        case .Cli(let code, let stderr):
            bannerMessage = "mockryx exited \(code): \(stderr)"
        case .Json(let reason):
            bannerMessage = "Could not parse mockryx output: \(reason)"
        case .Read(let path, let reason):
            bannerMessage = "Could not read \(path): \(reason)"
        }
    }
}
