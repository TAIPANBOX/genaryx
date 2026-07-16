import Foundation

/// UI-facing mirror of the UI-relevant fields of `genaryx_core::store::StoredEvent`
/// (`crates/core/src/store.rs`). Field names, including the trailing
/// underscore on `type_`, match the Rust struct exactly (Rust reserves
/// `type`, hence `type_`), so this reads as a 1:1 correspondence rather than
/// a loose paraphrase.
///
/// UNIFFI BRIDGE POINT: this is Phase 0 hand-written scaffolding, populated
/// by `MockData.swift`. The follow-up task adds a UniFFI binding over
/// `genaryx-core`, and this struct is deleted in favor of the generated
/// Swift type for `StoredEvent`; `BusExplorerView` then reads from a live
/// stream fed by `IngestService::subscribe()` (a
/// `tokio::sync::broadcast::Receiver<ConsoleEvent>` bridged across the FFI
/// boundary, most likely via a callback interface or an async sequence
/// wrapper) instead of `MockData.events`.
///
/// Omitted relative to the full Rust struct: `env`, `data` (the parsed JSON
/// payload; `raw` already carries it as text), `prev_hash`, `file`, and
/// `off`. Add them here, matched to the generated binding's names, if a
/// future view needs them.
struct UiEvent: Identifiable, Hashable, Sendable {
    /// `events.id`: the SQLite rowid, monotonically increasing insert order.
    let id: Int64
    /// `events.ts`: RFC 3339 timestamp, as stored.
    let ts: String
    /// `events.source`: the emitting service, one of the six bus sources.
    let source: String
    /// `events.type`: the event type within `source`.
    let type_: String
    /// `events.agent_id`: an `agent://taipanbox.dev/...` identifier.
    let agentId: String
    /// `events.run_id`: run correlation id, when the event belongs to one.
    let runId: String?
    /// `events.severity`: info, low, medium, high, or critical, when present.
    let severity: String?
    /// `events.schema`: the schema version string, e.g. "taipanbox.dev/agent-event/v0.2".
    let schema: String
    /// `events.on_behalf_of`: delegation chain, root-first; empty when the
    /// event was not delegated.
    let onBehalfOf: [String]
    /// `events.raw`: the original NDJSON line, verbatim.
    let raw: String
}
