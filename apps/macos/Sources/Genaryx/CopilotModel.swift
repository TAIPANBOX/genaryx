import Foundation
import GenaryxCoreFFI
import Observation

/// One turn of the Copilot conversation - either the operator's own question
/// or Felyx's reply. `toolTrace`/`promptTokens`/`completionTokens` are only
/// ever non-empty/non-zero on an `.assistant` message (mirrors
/// `genaryx_copilot::Answer`'s own shape: a question has no trace or usage of
/// its own).
struct CopilotMessage: Identifiable {
    enum Role {
        case user
        case assistant
    }

    let id = UUID()
    let role: Role
    let text: String
    var toolTrace: [CopilotToolDto] = []
    var promptTokens: UInt32 = 0
    var completionTokens: UInt32 = 0
}

/// Live Copilot state for the SwiftUI shell: owns a `CopilotHandle`
/// (constructed once at `connect()`), a fetched-once `status` (the residency
/// banner's data), and the running `messages` transcript. Unlike
/// `IdentityModel`/`CryptoModel`, this has no `Connection` enum with a
/// `NoEnvironment`/`ready` split - `CopilotHandle.create()` always succeeds
/// against the C0 default (`provider = "none"`; see
/// `crates/ffi/src/copilot/mod.rs`'s own module doc, "Simpler than every
/// other handle: no environment to discover"), so the only distinct failure
/// this model can reach at connect time is a genuine local construction
/// problem, surfaced through the plain `bannerMessage` like every other
/// model's own transient-error banner.
///
/// `CopilotHandle`'s exported methods are synchronous and can block:
/// `ask(question:)` runs the agent loop on the handle's own owned Tokio
/// runtime (`crates/ffi/src/copilot/mod.rs`: "`ask` is async... this handle
/// still owns a `tokio::runtime::Runtime` and bridges with `block_on`").
/// Every call into the handle below therefore runs inside `Task.detached`,
/// off this model's `@MainActor` isolation, exactly like
/// `IdentityModel`/`PolicyModel`/`CryptoModel` - see `PolicyModel.swift`'s
/// own doc for the full rationale. `CopilotHandle` is generated as
/// `@unchecked Sendable`, so capturing it into a detached task's closure is
/// safe. `status()` is the one exception: it never blocks (a plain
/// in-memory read, no I/O - see the handle's own doc comment), so it is
/// called directly on the main actor rather than detached.
@MainActor
@Observable
final class CopilotModel {
    private(set) var status: CopilotStatusDto?
    private(set) var messages: [CopilotMessage] = []
    private(set) var isSending = false
    /// A construction-time failure only - mirrors every other model's own
    /// `bannerMessage`, but `ask`'s own failures never land here: they are
    /// appended into `messages` as an assistant-role explanation instead
    /// (docs/PHASE6.md C0-W2's chat pane keeps every outcome, including "no
    /// provider configured", inline in the transcript rather than a banner
    /// that could scroll out of context).
    private(set) var bannerMessage: String?

    private var handle: CopilotHandle?

    init() {
        Task { await self.connect() }
    }

    // MARK: - connect

    /// Build a fresh handle and fetch its status once. Called once from
    /// `init()`; also reachable from a "retry" affordance in the empty
    /// state, mirroring every other model's own `connect()`.
    func connect() async {
        bannerMessage = nil

        do {
            let newHandle = try await Task.detached { try CopilotHandle.create() }.value
            handle = newHandle
            status = newHandle.status()
        } catch {
            handle = nil
            status = nil
            bannerMessage = describe(error)
        }
    }

    // MARK: - ask

    /// Append the operator's question, run it through Felyx, and append the
    /// reply - or, when `handle` is unavailable or `ask` fails, an
    /// assistant-role message explaining why (including the honest "no
    /// provider configured" outcome - `ask`'s NORMAL result against the C0
    /// default, never a bug). Blank/whitespace-only input is a silent no-op,
    /// same convention `CryptoModel.runScan`/`MemoryModel.recall` use for an
    /// empty required field.
    func send(_ text: String) async {
        let question = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !question.isEmpty else { return }

        guard let handle else {
            messages.append(
                CopilotMessage(
                    role: .assistant,
                    text: bannerMessage.map { "Felyx is not available: \($0)" }
                        ?? "Felyx is not available right now."))
            return
        }

        messages.append(CopilotMessage(role: .user, text: question))
        isSending = true
        defer { isSending = false }

        do {
            let answer = try await Task.detached { try handle.ask(question: question) }.value
            messages.append(
                CopilotMessage(
                    role: .assistant, text: answer.text, toolTrace: answer.toolTrace,
                    promptTokens: answer.promptTokens, completionTokens: answer.completionTokens))
        } catch {
            messages.append(CopilotMessage(role: .assistant, text: describe(error)))
        }
    }

    // MARK: - explain (C1 "Explain with Felyx" affordance)

    /// Ask Felyx to explain one incident (docs/PHASE6-C1.md): seeds the
    /// transcript with a synthetic user-role question naming the incident,
    /// then runs `CopilotHandle.explain` (the focused `explain_incident`
    /// flow over the money/policy/identity planes plus memory) and appends
    /// the reply - mirrors `send(_:)` exactly (same detached-task bridge,
    /// same "handle unavailable" / thrown-error handling) so a click from
    /// the Incidents rail lands in the Copilot tab looking like a normal
    /// conversation turn, not a bespoke one-off rendering. The caller (see
    /// `GenaryxApp.explainIncident(_:)`) is expected to switch to the
    /// Copilot tab alongside calling this.
    func explain(incidentId: String) async {
        guard let handle else {
            messages.append(
                CopilotMessage(
                    role: .assistant,
                    text: bannerMessage.map { "Felyx is not available: \($0)" }
                        ?? "Felyx is not available right now."))
            return
        }

        messages.append(CopilotMessage(role: .user, text: "Explain incident \(incidentId)"))
        isSending = true
        defer { isSending = false }

        do {
            let answer = try await Task.detached { try handle.explain(incidentId: incidentId) }.value
            messages.append(
                CopilotMessage(
                    role: .assistant, text: answer.text, toolTrace: answer.toolTrace,
                    promptTokens: answer.promptTokens, completionTokens: answer.completionTokens))
        } catch {
            messages.append(CopilotMessage(role: .assistant, text: describe(error)))
        }
    }

    // MARK: - error presentation

    /// Fold any thrown error into display text. `.NoProvider` prefers the
    /// fuller `status.disabledReason` (the same sentence the residency
    /// banner already shows) over the terser Rust `Display` string, so the
    /// transcript and the banner never disagree about WHY Felyx declined to
    /// answer.
    private func describe(_ error: Error) -> String {
        guard let copilotError = error as? CopilotFfiError else {
            return String(describing: error)
        }
        switch copilotError {
        case .NoProvider:
            return status?.disabledReason ?? "No copilot provider is configured."
        case .Config(let reason):
            return "Copilot config error: \(reason)"
        case .Failed(let reason):
            return "Copilot request failed: \(reason)"
        }
    }
}
