import Foundation
import GenaryxCoreFFI
import Observation

/// Whole-panel connection state for the Identity surface - honest, distinct
/// states the views render directly rather than inferring from a read's
/// error shape. Mirrors `WardryxConnection` (`PolicyModel.swift`), swapped to
/// `IdryxHandle`'s even simpler construction: there is no `ConnectFailed`
/// reachable from a network problem here either (idryx has no
/// pairing/auth handshake at all - see `crates/ffi/src/idryx/mod.rs`'s
/// module doc), so `.connectFailed` can only mean a local runtime/resource
/// problem, same as `WardryxConnection`'s own `.connectFailed`.
enum IdryxConnection: Equatable {
    case connecting
    case noEnvironment
    case connectFailed(reason: String)
    case ready(source: IdryxEnvSource, idryxUrl: String)

    var isReady: Bool {
        if case .ready = self { return true }
        return false
    }
}

/// Live Identity state for the SwiftUI shell: owns an `IdryxHandle`
/// (constructed once at `connect()`) and reads through it. Read-only, at
/// parity with the handle it wraps: no decision/mutation method exists here
/// at all (unlike `PolicyModel.decide`), since Identity is a read-only panel
/// this wave (docs/PHASE3.md W2).
///
/// `IdryxHandle`'s exported methods are synchronous and can block (I/O:
/// `listIdentities`/`listAlerts`/`listRemediations` block on HTTP,
/// `rescan`/`rescanUnavailableReason` block on local filesystem/subprocess
/// work). Every call into the handle below therefore runs inside
/// `Task.detached`, off this model's `@MainActor` isolation, exactly like
/// `PolicyModel`/`CloudModel` - see `PolicyModel.swift`'s own doc for the
/// full rationale. `IdryxHandle` is generated as `@unchecked Sendable`, so
/// capturing it into a detached task's closure is safe.
@MainActor
@Observable
final class IdentityModel {
    private(set) var connection: IdryxConnection = .connecting

    private(set) var identities: [IdentityRecord] = []
    private(set) var alerts: [AlertRecord] = []
    private(set) var remediations: [RemediationRecord] = []

    /// When the current `identities`/`alerts`/`remediations` snapshot was
    /// pulled - the SwiftUI panel's "as of load" label (docs/PHASE3.md:
    /// "serve is LOAD-ONCE... Polling `/api/*` returns byte-identical data
    /// for the process lifetime", so this timestamp, not a live indicator,
    /// is the honest thing to show). Set on every successful `refresh()` and
    /// again on every successful `rescan()` (which replaces `alerts` with a
    /// freshly recomputed set).
    private(set) var loadedAt: Date?

    /// Any `IdryxError`, or a connect-time failure that is not itself a
    /// whole-panel state - rendered as a dismissable banner. Mirrors
    /// `PolicyModel.bannerMessage`.
    private(set) var bannerMessage: String?
    /// Set after a Rescan attempt settles (success or failure), for the
    /// transient notice bar the Identity view shows - mirrors
    /// `CloudModel.mutationNotice` / `PolicyModel.mutationNotice`. Named the
    /// same across all three models even though Rescan is not a privileged
    /// mutation (see the type doc): the UI role - a one-shot, dismissable
    /// "here is what just happened" line - is identical.
    private(set) var mutationNotice: String?

    private(set) var isRefreshing = false
    private(set) var isRescanning = false
    /// `nil` when Rescan is currently expected to work; otherwise the exact
    /// reason `IdryxHandle.rescanUnavailableReason()` gave (PHASE3.md: "a
    /// Rescan button... disabled with an honest note when the binary is
    /// unavailable"). Refreshed on connect and after every Rescan attempt,
    /// so the button's disabled state is known BEFORE the operator clicks it
    /// whenever possible, not only after a failed attempt.
    private(set) var rescanUnavailableReason: String?

    private var handle: IdryxHandle?

    init() {
        Task { await self.connect() }
    }

    // MARK: - connect

    /// (Re)resolve an environment and build a fresh client. Called once from
    /// `init()`; also reachable from a "retry" affordance in the empty
    /// state.
    func connect() async {
        connection = .connecting
        bannerMessage = nil
        mutationNotice = nil
        rescanUnavailableReason = nil
        handle = nil

        do {
            let newHandle = try await Task.detached { try IdryxHandle.discover() }.value
            handle = newHandle
            connection = .ready(source: newHandle.source(), idryxUrl: newHandle.idryxUrl())
            await refresh()
            await refreshRescanAvailability()
        } catch {
            handle = nil
            connection = Self.connectionFailure(from: error)
        }
    }

    private static func connectionFailure(from error: Error) -> IdryxConnection {
        guard let idryxError = error as? IdryxError else {
            return .connectFailed(reason: String(describing: error))
        }
        switch idryxError {
        case .NoEnvironment:
            return .noEnvironment
        case .ConnectFailed(let reason):
            return .connectFailed(reason: reason)
        case .Api, .Transport, .Json, .RescanUnavailable, .RescanFailed:
            // Not expected during connect (no read/rescan has happened yet -
            // `IdryxHandle.discover`/`connect` never call the network or a
            // subprocess), but handled honestly rather than assumed
            // impossible - mirrors `PolicyModel.connectionFailure`'s own
            // defensive default arm.
            return .connectFailed(reason: String(describing: idryxError))
        }
    }

    // MARK: - reads

    /// Pull the load-once snapshot: identities, alerts, remediations, all in
    /// parallel. Mirrors `PolicyModel.refresh`'s shape.
    func refresh() async {
        guard let handle else { return }
        isRefreshing = true
        defer { isRefreshing = false }
        do {
            async let identitiesLoad = Task.detached { try handle.listIdentities() }.value
            async let alertsLoad = Task.detached { try handle.listAlerts() }.value
            async let remediationsLoad = Task.detached { try handle.listRemediations() }.value
            let (loadedIdentities, loadedAlerts, loadedRemediations) = try await (
                identitiesLoad, alertsLoad, remediationsLoad
            )
            identities = loadedIdentities
            alerts = loadedAlerts
            remediations = loadedRemediations
            loadedAt = Date()
        } catch {
            present(error)
        }
    }

    /// Refresh the Rescan button's honest availability note - see
    /// `rescanUnavailableReason`'s own doc comment.
    func refreshRescanAvailability() async {
        guard let handle else { return }
        rescanUnavailableReason = await Task.detached { handle.rescanUnavailableReason() }.value
    }

    // MARK: - Rescan (a recompute, not a privileged mutation - no journal,
    // no Touch ID gate: see `IdentityModel`'s own type doc and
    // `crates/ffi/src/idryx/mod.rs`'s module doc, "a subprocess call, not an
    // HTTP mutation")

    /// Recompute the 21 detectors over the current stack bus files and
    /// replace `alerts` with the fresh result. Never destructive (idryx's
    /// own snapshot is untouched; this only recomputes locally), so unlike
    /// `PolicyModel.decide` this needs no confirm ceremony - a plain button
    /// press is enough (docs/PHASE3.md W2 names it a "Rescan button", not a
    /// guarded action).
    @discardableResult
    func rescan() async -> Bool {
        guard let handle else { return false }
        isRescanning = true
        defer { isRescanning = false }
        do {
            let rescanned = try await Task.detached { try handle.rescan() }.value
            alerts = rescanned
            loadedAt = Date()
            rescanUnavailableReason = nil
            mutationNotice = "Rescan complete - \(rescanned.count) alert\(rescanned.count == 1 ? "" : "s")."
            return true
        } catch {
            present(error)
            await refreshRescanAvailability()
            return false
        }
    }

    // MARK: - error presentation

    /// Fold any thrown error into the plain banner. Mirrors
    /// `PolicyModel.present`.
    private func present(_ error: Error) {
        guard let idryxError = error as? IdryxError else {
            bannerMessage = String(describing: error)
            return
        }
        switch idryxError {
        case .NoEnvironment:
            connection = .noEnvironment
        case .ConnectFailed(let reason):
            bannerMessage = "Could not connect to Idryx: \(reason)"
        case .Api(let status, let message):
            bannerMessage = "Idryx error \(status): \(message)"
        case .Transport(let reason):
            bannerMessage = "Could not reach Idryx: \(reason)"
        case .Json(let reason):
            bannerMessage = "Unexpected response from Idryx: \(reason)"
        case .RescanUnavailable(let reason):
            bannerMessage = "Rescan unavailable: \(reason)"
        case .RescanFailed(let reason):
            bannerMessage = "Rescan failed: \(reason)"
        }
    }
}
