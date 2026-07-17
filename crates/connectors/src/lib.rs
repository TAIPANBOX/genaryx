//! genaryx-connectors: environment and service connectors.
//!
//! Phase-0 status (06 §7 spike #6): [`CloudSse`] is implemented and proven
//! against a local mock server plus direct decoder unit tests; see
//! `docs/PHASE0.md` spike row 6. Other planned impls (07 §6): Local FS (tail +
//! descriptor autodiscovery; `FileTail` itself already lives in
//! `genaryx_core::ingest`), SSH/VPS (tunnels + remote tail, host-key pinning),
//! Hetzner/AWS/GCP read-only inventory, MCP client/server. The `EventSource`
//! trait these implement lives in `genaryx_core::ingest`.

mod cloud_sse;
mod sse_decoder;

pub use cloud_sse::{CloudSse, CloudSseConfig};
pub use sse_decoder::{SseDecoder, SseEvent};

/// Marker for the connectors crate; real connectors land in F1+.
pub const CRATE: &str = "genaryx-connectors";
