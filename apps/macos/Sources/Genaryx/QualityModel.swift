import Foundation
import GenaryxCoreFFI
import Observation

/// Whole-panel connection state for the Quality surface - honest, distinct
/// states the views render directly rather than inferring from a read's
/// error shape. Mirrors `IdryxConnection` (`IdentityModel.swift`), swapped to
/// `QualityHandle`'s even simpler construction: there is no network call
/// anywhere in this handle at all (Verdryx is a local SQLite reader -
/// `crates/ffi/src/quality/mod.rs`'s module doc), so `.connectFailed` here
/// can only mean a local filesystem/schema problem (an operator-named
/// `verdryx.db` that will not open, or opens but does not match the expected
/// schema), never a network failure.
enum QualityConnection: Equatable {
    case connecting
    case noEnvironment
    case connectFailed(reason: String)
    case ready(source: QualityEnvSource, dbPath: String)

    var isReady: Bool {
        if case .ready = self { return true }
        return false
    }
}

/// Live Quality state for the SwiftUI shell: owns a `QualityHandle`
/// (constructed once at `connect()`) and reads through it. Read-only, at
/// parity with the handle it wraps: Quality has no mutation of any kind this
/// wave (docs/PHASE4.md W1).
///
/// `QualityHandle`'s exported methods are synchronous and open a fresh
/// SQLite connection per call (`crates/ffi/src/quality/mod.rs`'s own doc:
/// "open fresh, per call") - cheap, but still local I/O, so every call below
/// still runs inside `Task.detached`, off this model's `@MainActor`
/// isolation, exactly like `IdentityModel`/`PolicyModel` - see
/// `PolicyModel.swift`'s own doc for the full rationale. `QualityHandle` is
/// generated as `@unchecked Sendable`, so capturing it into a detached task's
/// closure is safe.
@MainActor
@Observable
final class QualityModel {
    private(set) var connection: QualityConnection = .connecting

    /// Newest-started first (the connector's own sort order).
    private(set) var evalRuns: [EvalRunRecord] = []
    /// Per-run rollups (case count, mean score, total cost), keyed by
    /// `EvalRunRecord.id` - fanned out one `run_summary` call per run in
    /// `evalRuns` (there is no bulk-summary read on the handle: see
    /// `crates/ffi/src/quality/mod.rs`'s own doc on `list_eval_runs`),
    /// bounded by `summaryFanOutLimit`. A run beyond that bound, or one whose
    /// summary call is still in flight/failed, is simply absent here - the
    /// history row falls back to its base fields in that case, never a crash
    /// or a fabricated stat.
    private(set) var runSummaries: [String: RunSummaryRecord] = [:]
    private(set) var baselines: [BaselineRecord] = []

    /// The Eval Runs history row currently expanded into the run-detail
    /// section below it. Defaults to the newest run once `evalRuns` first
    /// loads.
    private(set) var selectedRunId: String?
    /// The selected run's per-case scores, oldest-inserted-first (the
    /// connector's own order).
    private(set) var scores: [ScoreRecord] = []
    private(set) var isLoadingDetail = false

    /// When the current snapshot was pulled - this panel's "as of load"
    /// label, the same honesty convention `IdentityModel.loadedAt` uses:
    /// Verdryx has no live push of its own (only the `quality_drift` BUS
    /// event is live, read separately - see `QualityView.swift`), so a
    /// periodic re-poll (this model's own `.task` loop in `QualityView`) is
    /// how new `verdryx eval` runs become visible, and this timestamp is the
    /// honest "as of" marker for whatever is currently on screen.
    private(set) var loadedAt: Date?

    private(set) var bannerMessage: String?
    private(set) var isRefreshing = false

    private var handle: QualityHandle?

    /// Bounds the per-run `run_summary` fan-out on every refresh - mirrors
    /// `IdentityView.swift`'s own `displayLimit` convention (a UI-side cap on
    /// how much of a potentially-long list gets the expensive treatment,
    /// applied here to FFI call count rather than rendered rows).
    private static let summaryFanOutLimit = 50

    init() {
        Task { await self.connect() }
    }

    // MARK: - connect

    /// (Re)resolve an environment and build a fresh handle. Called once from
    /// `init()`; also reachable from a "retry" affordance in the empty state.
    func connect() async {
        connection = .connecting
        bannerMessage = nil
        selectedRunId = nil
        scores = []
        handle = nil

        do {
            let newHandle = try await Task.detached { try QualityHandle.discover() }.value
            handle = newHandle
            connection = .ready(source: newHandle.source(), dbPath: newHandle.dbPath())
            await refresh()
        } catch {
            handle = nil
            connection = Self.connectionFailure(from: error)
        }
    }

    private static func connectionFailure(from error: Error) -> QualityConnection {
        guard let qualityError = error as? QualityError else {
            return .connectFailed(reason: String(describing: error))
        }
        switch qualityError {
        case .NoEnvironment:
            return .noEnvironment
        case .Open(let path, let reason):
            return .connectFailed(reason: "\(path): \(reason)")
        case .Query(let reason):
            // Not expected during connect (no read has happened yet -
            // `QualityHandle.discover`/`connect` never open the database at
            // construction), but handled honestly rather than assumed
            // impossible - mirrors `PolicyModel.connectionFailure`'s own
            // defensive default arm.
            return .connectFailed(reason: reason)
        }
    }

    // MARK: - reads

    /// Pull eval-runs history + baselines, then (re)load the selected run's
    /// detail (or default-select the newest run on first load).
    func refresh() async {
        guard let handle else { return }
        isRefreshing = true
        defer { isRefreshing = false }
        do {
            async let runsLoad = Task.detached { try handle.listEvalRuns() }.value
            async let baselinesLoad = Task.detached { try handle.listBaselines() }.value
            let (loadedRuns, loadedBaselines) = try await (runsLoad, baselinesLoad)
            evalRuns = loadedRuns
            baselines = loadedBaselines
            loadedAt = Date()

            await loadSummaries(for: Array(loadedRuns.prefix(Self.summaryFanOutLimit)))

            if let selectedRunId, loadedRuns.contains(where: { $0.id == selectedRunId }) {
                await loadRunDetail(selectedRunId)
            } else if let newest = loadedRuns.first {
                await selectRun(newest.id)
            } else {
                selectedRunId = nil
                scores = []
            }
        } catch {
            present(error)
        }
    }

    /// One `run_summary` call per run, sequentially (each is a fast local
    /// SQLite read - see the handle's own "open fresh, per call" doc, so
    /// paying for up to `summaryFanOutLimit` of them in sequence is not a
    /// meaningful delay). A single run's summary failing is swallowed here
    /// (that row just falls back to its base fields) rather than failing the
    /// whole refresh over one bad row.
    private func loadSummaries(for runs: [EvalRunRecord]) async {
        guard let handle else { return }
        for run in runs {
            let summary = try? await Task.detached { try handle.runSummary(runId: run.id) }.value
            if let summary {
                runSummaries[run.id] = summary
            }
        }
    }

    /// Select a history row: loads its summary (kept in `runSummaries`
    /// alongside every other fanned-out row) and its per-case scores for the
    /// run-detail section.
    func selectRun(_ runId: String) async {
        selectedRunId = runId
        await loadRunDetail(runId)
    }

    private func loadRunDetail(_ runId: String) async {
        guard let handle else { return }
        isLoadingDetail = true
        defer { isLoadingDetail = false }
        do {
            async let summaryLoad = Task.detached { try handle.runSummary(runId: runId) }.value
            async let scoresLoad = Task.detached { try handle.scoresForRun(runId: runId) }.value
            let (summary, loadedScores) = try await (summaryLoad, scoresLoad)
            if let summary {
                runSummaries[runId] = summary
            }
            scores = loadedScores
        } catch {
            present(error)
        }
    }

    /// The selected run's rollup, for the run-detail header. `nil` while
    /// still loading or when no run is selected (an empty store) - the view
    /// renders that as "n/a" fields, never a fabricated zero.
    var selectedRunSummary: RunSummaryRecord? {
        guard let selectedRunId else { return nil }
        return runSummaries[selectedRunId]
    }

    // MARK: - error presentation

    /// Fold any thrown error into the plain banner. Mirrors
    /// `IdentityModel.present`.
    private func present(_ error: Error) {
        guard let qualityError = error as? QualityError else {
            bannerMessage = String(describing: error)
            return
        }
        switch qualityError {
        case .NoEnvironment:
            connection = .noEnvironment
        case .Open(let path, let reason):
            bannerMessage = "Could not open verdryx.db at \(path): \(reason)"
        case .Query(let reason):
            bannerMessage = "verdryx.db query failed: \(reason)"
        }
    }
}
