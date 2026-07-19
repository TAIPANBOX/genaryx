import Foundation
import GenaryxCoreFFI
import LocalAuthentication
import Observation

/// Whole-panel connection state for the Policy surface - honest, distinct
/// states the views render directly rather than inferring from a read's
/// error shape. Mirrors `CloudConnection` exactly (`CloudModel.swift`),
/// swapped to `WardryxHandle`'s simpler, pairing-free construction: there is
/// no `PairingFailed` case here because `WardryxHandle.connect` never
/// touches the network at all (see `crates/ffi/src/wardryx/mod.rs`'s module
/// doc) - a `.connectFailed` here can only mean a local runtime/filesystem
/// problem, never an unreachable server.
enum WardryxConnection: Equatable {
    case connecting
    case noEnvironment
    case connectFailed(reason: String)
    case ready(source: WardryxEnvSource, wardryxUrl: String, orgDomain: String)

    var isReady: Bool {
        if case .ready = self { return true }
        return false
    }
}

/// Live Policy state for the SwiftUI shell: owns a `WardryxHandle`
/// (constructed once at `connect()`) and reads/decides through it.
///
/// `WardryxHandle`'s exported methods are synchronous and can block on
/// network I/O (each one wraps `genaryx-connectors::WardryxClient`'s async
/// calls with the handle's own owned `tokio::runtime::Runtime` - see
/// `crates/ffi/src/wardryx/mod.rs`). Every call into the handle below
/// therefore runs inside `Task.detached`, off this model's `@MainActor`
/// isolation, exactly like `CloudModel` - see that file's own doc for the
/// full rationale. `WardryxHandle` is generated as `@unchecked Sendable`, so
/// capturing it into a detached task's closure is safe.
@MainActor
@Observable
final class PolicyModel {
    private(set) var connection: WardryxConnection = .connecting

    private(set) var approvals: [ApprovalRecord] = []
    private(set) var policies: [PolicyRecord] = []

    /// Any `WardryxError`, or a connect-time failure that is not itself a
    /// whole-panel state - rendered as a dismissable banner.
    private(set) var bannerMessage: String?
    /// Set after a decision settles (success or failure), for the transient
    /// notice bar the Policy view shows - mirrors `CloudModel.mutationNotice`.
    private(set) var mutationNotice: String?
    /// The most recent GRANT's decoded outcome (ceiling / TTL / tools),
    /// shown as a detail card until dismissed or replaced by the next grant
    /// (PHASE2.md: "show the operator exactly what they authorized"). `nil`
    /// after a deny - there is no token to show.
    private(set) var lastGrant: ApprovalDecideOutcome?

    private(set) var isRefreshing = false

    private var handle: WardryxHandle?

    /// The connected `WardryxHandle` this model owns, exposed read-only so
    /// the C2 copilot-approval audit link (docs/PHASE6-C2.md, `GenaryxApp`'s
    /// `approveCopilotGrantDeny`) can journal
    /// `console.copilot_proposal_approved` through the SAME bearer client
    /// the Policy panel already reads/decides through, mirroring
    /// `CloudModel.cloudHandle`'s identical "reuse the existing handle,
    /// never a second connection" rationale exactly. `nil` whenever
    /// `connection` is not `.ready` - same guard as every other read on
    /// `handle` in this model.
    var wardryxHandle: WardryxHandle? { handle }

    init() {
        Task { await self.connect() }
    }

    // MARK: - connect

    /// (Re)resolve an environment and build a fresh bearer client. Called
    /// once from `init()`; also reachable from a "retry" affordance in the
    /// empty state.
    func connect() async {
        connection = .connecting
        bannerMessage = nil
        lastGrant = nil
        handle = nil

        do {
            let newHandle = try await Task.detached { try WardryxHandle.discover() }.value
            handle = newHandle
            connection = .ready(
                source: newHandle.source(), wardryxUrl: newHandle.wardryxUrl(), orgDomain: newHandle.orgDomain())
            await refresh()
        } catch {
            handle = nil
            connection = Self.connectionFailure(from: error)
        }
    }

    private static func connectionFailure(from error: Error) -> WardryxConnection {
        guard let wardryxError = error as? WardryxError else {
            return .connectFailed(reason: String(describing: error))
        }
        switch wardryxError {
        case .NoEnvironment:
            return .noEnvironment
        case .ConnectFailed(let reason):
            return .connectFailed(reason: reason)
        case .ApprovalNotFound, .ApprovalAlreadyDecided, .Forbidden, .NoApprovalSecret, .BadToken, .Api:
            // Not expected during connect (no read/decide has happened yet -
            // `WardryxHandle.discover`/`connect` never call the network),
            // but handled honestly rather than assumed impossible.
            return .connectFailed(reason: String(describing: wardryxError))
        }
    }

    // MARK: - reads

    func refresh() async {
        guard let handle else { return }
        isRefreshing = true
        defer { isRefreshing = false }
        do {
            async let approvalsLoad = Task.detached { try handle.listApprovals() }.value
            async let policiesLoad = Task.detached { try handle.listPolicies() }.value
            let (loadedApprovals, loadedPolicies) = try await (approvalsLoad, policiesLoad)
            approvals = loadedApprovals
            policies = loadedPolicies
        } catch {
            present(error)
        }
    }

    /// The most-recently-requested approval's `policyVersion`, the closest
    /// honestly-observable proxy for the Policy view's "set-level
    /// `policy_version`" (`PolicyRecord`'s own Rust doc comment explains why
    /// this is composed here, at the view-model layer, rather than carried
    /// on the DTO: `GET /v1/policies`'s wire shape has no such field at
    /// all). `nil` until at least one approval has ever been observed.
    var latestPolicyVersion: String? {
        approvals.max { $0.requestedAt < $1.requestedAt }?.policyVersion
    }

    // MARK: - the one privileged mutation

    /// Grant or deny a pending approval. ALWAYS challenges a local hardware
    /// confirmation (Touch ID, falling back to the device passcode when
    /// biometrics are unavailable/unenrolled) before ever calling
    /// `WardryxHandle.decideApproval` - PHASE2.md: "SwiftUI gates it behind
    /// a local hardware confirmation (LocalAuthentication / Touch ID)
    /// BEFORE the call". Both Grant and Deny go through this same gate: both
    /// are privileged mutations that get journaled as a `console_command`.
    @discardableResult
    func decide(_ id: String, verdict: ApprovalVerdict) async -> ApprovalDecideOutcome? {
        guard let handle else { return nil }

        let verb = verdict == .grant ? "grant" : "deny"
        guard await Self.confirmLocalAuthentication(reason: "Confirm to \(verb) approval \(id)") else {
            bannerMessage = "Touch ID confirmation was not completed; \(verb) cancelled."
            return nil
        }

        do {
            let outcome = try await Task.detached { try handle.decideApproval(id: id, verdict: verdict) }.value
            applyOutcome(outcome)
            return outcome
        } catch {
            present(error)
            // The most likely failure here (`ApprovalAlreadyDecided`) means
            // this row's state is stale - re-pull the real state rather than
            // leaving a now-wrong pending row on screen.
            Task { await refresh() }
            return nil
        }
    }

    private func applyOutcome(_ outcome: ApprovalDecideOutcome) {
        // Only ever SET here, never cleared by a deny: a deny on one
        // approval must not hide a still-relevant, still-counting-down
        // grant card from an earlier decision - see `lastGrant`'s own doc
        // comment ("until dismissed or replaced by the next grant").
        if outcome.granted {
            lastGrant = outcome
        }
        mutationNotice =
            outcome.busRecorded
            ? "\(outcome.summary) - signed console_command recorded."
            : "\(outcome.summary) (bus journal not recorded: \(outcome.busError ?? "unknown reason"))"
        Task {
            await refresh()
        }
    }

    /// Dismiss the grant detail card without waiting for the next decision.
    func dismissLastGrant() {
        lastGrant = nil
    }

    // MARK: - local hardware confirmation (Touch ID)

    /// Challenge `LAContext` for a device-owner confirmation: Touch ID when
    /// the device can evaluate biometrics, falling back to
    /// `.deviceOwnerAuthentication` (Touch ID OR the account
    /// password/passcode) otherwise - exactly the two policies PHASE2.md
    /// names. `false` on any refusal: no biometrics/passcode enrolled at
    /// all, the system sheet was cancelled, or the challenge failed -
    /// [`decide`] never proceeds to the privileged call in that case.
    private static func confirmLocalAuthentication(reason: String) async -> Bool {
        let context = LAContext()
        let policy: LAPolicy =
            context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
            ? .deviceOwnerAuthenticationWithBiometrics
            : .deviceOwnerAuthentication
        guard context.canEvaluatePolicy(policy, error: nil) else {
            return false
        }
        return await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
            context.evaluatePolicy(policy, localizedReason: reason) { success, _ in
                continuation.resume(returning: success)
            }
        }
    }

    // MARK: - error presentation

    /// Fold any thrown error into the plain banner (Wardryx has no
    /// `plan_required`-shaped upsell case, unlike `CloudError`). Unrecognized
    /// errors (not a `WardryxError` at all) still render, just with
    /// `String(describing:)`.
    private func present(_ error: Error) {
        guard let wardryxError = error as? WardryxError else {
            bannerMessage = String(describing: error)
            return
        }
        switch wardryxError {
        case .NoEnvironment:
            connection = .noEnvironment
        case .ConnectFailed(let reason):
            bannerMessage = "Could not connect to Wardryx: \(reason)"
        case .ApprovalNotFound:
            bannerMessage = "That approval no longer exists (already decided or expired)."
        case .ApprovalAlreadyDecided:
            bannerMessage = "That approval was already decided - refreshing."
        case .Forbidden:
            bannerMessage = "Admin role required for this action."
        case .NoApprovalSecret:
            bannerMessage = "The Wardryx server has no WARDRYX_APPROVAL_SECRET configured; grant refused."
        case .BadToken(let reason):
            bannerMessage = "Could not decode the approval token: \(reason)"
        case .Api(let status, let message):
            bannerMessage = status.map { "Wardryx error \($0): \(message)" } ?? message
        }
    }
}
