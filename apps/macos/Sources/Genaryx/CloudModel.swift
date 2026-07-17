import Foundation
import GenaryxCoreFFI
import Observation

/// Whole-panel connection state for the Money + Overview surface - honest,
/// distinct states the views render directly rather than inferring from a
/// read's error shape (mirrors the Tauri shell's `MoneyStatus` union,
/// collapsed to one enum since `CloudHandle`'s constructors are synchronous:
/// there is no separate "bootstrapping" window on the Rust side to poll for,
/// only a single `connecting` -> outcome transition here).
enum CloudConnection: Equatable {
    case connecting
    case noEnvironment
    case pairingFailed(reason: String)
    case ready(source: EnvSource, cloudUrl: String, orgDomain: String)

    var isReady: Bool {
        if case .ready = self { return true }
        return false
    }
}

/// One `CloudError.PlanRequired` rejection, rendered as an upsell tile
/// instead of an error banner (spec parity with the Tauri shell).
struct PlanRequiredNotice: Identifiable, Equatable {
    let id = UUID()
    let feature: String
    let org: String
    let upgradeUrl: String
}

/// Live Money + Overview state for the SwiftUI shell: owns a `CloudHandle`
/// (paired once at `connect()`) and fetches/mutates through it.
///
/// `CloudHandle`'s exported methods are synchronous and can block on network
/// I/O (each one wraps `genaryx-connectors::CloudClient`'s async calls with
/// the handle's own owned `tokio::runtime::Runtime` - see
/// `crates/ffi/src/cloud/mod.rs`). Every call into the handle below therefore
/// runs inside `Task.detached`, off this model's `@MainActor` isolation, and
/// only the already-resolved result is awaited back onto the main actor -
/// never the blocking call itself. `CloudHandle` is generated as
/// `@unchecked Sendable`, so capturing it into a detached task's closure is
/// safe.
@MainActor
@Observable
final class CloudModel {
    private(set) var connection: CloudConnection = .connecting

    private(set) var overview: Overview?
    private(set) var runs: [Run] = []
    private(set) var incidents: [Incident] = []
    private(set) var savings: Savings?

    /// Any non-`plan_required` `CloudError`, or a connect-time failure that
    /// is not itself a whole-panel state - rendered as a dismissable banner.
    private(set) var bannerMessage: String?
    /// A `plan_required` rejection - rendered as an upsell tile instead.
    private(set) var planRequired: PlanRequiredNotice?
    /// Set after a mutation settles (success or failure), for the transient
    /// notice bar the Money view shows.
    private(set) var mutationNotice: String?

    private(set) var isRefreshingOverview = false
    private(set) var isRefreshingMoney = false

    /// Session-local budget overrides are already handled Rust-side (mirrors
    /// `MoneyState::budget_overrides`), so this model just re-reads `runs()`
    /// after a successful `setBudget` - no client-side cache to keep here.

    private var handle: CloudHandle?

    init() {
        Task { await self.connect() }
    }

    // MARK: - connect

    /// (Re)resolve an environment and pair a fresh device. Called once from
    /// `init()`; also reachable from a "retry" affordance in the empty
    /// state.
    func connect() async {
        connection = .connecting
        bannerMessage = nil
        planRequired = nil
        handle = nil

        do {
            let newHandle = try await Task.detached { try CloudHandle.discover() }.value
            handle = newHandle
            connection = .ready(source: newHandle.source(), cloudUrl: newHandle.cloudUrl(), orgDomain: newHandle.orgDomain())
            async let overviewLoad: Void = refreshOverview()
            async let moneyLoad: Void = refreshMoney()
            _ = await (overviewLoad, moneyLoad)
        } catch {
            handle = nil
            connection = Self.connectionFailure(from: error)
        }
    }

    private static func connectionFailure(from error: Error) -> CloudConnection {
        guard let cloudError = error as? CloudError else {
            return .pairingFailed(reason: String(describing: error))
        }
        switch cloudError {
        case .NoEnvironment:
            return .noEnvironment
        case .PairingFailed(let reason):
            return .pairingFailed(reason: reason)
        case .PlanRequired(let feature, let org, let upgradeUrl):
            // Not expected during connect (no read/mutation has happened
            // yet), but handled honestly rather than assumed impossible.
            return .pairingFailed(reason: "plan required: \(feature) on \(org) (upgrade: \(upgradeUrl))")
        case .Cloud(let status, let message):
            return .pairingFailed(reason: status.map { "cloud error \($0): \(message)" } ?? message)
        }
    }

    // MARK: - reads

    func refreshOverview() async {
        guard let handle else { return }
        isRefreshingOverview = true
        defer { isRefreshingOverview = false }
        do {
            overview = try await Task.detached { try handle.overview() }.value
        } catch {
            present(error)
        }
    }

    func refreshMoney() async {
        guard let handle else { return }
        isRefreshingMoney = true
        defer { isRefreshingMoney = false }
        do {
            async let runsLoad = Task.detached { try handle.runs() }.value
            async let incidentsLoad = Task.detached { try handle.incidents() }.value
            async let savingsLoad = Task.detached { try handle.savings() }.value
            let (loadedRuns, loadedIncidents, loadedSavings) = try await (runsLoad, incidentsLoad, savingsLoad)
            runs = loadedRuns
            incidents = loadedIncidents
            savings = loadedSavings
        } catch {
            present(error)
        }
    }

    // MARK: - signed mutations
    // Every mutation below always journals a `console_command` Rust-side
    // (`CloudHandle::finish_mutation`, even on a Cloud rejection), then
    // triggers a refresh so the runs/incidents/savings tables reflect the
    // new state immediately.

    @discardableResult
    func killRun(_ runId: String) async -> Bool {
        guard let handle else { return false }
        do {
            let outcome = try await Task.detached { try handle.killRun(runId: runId) }.value
            applyMutationOutcome(outcome)
            return true
        } catch {
            present(error)
            return false
        }
    }

    @discardableResult
    func setBudget(runId: String, usd: Double) async -> Bool {
        guard let handle else { return false }
        do {
            let outcome = try await Task.detached { try handle.setBudget(runId: runId, budgetUsd: usd) }.value
            applyMutationOutcome(outcome)
            return true
        } catch {
            present(error)
            return false
        }
    }

    @discardableResult
    func ackIncident(_ id: String) async -> Bool {
        guard let handle else { return false }
        do {
            let outcome = try await Task.detached { try handle.ackIncident(id: id) }.value
            applyMutationOutcome(outcome)
            return true
        } catch {
            present(error)
            return false
        }
    }

    private func applyMutationOutcome(_ outcome: MutationOutcome) {
        mutationNotice =
            outcome.busRecorded
            ? "\(outcome.summary) - signed console_command recorded."
            : "\(outcome.summary) (bus journal not recorded: \(outcome.busError ?? "unknown reason"))"
        Task {
            await refreshOverview()
            await refreshMoney()
        }
    }

    // MARK: - error presentation

    /// Fold any thrown error into either the upsell tile or the plain
    /// banner - never both, and `plan_required` always goes to the upsell
    /// (spec: an upgrade is not an error toast). Unrecognized errors (not a
    /// `CloudError` at all) still render, just with `String(describing:)`.
    private func present(_ error: Error) {
        guard let cloudError = error as? CloudError else {
            bannerMessage = String(describing: error)
            return
        }
        switch cloudError {
        case .NoEnvironment:
            connection = .noEnvironment
        case .PairingFailed(let reason):
            bannerMessage = "Pairing failed: \(reason)"
        case .PlanRequired(let feature, let org, let upgradeUrl):
            planRequired = PlanRequiredNotice(feature: feature, org: org, upgradeUrl: upgradeUrl)
        case .Cloud(let status, let message):
            bannerMessage = status.map { "Cloud error \($0): \(message)" } ?? message
        }
    }
}
