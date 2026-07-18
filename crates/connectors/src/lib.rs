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
//!
//! Phase-4 wave 1 (docs/PHASE4.md): [`VerdryxClient`] and [`QryxClient`] are the
//! quality- and crypto-plane connectors the console's Quality and Crypto panels
//! render from. They diverge on transport because the services do: Verdryx is a
//! Python batch CLI with NO machine output, so [`VerdryxClient`] is a strictly
//! read-only SQLite reader over `verdryx.db`; Qryx is a Go CLI whose machine
//! surface IS `--format`, so [`QryxClient`] is a `ToolRunner` that shells it and
//! parses the JSON (`ncsc`/`cbom`/`evidence`), including the ML-DSA-signed
//! evidence bundles the Evidence Center reuses. See
//! `crates/connectors/src/{verdryx,qryx}.rs`.
//!
//! Phase-4 wave 2 (docs/PHASE4.md): the memory- and drills-plane connectors.
//! [`EngramClient`] is the console's FIRST MCP-client connector: it speaks the
//! Model Context Protocol over a stdio child (`engram-mcp`) via the generic,
//! reusable [`McpStdioClient`] transport (newline-delimited JSON-RPC 2.0, the
//! `initialize` handshake, fail-closed with a deadline), and types engram's
//! `stats`/`recall`/`why`/`forget` tools for the Memory panel.
//! [`MockryxClient`] is a `ToolRunner` over the mockryx fire-drill CLI
//! (`mockryx run --format json`, exit `0|1` both yield a report, `2` is a real
//! error) the Drills panel runs and renders. See
//! `crates/connectors/src/{mcp_stdio,engram,mockryx}.rs`.
//!
//! Phase-5 wave 2 (docs/PHASE5.md W2): [`RelayAdminClient`] is the desktop
//! Pocket panel's connector to `genaryx-relay`'s loopback/WG-only admin API
//! (pairing-info, pairing-window arm, paired-device view, disconnect) - see
//! `crates/connectors/src/relay_admin.rs`. [`CloudClient::pair_new`] is a
//! small addition alongside it: the mint-only half of the existing
//! [`CloudClient::pair`] flow, since the Pocket panel mints a code for a
//! PHONE to redeem later (over the relay) rather than redeeming it itself.
//!
//! Phase-4 wave 3 (docs/PHASE4.md W3): the Evidence Center. [`build_evidence_pack`]
//! is the ONE function both shells call to assemble a signed evidence zip - it
//! gathers Cloud compliance evidence + the audit verdict
//! ([`CloudClient::compliance_evidence`]/`audit_verify`), Qryx crypto evidence +
//! CBOM captured VERBATIM ([`QryxClient::scan_evidence_raw`]/`scan_cbom_raw`, so
//! their embedded self-verify survives), the idryx Agent-BOM
//! ([`IdryxClient::agent_bom`]), and the TokenFuse FOCUS cost CSV (the new
//! [`TokenfuseClient`]), then signs the manifest ES256
//! ([`CloudClient::sign_evidence_manifest`]) and hands the whole set to
//! `genaryx_core::evidence::assemble_zip`. Missing sources are recorded, not
//! dropped; signing is fail-closed. See `crates/connectors/src/evidence.rs`.

mod cloud_rest;
mod cloud_sse;
mod engram;
mod evidence;
mod hetzner;
mod idryx;
mod mcp_stdio;
mod mockryx;
mod qryx;
mod relay_admin;
mod sse_decoder;
mod ssh;
mod tokenfuse;
mod verdryx;
mod wardryx;
mod wg;

pub use cloud_rest::{
    AckResponse, AgentAgg, Alert, AuditVerifyResponse, BudgetResponse, CloudClient, ConnectorError,
    Incident, KillResponse, PairNewResponse, PairResponse, RunAgg, SavingsSummary, Severity,
    Summary,
};
pub use cloud_sse::{CloudSse, CloudSseConfig};
pub use engram::{
    EngramClient, EngramCounts, EngramMemory, EngramProvenance, EngramStats,
    ForgetResult as EngramForgetResult,
};
pub use evidence::{EvidenceBuildError, EvidenceInputs, EvidencePack, build_evidence_pack};
pub use hetzner::{HetznerClient, HetznerError, HetznerServer};
pub use idryx::{
    Alert as IdryxAlert, Identity as IdryxIdentity, IdryxClient, IdryxError,
    Permission as IdryxPermission, Recommendation as IdryxRecommendation,
    Remediation as IdryxRemediation,
};
pub use mcp_stdio::{McpError, McpStdioClient, ToolDef as McpToolDef};
pub use mockryx::{
    MockryxClient, MockryxError, MockryxFinding, MockryxMetrics, MockryxReport, MockryxResult,
};
pub use qryx::{
    EvidenceReport, EvidenceSummary, NcscDiscovery, NcscFinding, NcscFullMigration, NcscPriority,
    NcscReport, QryxClient, QryxError, Signature as QryxSignature, VerifyOutcome,
};
pub use relay_admin::{
    ArmPairingWindowResponse as RelayArmPairingWindowResponse, DeviceView as RelayDeviceView,
    DisconnectResponse as RelayDisconnectResponse, PairingInfo as RelayPairingInfo,
    RelayAdminClient, RelayAdminError,
};
pub use sse_decoder::{SseDecoder, SseEvent};
pub use ssh::{SshClient, SshError, SshTarget};
pub use tokenfuse::{TokenfuseClient, TokenfuseError};
pub use verdryx::{
    Baseline as VerdryxBaseline, EvalRun as VerdryxEvalRun, RunSummary as VerdryxRunSummary,
    Score as VerdryxScore, VerdryxClient, VerdryxError,
};
pub use wardryx::{
    Approval, ApprovalDecideResponse, ApprovalTokenClaims, ApprovalVerdict, DecideRequest,
    DecideResponse, Policy, PolicyRecord, WardryxClient, WardryxError,
};
pub use wg::{WgConfig, WgError, WgInterfaceAddr, WgKeypair, WgPeer, WgTunnel};

/// Marker for the connectors crate; real connectors land in F1+.
pub const CRATE: &str = "genaryx-connectors";

/// Deserialize a collection field a Go tool may emit as JSON `null` for an
/// empty value. Go's `encoding/json` marshals a nil slice/map as `null`, not
/// `[]`/`{}`, so a `--format`-emitting Go CLI (Qryx, Mockryx) sends `null` for
/// an empty `findings`/`coverageBySource`/`results` that lacks an `omitempty`
/// tag. A plain `Vec<T>`/`BTreeMap<K,V>` rejects that `null` even with
/// `#[serde(default)]` (which only covers an ABSENT key, not a present `null`).
/// Pair this with `#[serde(default)]` to map absent, `null`, and a real array
/// all to `T::default()`. This is exactly the "type written against an
/// assumption, not the real bytes" class the Engram live testing surfaced; the
/// qryx/mockryx live-shape tests (`tests/live_shapes_test.rs`) guard it.
pub(crate) fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::Deserialize<'de>,
{
    Ok(<Option<T> as serde::Deserialize>::deserialize(d)?.unwrap_or_default())
}
