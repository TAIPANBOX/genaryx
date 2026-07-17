//! Wire DTOs and error taxonomy for [`super::IdryxHandle`], mirroring
//! `crates/ffi/src/wardryx/dto.rs`'s shape (UniFFI `Record`/`Error` types
//! instead of `genaryx_connectors::idryx`'s plain Rust structs) but over the
//! Idryx contract (docs/PHASE3.md "Grounded Idryx contract").
//!
//! `genaryx_connectors` re-exports its Idryx types already `Idryx`-prefixed
//! (`Identity as IdryxIdentity`, `Alert as IdryxAlert`, ...) to avoid
//! colliding with its own `Cloud`/`Wardryx` sibling types; imported here
//! under a `Conn` prefix anyway (mirroring `wardryx/dto.rs`'s own
//! `ConnApproval`/`ConnPolicyRecord` convention), since this module defines
//! its own same-shaped [`IdentityRecord`], [`AlertRecord`],
//! [`RemediationRecord`], and [`IdryxError`] as the UniFFI-facing
//! counterparts.

use genaryx_connectors::{
    IdryxAlert as ConnAlert, IdryxError as ConnIdryxError, IdryxIdentity as ConnIdentity,
    IdryxRecommendation as ConnRecommendation, IdryxRemediation as ConnRemediation,
};

// ============================================================================
// DTOs
// ============================================================================

/// One row of the Identities list: exact field set of `GET /api/identities`'
/// wire shape (`genaryx_connectors::IdryxIdentity`, itself `apiIdentity`,
/// `server.go:119-134`), flattened into owned fields (UniFFI Records cannot
/// borrow). Deliberately carries NO `attestation` field - the wire contract
/// has none; see [`super`]'s module doc "Attestation is never a field".
/// The permission ARN is never exposed by the server at all (`apiPermission`
/// carries only `name`/`admin`/`used`), and the panel's own row spec
/// (docs/PHASE3.md W2) only ever wants a permission COUNT, not the list - so
/// this carries [`Self::permission_count`] / [`Self::admin_permission_count`]
/// rather than a nested `Vec<Permission>` a caller would just count anyway.
#[derive(Debug, Clone, uniffi::Record)]
pub struct IdentityRecord {
    pub id: String,
    /// `human | service_account | key | agent | mcp_server` - the server
    /// already defaults an empty type to the literal `"human"`
    /// (`server.go:163-166`), so this is never blank on a real payload.
    /// Kept as a raw `String` rather than a UniFFI enum (mirrors
    /// `ApprovalRecord.decision`'s own choice in `wardryx/dto.rs`): this is
    /// server-emitted data, not a closed set of client choices, so a client
    /// enum would just be one more place a new idryx identity type has to be
    /// taught to this shell before it renders at all.
    pub identity_type: String,
    pub privileged: bool,
    /// The connector/source name, e.g. `aws_iam`, `agents`, `mcp`, `okta`,
    /// `tokenfuse`, `wardryx`.
    pub source: String,
    pub owner: String,
    /// `"YYYY-MM-DD HH:MM:SS UTC"`, `None` when idryx never set it (the wire
    /// field is `""` when zero - a different format from [`AlertRecord::time`]).
    pub created: Option<String>,
    pub last_used: Option<String>,
    pub runtime: Option<String>,
    /// The delegation chain, root-first, max depth 32 (agent-passport SPEC
    /// §5); empty when idryx recorded none.
    pub on_behalf_of: Vec<String>,
    pub permission_count: u32,
    /// Of [`Self::permission_count`], how many are admin-capable
    /// (`apiPermission.admin`) - the signal an over-privileged-identity
    /// filter would key off, without exposing the underlying ARNs idryx
    /// itself withholds.
    pub admin_permission_count: u32,
    /// A right-sizing suggestion, `identity` always equal to [`Self::id`]
    /// (see [`RemediationRecord`]'s own doc for why that field exists here
    /// too, not just on [`super::IdryxHandle::list_remediations`]'s rows).
    pub remediation: Option<RemediationRecord>,
    /// A rotation suggestion, same shape as [`Self::remediation`].
    pub rotation: Option<RemediationRecord>,
    /// COUNT of this identity's events, NOT the objects (`server.go:200`) -
    /// docs/PHASE3.md W2: "label them as counts, not objects".
    pub events: u64,
    /// COUNT of alerts on this identity, NOT the objects (`server.go:201`).
    pub alerts: u64,
}

impl From<&ConnIdentity> for IdentityRecord {
    fn from(a: &ConnIdentity) -> Self {
        Self {
            id: a.id.clone(),
            identity_type: a.identity_type.clone(),
            privileged: a.privileged,
            source: a.source.clone(),
            owner: a.owner.clone(),
            created: non_empty(&a.created),
            last_used: non_empty(&a.last_used),
            runtime: non_empty(&a.runtime),
            on_behalf_of: a.on_behalf_of.clone(),
            permission_count: a.permissions.len() as u32,
            admin_permission_count: a.permissions.iter().filter(|p| p.admin).count() as u32,
            remediation: a
                .remediation
                .as_ref()
                .map(|r| RemediationRecord::from_identity(&a.id, r)),
            rotation: a
                .rotation
                .as_ref()
                .map(|r| RemediationRecord::from_identity(&a.id, r)),
            events: a.events,
            alerts: a.alerts,
        }
    }
}

/// One row of the Alerts stream: exact field set of `GET /api/alerts` /
/// `idryx detect --format json` (`genaryx_connectors::IdryxAlert`, itself
/// `apiAlert`/`jsonAlert`, byte-identical wire shapes - see
/// `IdryxClient::rescan`'s own doc comment). `detector` is one of the 21 ids
/// (docs/PHASE3.md: `impossible_travel`, ..., `unmanaged_egress`); `severity`
/// is `critical|high|medium|low|info|none`, dynamic per detector, so the
/// panel filters on `detector` AND `severity`, never a hard-coded per-detector
/// severity. `summary` is free text - for `attestation_missing` it embeds
/// `attestation=<value>`, the only place attestation status ever reaches this
/// client (see [`super`]'s module doc).
#[derive(Debug, Clone, uniffi::Record)]
pub struct AlertRecord {
    pub detector: String,
    /// Joins to [`IdentityRecord::id`].
    pub identity: String,
    pub severity: String,
    /// `"YYYY-MM-DDTHH:MM:SSZ"` (UTC, no fractional) - a different format
    /// from [`IdentityRecord::created`].
    pub time: String,
    pub summary: String,
}

impl From<&ConnAlert> for AlertRecord {
    fn from(a: &ConnAlert) -> Self {
        Self {
            detector: a.detector.clone(),
            identity: a.identity.clone(),
            severity: a.severity.clone(),
            time: a.time.clone(),
            summary: a.summary.clone(),
        }
    }
}

/// A right-size/rotation suggestion: the shared shape of `GET
/// /api/remediations`' rows (`genaryx_connectors::IdryxRecommendation`,
/// `apiRecommendation`) AND of [`IdentityRecord::remediation`]/
/// [`IdentityRecord::rotation`] (`genaryx_connectors::IdryxRemediation`,
/// `apiRemediation` - the same Go struct serves both call sites, `server.go`
/// doc comment). One UniFFI Record covers both: `identity` is always
/// populated (never `Option`), even when this is embedded inside an
/// `IdentityRecord` that already names the same id - simpler than adding a
/// second, nearly-identical Record just to drop one field, and it keeps a
/// `RemediationRecord` self-describing wherever the Swift shell encounters
/// one on its own (e.g. a flattened list built from several identities).
#[derive(Debug, Clone, uniffi::Record)]
pub struct RemediationRecord {
    pub identity: String,
    /// `"right_size"` or `"rotation"`.
    pub kind: String,
    pub explanation: String,
    pub code: String,
    pub created_at: Option<String>,
}

impl RemediationRecord {
    /// Build from an [`IdentityRecord`]'s own embedded `remediation`/
    /// `rotation` field, which the wire shape carries with no `identity` of
    /// its own - the caller already knows it (its parent identity's id).
    fn from_identity(identity: &str, r: &ConnRemediation) -> Self {
        Self {
            identity: identity.to_string(),
            kind: r.kind.clone(),
            explanation: r.explanation.clone(),
            code: r.code.clone(),
            created_at: non_empty(&r.created_at),
        }
    }
}

impl From<&ConnRecommendation> for RemediationRecord {
    fn from(r: &ConnRecommendation) -> Self {
        Self {
            identity: r.identity.clone(),
            kind: r.kind.clone(),
            explanation: r.explanation.clone(),
            code: r.code.clone(),
            created_at: non_empty(&r.created_at),
        }
    }
}

// ============================================================================
// error taxonomy
// ============================================================================

/// Every failure mode an [`super::IdryxHandle`] call can surface, fail-closed
/// throughout (06 §0.5: no panics/unwraps cross the FFI boundary). Mirrors
/// [`crate::wardryx::WardryxError`]'s role and shape, collapsed from
/// `genaryx_connectors::IdryxError`'s variants, plus two ffi-layer-only
/// additions ([`Self::NoEnvironment`], [`Self::RescanUnavailable`]) that have
/// no connector-level equivalent because they are about resolving an
/// environment, not about a call the connector itself made.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum IdryxError {
    /// [`super::env::discover`] found nothing usable: no `taipan up`
    /// descriptor with an `idryx` service, and no `IDRYX_URL`.
    #[error("no Idryx identity plane found (no taipan up descriptor, no IDRYX_URL)")]
    NoEnvironment,
    /// An environment resolved (or was given explicitly via `connect`), but
    /// building the client or the local async runtime failed. Never a
    /// network failure: idryx has no pairing/auth handshake at all (see
    /// `super`'s module doc), so this is reachable only via a local
    /// runtime/resource problem.
    #[error("could not set up the Idryx connection: {reason}")]
    ConnectFailed { reason: String },
    /// Any non-2xx REST response: the status and raw body text. Idryx's own
    /// JSON handlers ignore the request and always answer 200 with an array
    /// (`IdryxClient`'s own doc comment), so in practice this only surfaces
    /// from a stray path or a transport-adjacent gateway error.
    #[error("idryx returned HTTP {status}: {body}")]
    Api { status: u16, body: String },
    /// The request never got a response (DNS, connect, timeout, or a body
    /// that failed to read).
    #[error("could not reach idryx: {reason}")]
    Transport { reason: String },
    /// A 2xx REST body that failed to deserialize into the expected shape.
    #[error("unexpected response shape from idryx: {reason}")]
    Json { reason: String },
    /// [`super::env::resolve_rescan_inputs`] could not find what `Rescan`
    /// needs (the idryx binary, or a taipan environment descriptor's event
    /// files) - a clean, honest "cannot recompute right now", never a fake
    /// empty `Vec<AlertRecord>` dressed up as "no findings" (docs/PHASE3.md
    /// W2: "never a fake empty success").
    #[error("Rescan unavailable: {reason}")]
    RescanUnavailable { reason: String },
    /// `idryx detect` spawned but exited nonzero, or failed to spawn at all.
    /// Idryx's exit code does not signal findings (0 whether or not there
    /// are alerts; only a real error - bad flags, a parse failure - exits
    /// 1), so a nonzero exit here is a genuine failure, carrying idryx's own
    /// stderr.
    #[error("idryx detect failed: {reason}")]
    RescanFailed { reason: String },
}

impl From<ConnIdryxError> for IdryxError {
    fn from(e: ConnIdryxError) -> Self {
        match e {
            ConnIdryxError::Api { status, body } => IdryxError::Api { status, body },
            ConnIdryxError::Transport(err) => IdryxError::Transport {
                reason: err.to_string(),
            },
            ConnIdryxError::Json(err) => IdryxError::Json {
                reason: err.to_string(),
            },
            ConnIdryxError::Cli(reason) => IdryxError::RescanFailed { reason },
        }
    }
}

// ============================================================================
// helpers
// ============================================================================

/// Collapse idryx's `#[serde(default)]` empty-string convention
/// (`created`/`last_used`/`runtime`/`created_at` are `""` when idryx never
/// set them, never absent from the JSON entirely) into an honest `Option`,
/// so the Swift shell can use `if let` instead of every call site re-checking
/// `.isEmpty` itself.
fn non_empty(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}
