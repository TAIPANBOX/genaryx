//! genaryx-connectors: environment and service connectors.
//!
//! Phase-0 status (06 §7 spike #6): [`CloudSse`] is implemented and proven
//! against a local mock server plus direct decoder unit tests; see
//! `docs/PHASE0.md` spike row 6. Other planned impls (07 §6): Local FS (tail +
//! descriptor autodiscovery; `FileTail` itself already lives in
//! `genaryx_core::ingest`), SSH/VPS (tunnels + remote tail, host-key pinning),
//! Hetzner/AWS/GCP read-only inventory, MCP client/server. The `EventSource`
//! trait these implement lives in `genaryx_core::ingest`.
//!
//! Phase-1 wave 1 (docs/PHASE1.md): [`CloudClient`] is the Cloud REST
//! connector - typed reads plus ES256-signed mutations - the Money panel
//! renders from. It reuses [`CloudSse`] for the live ticker and
//! `genaryx-signing`'s `es256` module for signing; see
//! `crates/connectors/src/cloud_rest.rs`.

mod cloud_rest;
mod cloud_sse;
mod sse_decoder;

pub use cloud_rest::{
    AckResponse, AgentAgg, Alert, AuditVerifyResponse, BudgetResponse, CloudClient, ConnectorError,
    Incident, KillResponse, PairResponse, RunAgg, SavingsSummary, Severity, Summary,
};
pub use cloud_sse::{CloudSse, CloudSseConfig};
pub use sse_decoder::{SseDecoder, SseEvent};

/// Marker for the connectors crate; real connectors land in F1+.
pub const CRATE: &str = "genaryx-connectors";
