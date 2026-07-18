import Foundation
import GenaryxCoreFFI
import Observation

/// Whole-panel connection state - mirrors `RemoteConnection`'s own shape
/// (`RemoteModel.swift`): `PocketHandle()` only starts a local async
/// runtime (see its own module doc, `crates/ffi/src/pocket/mod.rs`), so
/// `.failed` here can only mean a genuine local resource problem, never a
/// Cloud/relay reachability issue - those are each `model.status`'s own
/// honest states instead (idle / paired / relay-unreachable), read
/// separately once `connection == .ready`.
enum PocketConnection: Equatable {
    case connecting
    case failed(reason: String)
    case ready

    var isReady: Bool {
        if case .ready = self { return true }
        return false
    }
}

/// Live Pocket panel state for the SwiftUI shell (docs/PHASE5.md W2,
/// itrat-console/13 D12.2a): owns a `PocketHandle` (constructed once at
/// `connect()`) and drives the "Connect TokenFuse Pocket" pairing flow
/// through it - mint a code at the Cloud, arm the relay's pairing window,
/// hold the returned QR for the view to render, then show the paired device
/// + Disconnect. At parity with the Tauri shell's `pocket` module
/// (`apps/desktop/src-tauri/src/pocket/`, see its own module doc for the
/// identical flow this mirrors).
///
/// The "showing-QR" step lives entirely in THIS model's own `armedQr` field,
/// never the backend: the relay exposes no "is a window currently armed"
/// read, only device-paired state (mirrors
/// `apps/desktop/src/lib/usePocketStatus.ts`'s identical design note on the
/// Tauri side). `PocketView` polls `refreshStatus()` on a timer while
/// `armedQr` is set (and not yet expired) so the panel notices the phone
/// pairing and flips to the Paired view on its own, without the operator
/// refreshing anything.
///
/// `PocketHandle`'s exported methods are synchronous and can block (real
/// HTTP calls to the Cloud and the relay), so every call into the handle
/// below runs inside `Task.detached`, off this model's `@MainActor`
/// isolation - mirrors `RemoteModel`/`CryptoModel`/`DrillsModel`'s identical
/// discipline (see `RemoteModel.swift`'s own doc for the full rationale).
/// `PocketHandle` is generated as `@unchecked Sendable`, so capturing it
/// into a detached task's closure is safe.
@MainActor
@Observable
final class PocketModel {
    private(set) var connection: PocketConnection = .connecting
    private(set) var status: PocketStatusRecord?
    /// The QR the operator is currently being shown, if any - see the type
    /// doc's "showing-QR" paragraph for why this is local, not
    /// backend-tracked state.
    private(set) var armedQr: PocketQrRecord?
    private(set) var isConnecting = false
    private(set) var isDisconnecting = false
    private(set) var bannerMessage: String?

    private var handle: PocketHandle?

    init() {
        Task { await self.connect() }
    }

    // MARK: - connect

    /// Build a fresh handle. Called once from `init()`; also reachable from
    /// a "retry" affordance in the empty state.
    func connect() async {
        connection = .connecting
        bannerMessage = nil
        do {
            let newHandle = try await Task.detached { try PocketHandle() }.value
            handle = newHandle
            connection = .ready
            await refreshStatus()
        } catch {
            handle = nil
            connection = .failed(reason: String(describing: error))
        }
    }

    // MARK: - status

    /// Re-read the whole-panel status. `PocketView` polls this on a timer
    /// while `armedQr` is set, and every mutating method below also calls
    /// it (or applies its own return value) so the view never has to guess
    /// when to refresh.
    func refreshStatus() async {
        guard let handle else { return }
        let next = await Task.detached { handle.status() }.value
        status = next
        // The moment the backend reports paired (the phone scanned and
        // redeemed the code), drop the local QR state - the view then falls
        // through to the Paired branch on `status` alone.
        if case .paired = next, armedQr != nil {
            armedQr = nil
        }
    }

    // MARK: - connect / cancel / disconnect

    /// "Connect TokenFuse Pocket": mint a code, arm the relay's pairing
    /// window, and hold the returned QR for the view to render.
    func connectPocket() async {
        guard let handle, !isConnecting else { return }
        isConnecting = true
        defer { isConnecting = false }
        bannerMessage = nil
        do {
            let qr = try await Task.detached { try handle.connect() }.value
            armedQr = qr
        } catch PocketError.DeviceExists {
            // Someone paired between the last status poll and this tap - a
            // normal race, not a failure; refreshStatus() picks up the real
            // Paired state instead of showing an error banner.
            await refreshStatus()
        } catch {
            present(error)
        }
    }

    /// "Cancel": give up on an armed-but-unredeemed window. Nothing is
    /// actually paired yet at this point in the flow, so this reuses
    /// Disconnect purely to clear the pairing-window row - an
    /// operator-initiated version of docs/PHASE5.md W2's "do not leave a
    /// half-armed window silently" rule, not just the error-path cleanup
    /// `PocketHandle.connect()` itself already does server-side.
    func cancelArmedWindow() async {
        guard let handle else { return }
        armedQr = nil
        _ = try? await Task.detached { try handle.disconnect() }.value
        await refreshStatus()
    }

    /// Disconnect the paired phone (always safe to call).
    func disconnectPocket() async {
        guard let handle, !isDisconnecting else { return }
        isDisconnecting = true
        defer { isDisconnecting = false }
        bannerMessage = nil
        do {
            status = try await Task.detached { try handle.disconnect() }.value
        } catch {
            present(error)
        }
    }

    // MARK: - error presentation

    /// Fold any thrown error into the plain banner. Mirrors
    /// `RemoteModel.present`/`CryptoModel.present`/`DrillsModel.present`.
    private func present(_ error: Error) {
        guard let pocketError = error as? PocketError else {
            bannerMessage = String(describing: error)
            return
        }
        bannerMessage = Self.describe(pocketError)
    }

    private static func describe(_ error: PocketError) -> String {
        switch error {
        case .Runtime(let reason):
            return "Could not start the local runtime: \(reason)"
        case .NoCloudEnvironment:
            return "No TokenFuse Cloud environment found (see Overview/Money) - cannot mint a pairing code yet."
        case .Cloud(let message):
            return "Cloud error: \(message)"
        case .DeviceExists:
            return "A phone is already paired - disconnect it first to pair a different one."
        case .Relay(let message):
            return "Relay error: \(message)"
        }
    }
}
