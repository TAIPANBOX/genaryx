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
//!
//! Phase-2 wave 1 (07 §4.3): [`WardryxClient`] is the Wardryx policy-plane
//! REST connector - the PDP (`/v1/decide`), the approvals inbox
//! (`/v1/approvals[...]`), and the admin policy-as-code routes
//! (`/v1/policies[...]`) the console's policy panel renders from and acts
//! through. Bearer-only auth (no device/signing, unlike `CloudClient`); see
//! `crates/connectors/src/wardryx.rs`.
//!
//! Phase-3 wave 1 (07 §4.4): [`IdryxClient`] is the Idryx identity-plane
//! connector the Identity panel and Agent 360 render from - a REST snapshot
//! over `idryx serve` (`/api/identities`, `/api/alerts`, `/api/remediations`)
//! plus an `idryx detect --format json` Rescan. Unauthenticated by design, and
//! load-once, so the live delegation graph is genaryx-core's job (bus-fed), not
//! this snapshot; see `crates/connectors/src/idryx.rs`.

mod cloud_rest;
mod cloud_sse;
mod idryx;
mod sse_decoder;
mod wardryx;

pub use cloud_rest::{
    AckResponse, AgentAgg, Alert, AuditVerifyResponse, BudgetResponse, CloudClient, ConnectorError,
    Incident, KillResponse, PairResponse, RunAgg, SavingsSummary, Severity, Summary,
};
pub use cloud_sse::{CloudSse, CloudSseConfig};
pub use idryx::{
    Alert as IdryxAlert, Identity as IdryxIdentity, IdryxClient, IdryxError,
    Permission as IdryxPermission, Recommendation as IdryxRecommendation,
    Remediation as IdryxRemediation,
};
pub use sse_decoder::{SseDecoder, SseEvent};
pub use wardryx::{
    Approval, ApprovalDecideResponse, ApprovalTokenClaims, ApprovalVerdict, DecideRequest,
    DecideResponse, Policy, PolicyRecord, WardryxClient, WardryxError,
};

/// Marker for the connectors crate; real connectors land in F1+.
pub const CRATE: &str = "genaryx-connectors";
