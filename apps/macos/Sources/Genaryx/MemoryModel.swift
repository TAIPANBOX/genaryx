import Foundation
import GenaryxCoreFFI
import Observation

/// Whole-panel connection state for the Memory surface - honest, distinct
/// states the views render directly rather than inferring from a read's
/// error shape. Mirrors `CryptoConnection`/`QualityConnection`, but unlike
/// either of those, `.connectFailed` here can genuinely mean the real
/// spawn+handshake of a subprocess failed (bad interpreter, missing Python
/// package, unusable db path) - not just a local file/filesystem problem -
/// because `MemoryHandle.discover`/`connect` both attempt a real
/// `engram-mcp` spawn at construction (`crates/ffi/src/memory/mod.rs`'s own
/// module doc: "New shape vs every sibling handle").
enum MemoryConnection: Equatable {
    case connecting
    case noEnvironment
    case connectFailed(reason: String)
    case ready(source: MemoryEnvSource, engramMcpBin: String, dbPath: String, agentId: String?)

    var isReady: Bool {
        if case .ready = self { return true }
        return false
    }
}

/// `recall`'s `mode` parameter - a closed set on the Swift side even though
/// the wire itself is a raw string (`crates/connectors/src/engram.rs`'s own
/// doc: "`mode` is NOT enum-validated on the MCP wire: an unknown mode
/// silently behaves as cosine"), so the picker can only ever send one of the
/// three engram actually documents.
enum RecallMode: String, CaseIterable, Identifiable {
    case cosine
    case spreading
    case hybrid

    var id: String { rawValue }
}

/// Live Memory state for the SwiftUI shell: owns a `MemoryHandle`
/// (constructed once at `connect()`, which itself spawns a real `engram-mcp`
/// child - see `MemoryConnection`'s own doc) and reads/acts through it.
///
/// `MemoryHandle`'s exported methods are synchronous and can block on real
/// subprocess I/O - `recall` especially so on its first call (the embedding
/// model's lazy load, `MemoryHandle`'s own doc comment). Every call into the
/// handle below therefore runs inside `Task.detached`, off this model's
/// `@MainActor` isolation, exactly like `CryptoModel`/`QualityModel` - see
/// `PolicyModel.swift`'s own doc for the full rationale. `MemoryHandle` is
/// generated as `@unchecked Sendable`, so capturing it into a detached task's
/// closure is safe.
///
/// Stats auto-refresh on a timer (mirrors `IdentityModel`/`QualityModel`:
/// the store's contents genuinely change as agents write memories, and
/// `stats` is a cheap read against an already-running process - unlike
/// Crypto's expensive per-scan subprocess spawn). `recall`/`why`/`forget`
/// stay strictly on-demand, never auto-run (docs/PHASE4.md W2: "running
/// `recall` on demand").
@MainActor
@Observable
final class MemoryModel {
    private(set) var connection: MemoryConnection = .connecting

    private(set) var stats: EngramStatsRecord?
    /// When `stats` was last pulled - this panel's own "as of" label,
    /// distinct from `lastRecallAt` (a live-ish background poll vs an
    /// explicit operator query - mirrors `LastScanFormat`/`LoadedAtFormat`'s
    /// own deliberate separation for the same reason).
    private(set) var statsLoadedAt: Date?

    // MARK: - recall

    var recallQuery: String = ""
    var recallMode: RecallMode = .cosine
    var recallLimit: Int = 5
    private(set) var recallResults: [EngramMemoryRecord] = []
    /// When the current `recallResults` were pulled - "as of last query"
    /// (docs/PHASE4.md W2), never implied live.
    private(set) var lastRecallAt: Date?
    private(set) var isRecalling = false
    /// `true` once ANY `recall` call has completed (success or failure) this
    /// session - lets the view show a heavier "loading the embedding model,
    /// this can take a few seconds..." progress copy only on the genuinely
    /// slow first call, and a lighter "searching..." on every one after
    /// (`MemoryHandle`'s own doc: "recall's first call can take several
    /// seconds").
    private(set) var hasRecalledOnce = false

    // MARK: - why (provenance)

    private(set) var selectedMemoryId: String?
    private(set) var provenance: EngramProvenanceRecord?
    private(set) var isLoadingProvenance = false
    /// A `why` failure (most often `.Tool` for an id that no longer exists -
    /// docs/PHASE4.md W2: "an unknown id shown as the honest Tool error"),
    /// scoped to the Provenance card rather than the whole-panel
    /// `bannerMessage` - selecting a since-forgotten memory should not read
    /// like the whole Memory plane broke.
    private(set) var provenanceError: String?

    // MARK: - forget (irreversible admin action)

    private(set) var isForgetting = false
    private(set) var mutationNotice: String?

    private(set) var bannerMessage: String?
    private(set) var isLoadingStats = false

    private var handle: MemoryHandle?

    init() {
        Task { await self.connect() }
    }

    // MARK: - connect

    /// (Re)resolve an environment and spawn a fresh `engram-mcp`. Called
    /// once from `init()`; also reachable from a "retry" affordance in the
    /// empty state. Unlike every sibling model's `connect()`, this one can
    /// take real (if brief) wall-clock time and can fail for a genuinely
    /// wide range of reasons - see `MemoryConnection`'s own doc.
    func connect() async {
        connection = .connecting
        bannerMessage = nil
        mutationNotice = nil
        selectedMemoryId = nil
        provenance = nil
        provenanceError = nil
        recallResults = []
        stats = nil
        handle = nil

        do {
            let newHandle = try await Task.detached { try MemoryHandle.discover() }.value
            handle = newHandle
            connection = .ready(
                source: newHandle.source(), engramMcpBin: newHandle.engramMcpBin(), dbPath: newHandle.dbPath(),
                agentId: newHandle.agentId())
            await refreshStats()
        } catch {
            handle = nil
            connection = Self.connectionFailure(from: error)
        }
    }

    private static func connectionFailure(from error: Error) -> MemoryConnection {
        guard let memoryError = error as? MemoryError else {
            return .connectFailed(reason: String(describing: error))
        }
        switch memoryError {
        case .NoEnvironment:
            return .noEnvironment
        case .Spawn(let reason):
            return .connectFailed(reason: reason)
        case .Io(let reason):
            return .connectFailed(reason: "io error: \(reason)")
        case .Protocol(let reason), .Rpc(_, let reason):
            // Not expected during connect (the handshake itself succeeding is
            // exactly what makes `discover()`/`connect()` return `Ok` - a
            // malformed-handshake failure would already be `.Spawn`-shaped
            // from the connector's own retry-free spawn path), but handled
            // honestly rather than assumed impossible - mirrors
            // `PolicyModel.connectionFailure`'s own defensive default arm.
            return .connectFailed(reason: reason)
        case .Tool(let message):
            return .connectFailed(reason: message)
        case .Timeout(let seconds):
            return .connectFailed(reason: "engram-mcp did not answer within \(Int(seconds))s")
        }
    }

    // MARK: - stats (auto-refreshed - see the type doc)

    func refreshStats() async {
        guard let handle else { return }
        isLoadingStats = true
        defer { isLoadingStats = false }
        do {
            stats = try await Task.detached { try handle.stats(agentId: nil) }.value
            statsLoadedAt = Date()
        } catch {
            present(error)
        }
    }

    // MARK: - recall (on demand only - see the type doc)

    @discardableResult
    func recall() async -> Bool {
        guard let handle else { return false }
        let query = recallQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else {
            bannerMessage = "Enter a recall query first."
            return false
        }
        isRecalling = true
        defer {
            isRecalling = false
            hasRecalledOnce = true
        }
        bannerMessage = nil
        let mode = recallMode.rawValue
        let limit = UInt32(clamping: max(1, recallLimit))
        do {
            recallResults = try await Task.detached {
                try handle.recall(query: query, limit: limit, mode: mode, agentId: nil)
            }.value
            lastRecallAt = Date()
            return true
        } catch {
            present(error)
            return false
        }
    }

    // MARK: - why (on demand, selecting a recall result - see the type doc)

    @discardableResult
    func why(_ memoryId: String) async -> Bool {
        guard let handle else { return false }
        selectedMemoryId = memoryId
        provenance = nil
        provenanceError = nil
        isLoadingProvenance = true
        defer { isLoadingProvenance = false }
        do {
            provenance = try await Task.detached { try handle.why(memoryId: memoryId) }.value
            return true
        } catch {
            provenanceError = Self.presentableMessage(error)
            return false
        }
    }

    // MARK: - forget (irreversible - the view guards this behind a confirm step)

    /// Permanently erase `memoryId`. Only ever called after the operator has
    /// confirmed (the Swift view's own guard, docs/PHASE4.md W2: "guarded as
    /// irreversible") - this method itself does not re-confirm, mirroring
    /// `IdentityModel.rescan`'s own "the ceremony lives in the view" split.
    @discardableResult
    func forget(_ memoryId: String) async -> Bool {
        guard let handle else { return false }
        isForgetting = true
        defer { isForgetting = false }
        do {
            let result = try await Task.detached { try handle.forget(memoryId: memoryId) }.value
            mutationNotice = "Forgot \(result.kind) memory \(result.id)."
            // The forgotten memory is gone - drop it from every place this
            // model still shows it, rather than leaving a stale row/card an
            // operator could mistake for still-live data.
            recallResults.removeAll { $0.id == memoryId }
            if selectedMemoryId == memoryId {
                selectedMemoryId = nil
                provenance = nil
                provenanceError = nil
            }
            await refreshStats()
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
        guard let memoryError = error as? MemoryError else {
            bannerMessage = String(describing: error)
            return
        }
        switch memoryError {
        case .NoEnvironment:
            connection = .noEnvironment
        default:
            bannerMessage = Self.presentableMessage(error)
        }
    }

    /// Render any error (typed `MemoryError` or not) as one human line,
    /// without mutating panel state - used by `why`, whose failure is scoped
    /// to the Provenance card rather than the whole panel.
    private static func presentableMessage(_ error: Error) -> String {
        guard let memoryError = error as? MemoryError else {
            return String(describing: error)
        }
        switch memoryError {
        case .NoEnvironment:
            return "No memory plane found."
        case .Spawn(let reason):
            return "Could not start engram-mcp: \(reason)"
        case .Io(let reason):
            return "engram-mcp io error: \(reason)"
        case .Protocol(let reason):
            return "Could not parse engram-mcp output: \(reason)"
        case .Rpc(let code, let message):
            return "engram-mcp rpc error \(code): \(message)"
        case .Tool(let message):
            return message
        case .Timeout(let seconds):
            return "engram-mcp timed out after \(Int(seconds))s"
        }
    }
}
