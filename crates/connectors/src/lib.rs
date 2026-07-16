//! genaryx-connectors: environment and service connectors.
//!
//! Phase-0 placeholder. Planned impls (07 §6): Local FS (tail + descriptor
//! autodiscovery), SSH/VPS (tunnels + remote tail, host-key pinning), Cloud SSE
//! (`/v1/stream`), Hetzner/AWS/GCP read-only inventory, MCP client/server. The
//! `EventSource` trait these implement lives in `genaryx_core::ingest`.

/// Marker for the connectors crate; real connectors land in F1+.
pub const CRATE: &str = "genaryx-connectors";
