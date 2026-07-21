//! Wire DTOs and error taxonomy for [`super::WardryxHandle`], mirroring
//! `crates/ffi/src/cloud/dto.rs`'s shape (UniFFI `Record`/`Error` types
//! instead of `genaryx_connectors::wardryx`'s plain Rust structs) but over
//! the Wardryx contract (docs/PHASE2.md "Wave 2 data contract + UX").
//!
//! `genaryx_connectors`'s Wardryx types are imported under a `Conn` prefix
//! throughout (mirroring `cloud/dto.rs`'s own `Api`-prefixed aliases:
//! `Incident as ApiIncident`, `Severity as ApiSeverity`) since this module
//! defines its own same-named [`ApprovalVerdict`], [`WardryxError`], and
//! [`PolicyRecord`] as the UniFFI-facing counterparts.

use genaryx_connectors::{
    Approval as ConnApproval, ApprovalDecideResponse as ConnApprovalDecideResponse,
    ApprovalTokenClaims, ApprovalVerdict as ConnApprovalVerdict, PolicyRecord as ConnPolicyRecord,
    WardryxError as ConnWardryxError,
};
use std::time::SystemTime;

// ============================================================================
// DTOs
// ============================================================================

/// The Approvals Inbox row shape: `genaryx_connectors::Approval`'s untyped
/// `context` map flattened into typed fields (docs/PHASE2.md wave 2, "full
/// context pulled from `context`: who (`agent_id` + the `on_behalf_of`
/// chain), what (`tool_names`), how much (`est_cost_usd`), why (`reason`),
/// when (`requested_at`), `policy_version`"). Also carries the plain
/// top-level fields (`approval_id`, `pending`, `decision`, `decided_by`,
/// `decided_at`) so one record serves both the pending queue and the decided
/// history list (PHASE2.md: "Decided approvals move to a history list
/// (`decision`, `decided_by`, `decided_at`)").
///
/// Collection-shaped context fields (`tools`, `on_behalf_of`) flatten a
/// missing/`null` context entry to an empty `Vec` rather than double-wrapping
/// in `Option`, matching [`crate::UiEvent::on_behalf_of`]'s own convention.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ApprovalRecord {
    pub approval_id: String,
    pub agent_id: String,
    pub run_id: String,
    /// RFC3339 UTC.
    pub requested_at: String,
    /// `None` while pending.
    pub decided_at: Option<String>,
    /// `None` while pending.
    pub decided_by: Option<String>,
    /// `None` while pending; `"grant"` / `"deny"` once decided.
    pub decision: Option<String>,
    pub pending: bool,
    /// `context["tool_names"]`, empty when absent.
    pub tools: Vec<String>,
    /// `context["est_cost_usd"]`.
    pub est_cost_usd: Option<f64>,
    /// `context["reason"]`.
    pub reason: Option<String>,
    /// `context["on_behalf_of"]`, the delegation chain root-first; empty
    /// when the request declared none.
    pub on_behalf_of: Vec<String>,
    /// `context["policy_version"]`.
    pub policy_version: Option<String>,
    /// `context["org"]`.
    pub org: Option<String>,
    /// `context["model"]`.
    pub model: Option<String>,
}

impl From<&ConnApproval> for ApprovalRecord {
    fn from(a: &ConnApproval) -> Self {
        Self {
            approval_id: a.approval_id.clone(),
            agent_id: a.agent_id.clone(),
            run_id: a.run_id.clone(),
            requested_at: a.requested_at.clone(),
            decided_at: a.decided_at.clone(),
            decided_by: a.decided_by.clone(),
            decision: a.decision.clone(),
            pending: a.pending,
            tools: a.tool_names().unwrap_or_default(),
            est_cost_usd: a.est_cost_usd(),
            reason: a.reason().map(str::to_string),
            on_behalf_of: a.on_behalf_of().unwrap_or_default(),
            policy_version: a.policy_version().map(str::to_string),
            org: a.org().map(str::to_string),
            model: a.model().map(str::to_string),
        }
    }
}

/// One row of the Policy view: exact field set of `GET /v1/policies`'
/// flattened wire shape (`genaryx_connectors::PolicyRecord`; see that type's
/// own doc comment for why the response is flattened, not nested under a
/// `"policy"` key). Deliberately carries NO `policy_version`: the live
/// wire contract for this route has no such field on a per-policy or
/// per-response basis (confirmed against `genaryx_connectors::PolicyRecord`,
/// a bare array of these with no wrapping object at all). The set-level
/// `policy_version` PHASE2.md's Policy view also wants to show is only ever
/// observable from a `context["policy_version"]` already carried on an
/// [`ApprovalRecord`] (or a live `/v1/decide` response, which this panel
/// never calls proactively), so the SwiftUI view composes that value from
/// whatever `ApprovalRecord`s it already has, the same "derive at the view
/// layer, never fabricate on the Rust side" rule the Decision Stream follows
/// (PHASE2.md: "Reuses the existing event pipeline... NOT a new REST read").
#[derive(Debug, Clone, uniffi::Record)]
pub struct PolicyRecord {
    /// The URL path segment this policy is addressed by; distinct from
    /// `name` below, which need not be unique.
    pub id: String,
    pub name: String,
    /// The `agent://` glob this policy targets.
    pub target: String,
    pub deny_tool: Vec<String>,
    pub allow_domains: Vec<String>,
    pub require_human_above_usd: f64,
    pub deny_above_usd: f64,
    pub max_steps: i64,
    pub deny_if_unattested: bool,
    /// RFC3339 UTC timestamp of the last write.
    pub updated_at: Option<String>,
}

impl From<&ConnPolicyRecord> for PolicyRecord {
    fn from(p: &ConnPolicyRecord) -> Self {
        Self {
            id: p.id.clone(),
            name: p.policy.name.clone(),
            target: p.policy.target.clone(),
            deny_tool: p.policy.deny_tool.clone(),
            allow_domains: p.policy.allow_domains.clone(),
            require_human_above_usd: p.policy.require_human_above_usd,
            deny_above_usd: p.policy.deny_above_usd,
            max_steps: p.policy.max_steps,
            deny_if_unattested: p.policy.deny_if_unattested,
            updated_at: p.updated_at.clone(),
        }
    }
}

/// The operator's choice on a pending approval - a UniFFI-exported mirror of
/// `genaryx_connectors::ApprovalVerdict` (that type carries no UniFFI derive,
/// and `crates/connectors` is out of scope for this wave, so this crate
/// exports its own and converts at the boundary).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ApprovalVerdict {
    Grant,
    Deny,
}

impl From<ApprovalVerdict> for ConnApprovalVerdict {
    fn from(v: ApprovalVerdict) -> Self {
        match v {
            ApprovalVerdict::Grant => ConnApprovalVerdict::Grant,
            ApprovalVerdict::Deny => ConnApprovalVerdict::Deny,
        }
    }
}

/// What [`super::WardryxHandle::decide_approval`] returns: the server's
/// verdict plus - on a grant - the DECODED (never verified; see
/// `ApprovalTokenClaims`'s own doc comment) token claims a human needs to see
/// exactly what they just authorized (PHASE2.md: "show the operator exactly
/// what they authorized: agent/run, tools, cost ceiling, expiry countdown"),
/// plus whether the attempt made it onto the local bus as a
/// `console_command`. Mirrors [`crate::cloud::MutationOutcome`]'s role.
///
/// The four claim fields are `None`/empty on a deny (a deny never mints a
/// token) and also on a grant whose token could not be decoded (never
/// reachable against a conforming server, but not a panic risk here either -
/// [`ApprovalVerdict::Grant`]'s own `granted` field still reports `true` from
/// the server's actual decision in that edge case, with `verify_result`
/// carrying the decode failure text).
#[derive(Debug, Clone, uniffi::Record)]
pub struct ApprovalDecideOutcome {
    pub approval_id: String,
    /// The server's actual verdict (`true` for `"grant"`, `false` for
    /// `"deny"`) - independent of whether this call's own journal attempt
    /// below succeeded.
    pub granted: bool,
    /// Short human summary, e.g. `"approval ap_1 granted"`.
    pub summary: String,
    /// e.g. `"granted ceiling_usd:50.00 ttl_s:600"` / `"denied"`
    /// (PHASE2.md's own examples).
    pub verify_result: String,
    /// `claims.max_cost_usd` - the ceiling `VerifyApprovalToken` will refuse
    /// to let any request's `est_cost_usd` exceed.
    pub cost_ceiling_usd: Option<f64>,
    /// Seconds remaining on the token's TTL as of this decode (effectively
    /// the full mint TTL, since decode happens immediately after grant).
    pub ttl_seconds: Option<u64>,
    /// `claims.exp`, Unix seconds - the absolute expiry, so the Swift shell
    /// can drive a live countdown from `Date` rather than a snapshot that
    /// would otherwise look frozen.
    pub expires_at_unix: Option<i64>,
    /// `claims.tools`, pre-sorted by the server; empty on a deny.
    pub tools: Vec<String>,
    /// Whether `genaryx_core::command::record` succeeded, i.e. whether a
    /// `console_command` line was appended to the events file.
    pub bus_recorded: bool,
    pub bus_error: Option<String>,
}

// ============================================================================
// error taxonomy
// ============================================================================

/// Every failure mode a [`super::WardryxHandle`] call can surface, fail-closed
/// throughout (06 §0.5: no panics/unwraps cross the FFI boundary). Mirrors
/// [`crate::cloud::CloudError`]'s role and shape, collapsed from
/// `genaryx_connectors::WardryxError`'s variants.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum WardryxError {
    /// [`super::env::discover`] found nothing usable: no `taipan up`
    /// descriptor with a `wardryx` service, and no `WARDRYX_ADMIN_KEY`.
    #[error("no Wardryx policy plane found (no taipan up descriptor, no WARDRYX_ADMIN_KEY)")]
    NoEnvironment,
    /// An environment resolved (or was given explicitly via `connect`), but
    /// building the client or this handle's local journal world failed.
    /// Unlike `CloudError::PairingFailed`, this is never a network failure:
    /// Wardryx has no pairing handshake (bearer-only auth), so this variant
    /// is reachable only via a local runtime/filesystem problem.
    #[error("could not set up the Wardryx connection: {reason}")]
    ConnectFailed { reason: String },
    /// `404 {"error":"approval not found"}`.
    #[error("approval not found")]
    ApprovalNotFound,
    /// `409` - an approval may be decided exactly once.
    #[error("approval was already decided")]
    ApprovalAlreadyDecided,
    /// `403 {"error":"admin role required"}`.
    #[error("admin role required")]
    Forbidden,
    /// `500` - the server has no `WARDRYX_APPROVAL_SECRET` configured, so it
    /// refuses to mint a token rather than granting unsigned.
    #[error("WARDRYX_APPROVAL_SECRET is not configured on the server; grant refused")]
    NoApprovalSecret,
    /// A returned `approval_token` could not be decoded for display.
    #[error("could not decode approval_token: {reason}")]
    BadToken { reason: String },
    /// Any other Wardryx-side failure: transport, a plain non-2xx, or a
    /// response that failed to parse. `status` is `None` when the request
    /// never got far enough to have one.
    #[error("wardryx error (status {status:?}): {message}")]
    Api {
        status: Option<u16>,
        message: String,
    },
}

impl From<ConnWardryxError> for WardryxError {
    fn from(e: ConnWardryxError) -> Self {
        match e {
            ConnWardryxError::ApprovalNotFound => WardryxError::ApprovalNotFound,
            ConnWardryxError::ApprovalAlreadyDecided => WardryxError::ApprovalAlreadyDecided,
            ConnWardryxError::Forbidden => WardryxError::Forbidden,
            ConnWardryxError::NoApprovalSecret => WardryxError::NoApprovalSecret,
            ConnWardryxError::BadToken(reason) => WardryxError::BadToken { reason },
            ConnWardryxError::Api { status, body } => WardryxError::Api {
                status: Some(status),
                message: body,
            },
            ConnWardryxError::Transport(err) => WardryxError::Api {
                status: None,
                message: format!("could not reach Wardryx: {err}"),
            },
            ConnWardryxError::Json(err) => WardryxError::Api {
                status: None,
                message: format!("unexpected response shape from Wardryx: {err}"),
            },
            // Refused client-side, before the request was built: an id that
            // cannot be one URL path segment would otherwise address a
            // different route and read as a plain "not found".
            ConnWardryxError::InvalidPathSegment(err) => WardryxError::Api {
                status: None,
                message: format!("this id cannot be used in a request path: {err}"),
            },
        }
    }
}

// ============================================================================
// helpers
// ============================================================================

/// The HTTP status implied by a failed connector call, `0` when the request
/// never reached a point where one exists (never fabricated). Mirrors
/// `cloud::dto::status_of` exactly, adapted to `ConnWardryxError`'s variants.
pub(super) fn status_of(e: &ConnWardryxError) -> u16 {
    match e {
        ConnWardryxError::Api { status, .. } => *status,
        ConnWardryxError::ApprovalNotFound => 404,
        ConnWardryxError::ApprovalAlreadyDecided => 409,
        ConnWardryxError::Forbidden => 403,
        ConnWardryxError::NoApprovalSecret => 500,
        ConnWardryxError::Transport(_)
        | ConnWardryxError::Json(_)
        | ConnWardryxError::InvalidPathSegment(_)
        | ConnWardryxError::BadToken(_) => 0,
    }
}

/// Build the `verify_result` text plus the decoded claims (when a grant's
/// token decodes cleanly) for one `decide_approval` outcome - shared by
/// [`super::WardryxHandle::finish_decision`] between the journal line and the
/// returned [`ApprovalDecideOutcome`], both must agree on the same wording.
pub(super) fn describe_decision(
    resp: &ConnApprovalDecideResponse,
    now: SystemTime,
) -> (String, Option<ApprovalTokenClaims>) {
    if resp.decision != "grant" {
        return ("denied".to_string(), None);
    }
    match resp.approval_token.as_deref() {
        None => ("granted (no token returned)".to_string(), None),
        Some(token) => match ApprovalTokenClaims::decode(token) {
            Ok(claims) => {
                let ttl = claims.ttl_remaining(now).as_secs();
                let text = format!(
                    "granted ceiling_usd:{:.2} ttl_s:{ttl}",
                    claims.cost_ceiling_usd()
                );
                (text, Some(claims))
            }
            Err(e) => (format!("granted (token undecodable: {e})"), None),
        },
    }
}
