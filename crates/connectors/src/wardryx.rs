//! `WardryxClient`: a typed REST client for Wardryx's policy decision API
//! (07 §4.3) - the Policy Decision Point (PDP) the console's policy panel
//! and approvals inbox render from and act through. Covers all three route
//! groups: `POST /v1/decide` (the PDP itself), `GET /v1/approvals` +
//! `POST /v1/approvals/{id}/decide` (the stateless human-in-the-loop
//! hold/grant/deny flow), and the admin `GET/PUT/DELETE /v1/policies[/{id}]`
//! policy-as-code routes.
//!
//! Every wire shape below was read directly from
//! `~/Development/wardryx/{cmd/wardryx/main.go, internal/api/api.go,
//! internal/api/auth.go, internal/approval/approval.go,
//! internal/policy/policy.go, internal/pdp/pdp.go, internal/store/store.go}`
//! (the authority per this task's ground-truth instructions) and then
//! confirmed empirically against a live `wardryx serve` instance (built
//! from that same checkout) end to end - PUT a policy, trigger a hold,
//! grant it, decode the token, replay allow/deny/already-decided - not
//! guessed anywhere; see the per-struct doc comments for the exact source
//! lines. One genuine discrepancy from this task's own spec text surfaced
//! during that verification: `GET/PUT /v1/policies[/{id}]` responses are
//! FLATTENED (`{"id":"demo","target":"...",...,"updated_at":"..."}`), never
//! `{"id":"demo","policy":{...},"updated_at":"..."}` - see
//! [`PolicyRecord`]'s doc comment.
//!
//! ## Shape: plain `async fn`, bearer-only
//!
//! Like [`crate::CloudClient`], every [`WardryxClient`] method is one
//! request/response round trip over `reqwest`, awaited directly - no
//! background thread or channel machinery. Unlike `CloudClient`, Wardryx
//! has no device-pairing/ES256-signing story at all: every route (bar
//! `/healthz`) is gated purely by a static `Authorization: Bearer <token>`
//! header (`auth.go`'s `authenticate`/`requireAuth`/`requireAdmin`), so this
//! client carries no signer and no paired-device state. Response bytes are
//! parsed with `serde_json::from_slice` rather than `reqwest`'s `json`
//! feature, mirroring `cloud_rest.rs`'s own rationale (no new `reqwest`
//! feature flag needed beyond what `CloudSse` already activates).
//!
//! ## Fail-closed error classification (06 §0.5)
//!
//! Every non-2xx response becomes a specific [`WardryxError`] variant via
//! [`classify_error`], never a panic or a silently-swallowed failure. See
//! [`WardryxError`]'s own doc comments for exactly which status/body
//! combinations map to which variant, and why a couple of them (404, 500)
//! need the response body's message text, not just the status, to
//! disambiguate correctly.
//!
//! ## Typical flow
//!
//! ```no_run
//! # async fn go() -> Result<(), genaryx_connectors::WardryxError> {
//! use genaryx_connectors::{ApprovalVerdict, DecideRequest, WardryxClient};
//!
//! // `bearer_token` must be the BARE token from WARDRYX_KEYS's
//! // "token:org[:role]" spec - see WardryxClient's own doc comment.
//! let client = WardryxClient::new("http://127.0.0.1:8090", "tk_ops")?;
//!
//! let resp = client
//!     .decide(&DecideRequest {
//!         agent_id: "agent://acme/payments".to_string(),
//!         run_id: "run-1".to_string(),
//!         tool_names: vec!["charge".to_string()],
//!         est_cost_usd: 50.0,
//!         ..Default::default()
//!     })
//!     .await?;
//!
//! if resp.decision == "hold" {
//!     let approval_id = resp.approval_id;
//!     let decided = client
//!         .decide_approval(&approval_id, ApprovalVerdict::Grant, "user://acme/alice")
//!         .await?;
//!     println!("granted, approval_token = {:?}", decided.approval_token);
//! }
//! # Ok(())
//! # }
//! ```

use crate::urlpath::{PathSegmentError, path_segment};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ---- error -----------------------------------------------------------------

/// Every failure mode a [`WardryxClient`] call can surface. Fail-closed
/// throughout, mirroring [`crate::ConnectorError`]'s own rule: no panics,
/// no `unwrap`/`expect` in this module; a non-2xx response always becomes
/// one of these via [`classify_error`], never a silently-ignored or generic
/// failure.
#[derive(Debug, thiserror::Error)]
pub enum WardryxError {
    /// The request never got a response at all (DNS, connect, TLS, timeout,
    /// or a response body that failed to read).
    #[error("http transport: {0}")]
    Transport(#[from] reqwest::Error),

    /// A 2xx body that failed to deserialize into the expected shape -
    /// either this client's DTOs have drifted from the live server, or the
    /// server sent something unexpected.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// An approval or policy id cannot be one URL path segment (empty, or
    /// `.`/`..` - see [`crate::PathSegmentError`]). Returned before any
    /// request is built or sent. Unlike [`crate::CloudClient`]'s mutations
    /// nothing here is path-signed, so this is not a signature problem: it is
    /// the routing one underneath it. An id containing a `/` addressed a
    /// different route entirely (`/v1/policies/a/b` is not
    /// `/v1/policies/{"a/b"}`), which reads as "no such policy" rather than
    /// as the bug it is.
    #[error("invalid id for a URL path segment: {0}")]
    InvalidPathSegment(#[from] PathSegmentError),

    /// Any non-2xx response not classified as one of the more specific
    /// variants below: the status and raw body text (UTF-8 lossy). This is
    /// also where a 404 from `GET/DELETE /v1/policies/{id}` lands (body
    /// `"policy not found"`) - see [`WardryxError::ApprovalNotFound`]'s doc
    /// comment for why that is deliberately NOT the same variant as a 404
    /// from the approval-decide route.
    #[error("wardryx returned HTTP {status}: {body}")]
    Api { status: u16, body: String },

    /// `404 {"error":"approval not found"}` from
    /// `POST /v1/approvals/{id}/decide` (`api.go:319-343`,
    /// `handleApprovalDecide`'s `store.ErrNotFound` branch). A 404 from the
    /// policy routes carries a different message (`"policy not found"`,
    /// `api.go:481`/`585`) and is NOT this variant - this connector has no
    /// dedicated policy-not-found variant, so that one falls back to
    /// [`WardryxError::Api`]. See [`classify_error`]'s doc comment.
    #[error("approval not found")]
    ApprovalNotFound,

    /// `409 {"error":"approval was already decided"}` - an approval may be
    /// decided exactly once (`store.go:37-39`'s `ErrAlreadyDecided`,
    /// surfaced at `api.go:344-346`). Confirmed empirically to be the only
    /// 409 source anywhere in Wardryx's HTTP API, so [`classify_error`]
    /// maps every 409 here unconditionally.
    #[error("approval was already decided")]
    ApprovalAlreadyDecided,

    /// `500` from a `"grant"` decide whose body mentions
    /// `WARDRYX_APPROVAL_SECRET` (`api.go:347-349`, wrapping
    /// `approval.ErrNoSecret`): the server has no approval secret
    /// configured, so it refuses to mint a token rather than granting
    /// unsigned (`approval.go:314-316`). Distinguished from any other 500
    /// (every handler also does a generic
    /// `writeError(w, http.StatusInternalServerError, err.Error())` for
    /// unrelated store failures) by matching the error message text, since
    /// both share the same HTTP status - see [`classify_error`].
    #[error("WARDRYX_APPROVAL_SECRET is not configured on the server; grant refused")]
    NoApprovalSecret,

    /// `403 {"error":"admin role required"}` (`auth.go:162-170`'s
    /// `requireAdmin`). Confirmed by reading every `writeError(..., 403,
    /// ...)` call site in `internal/api`: this is the only 403 shape
    /// Wardryx's API ever sends (unlike TokenFuse Cloud, which
    /// distinguishes `signature_invalid` from a plain forbidden - see
    /// [`crate::ConnectorError::SignatureRejected`]), so [`classify_error`]
    /// maps every 403 here unconditionally.
    #[error("admin role required")]
    Forbidden,

    /// [`ApprovalTokenClaims::decode`] could not decode `token`: no `'.'`
    /// separator, invalid base64url, or invalid claims JSON. Never
    /// returned by any HTTP call - decoding is local, offline parsing with
    /// no network round trip.
    #[error("could not decode approval_token: {0}")]
    BadToken(String),
}

/// The `{"error": "..."}` envelope every non-2xx Wardryx response uses
/// (`errorDTO` / `writeError`, `api.go:608-614`). Wardryx has exactly one
/// error shape (unlike TokenFuse Cloud's `ErrorResponse` vs
/// `PlanRequiredResponse` split in `cloud_rest.rs`), so [`classify_error`]
/// only ever needs this one field to disambiguate by status *and* message
/// together.
#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: String,
}

/// Turn a non-2xx status + raw body into the most specific [`WardryxError`]
/// that applies. Status alone does not always disambiguate: 404 is shared
/// between approval-decide (message `"approval not found"` ->
/// [`WardryxError::ApprovalNotFound`]) and the policy routes (message
/// `"policy not found"` -> falls back to [`WardryxError::Api`]), and 500 is
/// shared between the specific `NoApprovalSecret` case and every other
/// internal error - so both are matched on message text, not status alone.
/// 409 and 403 each have exactly one source in the whole API (confirmed by
/// reading every call site; see the corresponding variants' doc comments),
/// so those map on status alone. A body that doesn't parse as
/// [`ErrorEnvelope`] at all (`message` stays `""`) always falls through to
/// [`WardryxError::Api`] with the raw status/body, never a panic on an
/// unexpected shape.
fn classify_error(status: reqwest::StatusCode, bytes: &[u8]) -> WardryxError {
    let message = serde_json::from_slice::<ErrorEnvelope>(bytes)
        .map(|e| e.error)
        .unwrap_or_default();
    match status.as_u16() {
        404 if message == "approval not found" => WardryxError::ApprovalNotFound,
        409 => WardryxError::ApprovalAlreadyDecided,
        403 => WardryxError::Forbidden,
        500 if message.contains("WARDRYX_APPROVAL_SECRET") => WardryxError::NoApprovalSecret,
        _ => WardryxError::Api {
            status: status.as_u16(),
            body: String::from_utf8_lossy(bytes).into_owned(),
        },
    }
}

/// Parse one HTTP response: a 2xx body deserializes as `T`; anything else
/// becomes a classified [`WardryxError`] via [`classify_error`] (never a
/// panic on an unexpected status or an unparseable error body).
async fn parse_response<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T, WardryxError> {
    let status = resp.status();
    let bytes = resp.bytes().await?;
    if status.is_success() {
        Ok(serde_json::from_slice(&bytes)?)
    } else {
        Err(classify_error(status, &bytes))
    }
}

// ---- client ------------------------------------------------------------------

/// A typed client for Wardryx's HTTP policy API (`internal/api/api.go`).
/// Bearer-only auth throughout (no device/signing story at all - unlike
/// [`crate::CloudClient`], every Wardryx route bar `/healthz` is gated
/// purely by a static `Authorization: Bearer <token>` header,
/// `auth.go:138-149`).
///
/// `bearer_token` MUST be the BARE token from `WARDRYX_KEYS`'s
/// `token:org[:role],...` spec, never the full `token:org:role` string:
/// `authenticate` strips the `Bearer ` prefix and looks the remainder up
/// directly in `s.keys` (`auth.go:138-149`), and `ParseKeys` keys that map
/// by `parts[0]` alone - the segment before the first colon
/// (`auth.go:30-58`). This is the exact bug this repo's own
/// `killer_demo_test.rs` module doc describes `taipan` having once shipped
/// against both TokenFuse and Wardryx (issue #20): minting and sending the
/// FULL `token:org:role` spec as the bearer 401s against both, because both
/// servers index their key map by the bare token before the first colon.
pub struct WardryxClient {
    base_url: String,
    bearer_token: String,
    http: reqwest::Client,
}

// Manual Debug: never print `bearer_token` verbatim (same rule
// `CloudClient`'s manual Debug impl follows in cloud_rest.rs) - a stray
// `eprintln!("{client:?}")` must not leak the credential.
impl std::fmt::Debug for WardryxClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WardryxClient")
            .field("base_url", &self.base_url)
            .field("bearer_token", &"<redacted>")
            .finish()
    }
}

impl WardryxClient {
    /// Construct a client for `base_url` (e.g. `http://127.0.0.1:8090` - a
    /// trailing slash is trimmed if present) authenticating every call with
    /// the BARE `bearer_token` (see the struct doc comment).
    ///
    /// Returns `Result` (rather than panicking, as `reqwest::Client::new()`
    /// effectively does internally) because building the underlying HTTP
    /// client can fail - mirrors [`crate::CloudClient::new`]'s identical
    /// rationale and shape.
    pub fn new(
        base_url: impl Into<String>,
        bearer_token: impl Into<String>,
    ) -> Result<Self, WardryxError> {
        let http = reqwest::Client::builder().build()?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            bearer_token: bearer_token.into(),
            http,
        })
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, WardryxError> {
        let resp = self
            .http
            .get(format!("{}{path}", self.base_url))
            .bearer_auth(&self.bearer_token)
            .send()
            .await?;
        parse_response(resp).await
    }

    /// Send `body` as JSON via `method` and parse the response as `T`. Used
    /// by every mutating call (`decide`, `decide_approval`, `put_policy`) -
    /// each just differs in HTTP method, path, and body/response shape.
    async fn send_json<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T, WardryxError> {
        let bytes = serde_json::to_vec(body)?;
        let resp = self
            .http
            .request(method, format!("{}{path}", self.base_url))
            .bearer_auth(&self.bearer_token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(bytes)
            .send()
            .await?;
        parse_response(resp).await
    }

    /// `GET /healthz` - unauthenticated liveness check (`api.go:127-130`,
    /// registered with no auth wrapper at `api.go:116`). No bearer is sent.
    /// The 200 body is the literal text `ok`, not JSON, so this only checks
    /// the status code.
    pub async fn healthz(&self) -> Result<(), WardryxError> {
        let resp = self
            .http
            .get(format!("{}/healthz", self.base_url))
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let bytes = resp.bytes().await?;
            Err(classify_error(status, &bytes))
        }
    }

    /// `POST /v1/decide` - the PDP itself (`handleDecide`, `api.go:202-304`).
    /// Evaluated server-side against the PDP rule order documented on
    /// [`DecideRequest`]; never reimplemented client-side.
    pub async fn decide(&self, req: &DecideRequest) -> Result<DecideResponse, WardryxError> {
        self.send_json(reqwest::Method::POST, "/v1/decide", req)
            .await
    }

    /// `GET /v1/approvals` - every approval belonging to the caller's org
    /// (a bare JSON array; `handleListApprovals`, `api.go:389-417`),
    /// sorted by `requested_at` ascending.
    pub async fn list_approvals(&self) -> Result<Vec<Approval>, WardryxError> {
        self.get_json("/v1/approvals").await
    }

    /// `POST /v1/approvals/{id}/decide` (admin) - grant or deny a pending
    /// approval (`handleApprovalDecide`, `api.go:319-368`). `decided_by` is
    /// a required, caller-supplied identifier (e.g. `user://org/alice`;
    /// `api.go:334-337`). On [`ApprovalVerdict::Grant`] the response
    /// carries a fresh `approval_token`; on [`ApprovalVerdict::Deny`] it
    /// does not ([`ApprovalDecideResponse::approval_token`] is `None`).
    ///
    /// Fails closed with [`WardryxError::ApprovalNotFound`] (404),
    /// [`WardryxError::ApprovalAlreadyDecided`] (409 - an approval may be
    /// decided exactly once), or [`WardryxError::NoApprovalSecret`] (500,
    /// only reachable via `Grant`: a `Deny` never touches the approval
    /// secret at all, `approval.go:314-316`).
    pub async fn decide_approval(
        &self,
        id: &str,
        verdict: ApprovalVerdict,
        decided_by: &str,
    ) -> Result<ApprovalDecideResponse, WardryxError> {
        let body = ApprovalDecideRequestBody {
            decision: verdict.as_wire(),
            decided_by: decided_by.to_string(),
        };
        let approval = path_segment(id)?;
        self.send_json(
            reqwest::Method::POST,
            &format!("/v1/approvals/{approval}/decide"),
            &body,
        )
        .await
    }

    /// `GET /v1/policies` (admin) - every stored policy, a bare JSON array
    /// ordered by id (`handleListPolicies`, `api.go:464-475`). Only
    /// store-persisted policies are listed here; a file-loaded base policy
    /// (`-policy`/`WARDRYX_POLICY`) that was never also written through
    /// this API is invisible to it, even though it still governs
    /// `/v1/decide` (`api.go`'s package doc comment, "Policy-as-code").
    pub async fn list_policies(&self) -> Result<Vec<PolicyRecord>, WardryxError> {
        self.get_json("/v1/policies").await
    }

    /// `GET /v1/policies/{id}` (admin). A 404 here
    /// ([`WardryxError::Api`] with `status: 404`, body `"policy not
    /// found"` - see [`WardryxError::ApprovalNotFound`]'s doc comment for
    /// why this is deliberately NOT that variant) means no policy is
    /// stored under `id`.
    pub async fn get_policy(&self, id: &str) -> Result<PolicyRecord, WardryxError> {
        let policy = path_segment(id)?;
        self.get_json(&format!("/v1/policies/{policy}")).await
    }

    /// `PUT /v1/policies/{id}` (admin) - create or replace the policy
    /// stored under `id` (`handlePutPolicy`, `api.go:502-556`). The
    /// request body is `policy` itself, exactly as sent - no id, no
    /// wrapper; `id` is the URL path segment, never a body field. A
    /// malformed or invalid resulting policy set 400s
    /// ([`WardryxError::Api`]) before anything is persisted or the live
    /// engine is touched (`policy.Compile` runs before the store write,
    /// `api.go:535-545`).
    ///
    /// Confirmed by reading `handlePutPolicy` end to end, and empirically
    /// against a live server: a successful write ALSO swaps the live PDP
    /// engine's policy set in the same request
    /// (`s.engine.SetPolicies(newSet)`, `api.go:546`, executed before the
    /// handler returns), so a subsequent [`WardryxClient::decide`]
    /// immediately observes the new policy - no separate reload, restart,
    /// or propagation delay.
    pub async fn put_policy(
        &self,
        id: &str,
        policy: &Policy,
    ) -> Result<PolicyRecord, WardryxError> {
        let id = path_segment(id)?;
        self.send_json(reqwest::Method::PUT, &format!("/v1/policies/{id}"), policy)
            .await
    }

    /// `DELETE /v1/policies/{id}` (admin) - `204 No Content`, no response
    /// body, on success (`handleDeletePolicy`, `api.go:565-604`). Handled
    /// separately from [`WardryxClient::get_json`]/`send_json` because
    /// those parse every response body as JSON, and a 204 body is empty (not
    /// even `null`). A 404 ([`WardryxError::Api`], body `"policy not
    /// found"`) means no policy was stored under `id`.
    pub async fn delete_policy(&self, id: &str) -> Result<(), WardryxError> {
        let id = path_segment(id)?;
        let resp = self
            .http
            .delete(format!("{}/v1/policies/{id}", self.base_url))
            .bearer_auth(&self.bearer_token)
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let bytes = resp.bytes().await?;
            Err(classify_error(status, &bytes))
        }
    }
}

// ---- DTOs: /v1/decide (api.go:174-200) --------------------------------------

/// `skip_serializing_if` predicate shared by every optional field below:
/// Go's `omitempty` tag drops a field from the wire the moment it holds its
/// zero value, so a client that wants byte-identical request bodies (and,
/// for [`Policy`], round-trips through `PUT`/`GET` without spurious diffs)
/// mirrors that with the same rule here.
fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// `POST /v1/decide` request body. Exact shape (field set and `omitempty`s)
/// of `decideRequestDTO` (`api.go:174-185`). `agent_id`/`run_id` are the
/// only required fields - `handleDecide` 400s if either is empty
/// (`api.go:208-211`).
///
/// The PDP evaluates a request in this order (`pdp.go:209-242`'s `Decide`
/// doc comment, not reimplemented here - this client always asks the live
/// server): (1) an invalid `on_behalf_of` delegation chain denies,
/// independent of any policy; (2) a requested tool in a matched policy's
/// `deny_tool` denies; (3) a matched policy's `deny_if_unattested` with no
/// live attestation denies; (4) a matched policy's `max_steps`, reached or
/// exceeded by `steps`, denies; (5) a matched policy's `allow_domains`,
/// missing an entry from `domains`, denies; (6) a matched policy's
/// `deny_above_usd`, exceeded by `est_cost_usd`, denies outright - a hard
/// ceiling no `approval_token`, however validly minted, can override; (7) a
/// matched policy's `require_human_above_usd`, exceeded by `est_cost_usd`,
/// resolves to `hold` unless a valid `approval_token` was presented (then
/// `allow`) or an *invalid* one was presented (then `deny`, not a
/// downgrade to `hold`); (8) otherwise, `allow`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DecideRequest {
    pub agent_id: String,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub on_behalf_of: Vec<String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub tool_names: Vec<String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub domains: Vec<String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub steps: i64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub model: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub est_cost_usd: f64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub attestation_method: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub approval_token: String,
}

/// `POST /v1/decide` response body. Exact shape of `decideResponseDTO`
/// (`api.go:187-200`). `decision` is one of the literal strings
/// `"allow"`/`"deny"`/`"hold"` (`pdp.go:41-45`'s `Allow`/`Deny`/`Hold`
/// consts) - kept as a plain `String` rather than a closed enum, since this
/// crate's public surface (per this task's spec) exports no `Decision`
/// type; compare against the literals directly.
#[derive(Debug, Clone, Deserialize)]
pub struct DecideResponse {
    pub decision: String,
    pub policy_version: String,
    pub reason: String,
    /// Only set when `decision == "hold"` (`omitempty` on the wire,
    /// `api.go:191`); `#[serde(default)]` so an omitted key deserializes
    /// as `""`, same as an `allow`/`deny` response.
    #[serde(default)]
    pub approval_id: String,
    pub approval_token_required: bool,
    pub cacheable: bool,
}

// ---- DTOs: /v1/approvals (api.go:308-417) -----------------------------------

/// `POST /v1/approvals/{id}/decide`'s `decision` field
/// (`approvalDecideRequestDTO`, `api.go:308-311`): exactly `"grant"` or
/// `"deny"`, else the server 400s (`api.go:330-333`). Modeled as a closed
/// enum here (unlike [`DecideResponse::decision`], which mirrors an
/// open-ended PDP verdict this crate never needs to branch on internally)
/// because this one is a caller-supplied choice with exactly two valid
/// values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalVerdict {
    Grant,
    Deny,
}

impl ApprovalVerdict {
    fn as_wire(self) -> &'static str {
        match self {
            ApprovalVerdict::Grant => "grant",
            ApprovalVerdict::Deny => "deny",
        }
    }
}

/// `POST /v1/approvals/{id}/decide` request body. Exact shape of
/// `approvalDecideRequestDTO` (`api.go:308-311`).
#[derive(Debug, Serialize)]
struct ApprovalDecideRequestBody {
    decision: &'static str,
    decided_by: String,
}

/// `POST /v1/approvals/{id}/decide` response body. Exact shape of
/// `approvalDecideResponseDTO` (`api.go:313-317`). `approval_token` is
/// present only on a `"grant"` decision: confirmed empirically (a `"deny"`
/// response omits the key entirely) and by reading `approval.Decide`
/// (`approval.go:310-331`), which only ever calls `MintApprovalToken` from
/// the grant branch.
#[derive(Debug, Clone, Deserialize)]
pub struct ApprovalDecideResponse {
    pub approval_id: String,
    pub decision: String,
    #[serde(default)]
    pub approval_token: Option<String>,
}

/// One element of `GET /v1/approvals`'s bare JSON array response
/// (`approvalDTO`, `api.go:372-382`; `handleListApprovals`,
/// `api.go:389-417`). Org-scoped server-side (only the caller's own org's
/// approvals are ever returned, matched against `context["org"]`) and
/// sorted by `requested_at` ascending.
///
/// `context` carries everything about the held decision that isn't one of
/// this struct's own named fields - `org`, `model`, `est_cost_usd`,
/// `attestation_method`, `on_behalf_of`, `reason`, `policy_version`, and
/// `tool_names` (`internal/approval.Request` stamps all but `org`,
/// `approval.go:277-299`; `handleDecide` stamps `org` from the
/// authenticated principal on top, `api.go:277-285`) - as an untyped JSON
/// object, per this task's spec: prefer the typed accessor methods below
/// over reaching into `context` directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub approval_id: String,
    pub agent_id: String,
    pub run_id: String,
    /// RFC3339, UTC.
    pub requested_at: String,
    /// `omitempty` on the wire (`api.go:377`); absent while pending.
    #[serde(default)]
    pub decided_at: Option<String>,
    /// `omitempty` on the wire (`api.go:378`); absent while pending.
    #[serde(default)]
    pub decided_by: Option<String>,
    /// `omitempty` on the wire (`api.go:379`): a pending approval OMITS
    /// this key entirely rather than sending `""`, so it deserializes as
    /// `None` here, same as `decided_at`/`decided_by`.
    #[serde(default)]
    pub decision: Option<String>,
    pub pending: bool,
    #[serde(default)]
    pub context: Option<serde_json::Map<String, serde_json::Value>>,
}

impl Approval {
    fn context_get(&self, key: &str) -> Option<&serde_json::Value> {
        self.context.as_ref()?.get(key)
    }

    /// `context["tool_names"]`: the tool set the held action declared,
    /// pre-sorted. Always present on a real hold - `approval.Request`
    /// stamps it unconditionally (`approval.go:286`).
    pub fn tool_names(&self) -> Option<Vec<String>> {
        self.context_get("tool_names")?
            .as_array()?
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect()
    }

    /// `context["est_cost_usd"]`: the estimated cost that triggered the
    /// hold (`handleDecide`, `api.go:280`) - also the ceiling a grant's
    /// freshly minted `approval_token` embeds
    /// (`costFromContext`/`approval.Decide`, `approval.go:310-331`,
    /// `369-382`).
    pub fn est_cost_usd(&self) -> Option<f64> {
        self.context_get("est_cost_usd")?.as_f64()
    }

    /// `context["reason"]`: the PDP's one-sentence explanation of why this
    /// action held (`api.go:284`).
    pub fn reason(&self) -> Option<&str> {
        self.context_get("reason")?.as_str()
    }

    /// `context["on_behalf_of"]`: the delegation chain, root-first, or
    /// `None` if the request declared none (stored as JSON `null` in that
    /// case, confirmed empirically - not an omitted key; `Value::as_array`
    /// already returns `None` for `null`, so no separate null check is
    /// needed here).
    pub fn on_behalf_of(&self) -> Option<Vec<String>> {
        self.context_get("on_behalf_of")?
            .as_array()?
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect()
    }

    /// `context["policy_version"]`: the policy set generation that decided
    /// this hold.
    pub fn policy_version(&self) -> Option<&str> {
        self.context_get("policy_version")?.as_str()
    }

    /// `context["org"]`: stamped from the authenticated principal that
    /// triggered the hold (`api.go:278`), not from the request body.
    pub fn org(&self) -> Option<&str> {
        self.context_get("org")?.as_str()
    }

    /// `context["model"]`: carried through for display only - the PDP
    /// never branches on it (`pdp.go:79-81`).
    pub fn model(&self) -> Option<&str> {
        self.context_get("model")?.as_str()
    }
}

// ---- DTOs: /v1/policies (api.go:420-604, policy.go:32-76) ------------------

/// One declarative Wardryx policy document; `POST /v1/decide`'s PDP rules
/// are compiled from these. Exact field set and JSON tags mirror
/// `policy.Policy` (`internal/policy/policy.go:32-76`); every field but
/// `target` carries Go's `omitempty` tag there, mirrored here with
/// `skip_serializing_if = "is_default"` on each. This is exactly the body
/// `PUT /v1/policies/{id}` accepts (`handlePutPolicy`, `api.go:502-556`,
/// decodes the request body directly into a `policy.Policy` - no id, no
/// wrapper: `id` is the URL path segment).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    /// Defaults to `target` when left empty, but only inside the PDP's
    /// compiled engine (`policy.go`'s `normalize`) - a `PUT` with no
    /// `name` still round-trips as `""`/absent through `GET`, never
    /// silently rewritten to the target glob.
    #[serde(default, skip_serializing_if = "is_default")]
    pub name: String,
    /// The `agent://` glob this policy targets. The one required field
    /// (`policy.go:296-298`'s `validate`: an empty target is a hard
    /// error).
    pub target: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub deny_tool: Vec<String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub allow_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub require_human_above_usd: f64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub deny_above_usd: f64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub max_steps: i64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub deny_if_unattested: bool,
}

/// One stored policy, as returned by `GET /v1/policies` (a bare array of
/// these), `GET /v1/policies/{id}`, and the response body of
/// `PUT /v1/policies/{id}` (all three share `policyDTO`, `api.go:426-438`
/// via `policyRecordToDTO`).
///
/// `policyDTO` EMBEDS `policy.Policy` as a Go anonymous struct field
/// (`api.go:428`), so on the wire `id` and `updated_at` sit FLATTENED
/// alongside the policy's own fields (`target`, `deny_tool`, ...) - NEVER
/// nested under a `"policy"` key. This is a real discrepancy from this
/// task's own spec text, which left the shape as an open question;
/// confirmed empirically against a live server:
/// `PUT /v1/policies/demo` returns
/// `{"id":"demo","target":"...","deny_tool":[...],"updated_at":"..."}`,
/// not `{"id":"demo","policy":{...},"updated_at":"..."}`.
/// `#[serde(flatten)]` on the embedded `policy` field reproduces the same
/// shape here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRecord {
    /// The URL path segment this policy is addressed by
    /// (`PUT /v1/policies/{id}`); distinct from [`Policy::name`], which
    /// need not be unique or URL-safe (`store.go:80-90`'s doc comment on
    /// `PolicyRecord.ID`).
    pub id: String,
    #[serde(flatten)]
    pub policy: Policy,
    /// RFC3339 UTC timestamp of the last `PutPolicy` write
    /// (`policyRecordToDTO`, `api.go:432-437`). Every live server response
    /// populates this (`handlePutPolicy` always passes
    /// `time.Now().UTC()`, `api.go:541-542`); `None` is only reachable in
    /// principle, for a `PolicyRecord` whose `UpdatedAt` was never set.
    #[serde(default)]
    pub updated_at: Option<String>,
}

// ---- approval_token claims (approval.go:107-114, 140-213) ------------------

/// The claims embedded in a minted `approval_token`'s first (base64url,
/// no-pad) segment. Exact JSON shape of `claims`
/// (`internal/approval/approval.go:107-114`). `tools` is always pre-sorted
/// by the server before signing (`sortedCopy`, `approval.go:116-120,149`),
/// so no re-sorting is applied on decode.
///
/// This is a DISPLAY-ONLY decoder. The console never holds a copy of
/// `WARDRYX_APPROVAL_SECRET` (it lives only on the wardryx server), so
/// [`ApprovalTokenClaims::decode`] can read what a token CLAIMS but can
/// never verify its HMAC-SHA256 signature - only
/// `VerifyApprovalToken` (`approval.go:176-213`) does that, and it runs
/// exclusively inside the wardryx server's own `/v1/decide` handler. Never
/// treat a decoded [`ApprovalTokenClaims`] as proof of anything; it exists
/// to show a human what a token says it authorizes (agent/run/tools/cost
/// ceiling/expiry), nothing more. This client always forwards
/// `approval_token` to `/v1/decide` as the exact raw string the server
/// handed back, never reconstructed from decoded claims.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ApprovalTokenClaims {
    pub agent_id: String,
    pub run_id: String,
    pub tools: Vec<String>,
    pub max_cost_usd: f64,
    /// Unix seconds (`claims.Exp`, `approval.go:112`).
    pub exp: i64,
    pub nonce: String,
}

impl ApprovalTokenClaims {
    /// Decode (never verify - see the struct doc comment) an
    /// `approval_token`'s claims: split on the first `'.'`
    /// (`strings.Cut`, `approval.go:180`, mirrored here with
    /// `str::split_once`), base64url-no-pad-decode the payload segment
    /// (`base64.RawURLEncoding`, `approval.go:154,191`), and parse the
    /// result as JSON.
    pub fn decode(token: &str) -> Result<Self, WardryxError> {
        let (payload, _sig) = token
            .split_once('.')
            .ok_or_else(|| WardryxError::BadToken("missing '.' separator".to_string()))?;
        if payload.is_empty() {
            return Err(WardryxError::BadToken("empty claims payload".to_string()));
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|e| WardryxError::BadToken(format!("invalid base64url payload: {e}")))?;
        serde_json::from_slice(&decoded)
            .map_err(|e| WardryxError::BadToken(format!("invalid claims JSON: {e}")))
    }

    /// The claims' embedded expiry as a [`SystemTime`]. A negative `exp`
    /// (never produced by a real server, but not a panic risk here either)
    /// clamps to [`UNIX_EPOCH`].
    pub fn expires_at(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(self.exp.max(0) as u64)
    }

    /// Whether `now` is past the claims' expiry. Mirrors the server's own
    /// boundary exactly (`time.Now().Unix() > c.Exp`, `approval.go:200`):
    /// `now == expires_at()` is NOT expired, only strictly after is.
    pub fn is_expired(&self, now: SystemTime) -> bool {
        now > self.expires_at()
    }

    /// How much of the token's TTL remains at `now`, saturating to
    /// [`Duration::ZERO`] once expired rather than underflowing/panicking.
    pub fn ttl_remaining(&self, now: SystemTime) -> Duration {
        self.expires_at()
            .duration_since(now)
            .unwrap_or(Duration::ZERO)
    }

    /// The cost ceiling this token authorizes (`claims.MaxCostUSD`,
    /// `approval.go:111`) - `VerifyApprovalToken` denies any presented
    /// `est_cost_usd` greater than this (`approval.go:209-211`).
    pub fn cost_ceiling_usd(&self) -> f64 {
        self.max_cost_usd
    }
}

// ---- unit tests (no network; see tests/wardryx_test.rs for the live cycle) -

#[cfg(test)]
mod tests {
    use super::*;

    // ---- DTO shape / round-trip -------------------------------------------

    #[test]
    fn decide_request_omits_zero_value_optional_fields() {
        let req = DecideRequest {
            agent_id: "agent://acme/payments".to_string(),
            run_id: "run-1".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_string(&req).expect("serialize");
        assert_eq!(
            json,
            r#"{"agent_id":"agent://acme/payments","run_id":"run-1"}"#
        );
    }

    #[test]
    fn decide_request_includes_set_optional_fields() {
        let req = DecideRequest {
            agent_id: "a".to_string(),
            run_id: "r".to_string(),
            tool_names: vec!["charge".to_string()],
            est_cost_usd: 50.0,
            ..Default::default()
        };
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&req).unwrap()).unwrap();
        assert_eq!(v["tool_names"], serde_json::json!(["charge"]));
        assert_eq!(v["est_cost_usd"], serde_json::json!(50.0));
        assert!(v.get("domains").is_none(), "unset fields stay omitted");
    }

    #[test]
    fn decide_response_deserializes_hold_and_allow_shapes() {
        let hold: DecideResponse = serde_json::from_str(
            r#"{"decision":"hold","policy_version":"abc123","reason":"needs a human",
                "approval_id":"ap_1","approval_token_required":true,"cacheable":false}"#,
        )
        .expect("valid hold DecideResponse");
        assert_eq!(hold.decision, "hold");
        assert_eq!(hold.approval_id, "ap_1");

        // allow/deny omit approval_id entirely (omitempty) - must still parse.
        let allow: DecideResponse = serde_json::from_str(
            r#"{"decision":"allow","policy_version":"abc123","reason":"ok",
                "approval_token_required":false,"cacheable":true}"#,
        )
        .expect("valid allow DecideResponse with approval_id omitted");
        assert_eq!(allow.decision, "allow");
        assert_eq!(allow.approval_id, "");
    }

    #[test]
    fn policy_record_is_flattened_not_nested() {
        // The exact shape observed from a live `PUT /v1/policies/demo`.
        let rec: PolicyRecord = serde_json::from_str(
            r#"{"id":"demo","target":"agent://test-org/*","deny_tool":["shell_exec"],
                "require_human_above_usd":1,"deny_above_usd":1000,
                "updated_at":"2026-07-17T05:33:20Z"}"#,
        )
        .expect("flattened PolicyRecord");
        assert_eq!(rec.id, "demo");
        assert_eq!(rec.policy.target, "agent://test-org/*");
        assert_eq!(rec.policy.deny_tool, vec!["shell_exec".to_string()]);
        assert!((rec.policy.require_human_above_usd - 1.0).abs() < f64::EPSILON);
        assert_eq!(rec.updated_at.as_deref(), Some("2026-07-17T05:33:20Z"));
        // A policy that never set optional fields (e.g. `name`) round-trips
        // them as defaults, not an error.
        assert_eq!(rec.policy.name, "");
        assert!(rec.policy.max_steps == 0 && !rec.policy.deny_if_unattested);
    }

    #[test]
    fn policy_serializes_with_omitempty_semantics() {
        let p = Policy {
            target: "agent://acme/*".to_string(),
            deny_above_usd: 1000.0,
            ..Default::default()
        };
        let json = serde_json::to_string(&p).expect("serialize");
        assert_eq!(
            json,
            r#"{"target":"agent://acme/*","deny_above_usd":1000.0}"#
        );
    }

    #[test]
    fn approval_deserializes_pending_with_null_on_behalf_of() {
        let a: Approval = serde_json::from_str(
            r#"{"approval_id":"ap_1","agent_id":"agent://test-org/payments","run_id":"run-1",
                "requested_at":"2026-07-17T05:33:25Z","pending":true,
                "context":{"attestation_method":"","est_cost_usd":50,"model":"",
                    "on_behalf_of":null,"org":"test-org","policy_version":"d67348cb5c14",
                    "reason":"estimated cost $50.00 exceeds policy threshold $1.00",
                    "tool_names":["charge"]}}"#,
        )
        .expect("valid pending Approval json");
        assert!(a.pending);
        assert_eq!(a.decision, None, "omitempty key must be absent, not \"\"");
        assert_eq!(a.decided_at, None);
        assert_eq!(a.tool_names(), Some(vec!["charge".to_string()]));
        assert!((a.est_cost_usd().unwrap() - 50.0).abs() < f64::EPSILON);
        assert_eq!(a.org(), Some("test-org"));
        assert_eq!(a.policy_version(), Some("d67348cb5c14"));
        assert!(!a.reason().unwrap().is_empty());
        assert_eq!(
            a.on_behalf_of(),
            None,
            "a JSON null on_behalf_of must decode as None, not Some(vec![])"
        );
        assert_eq!(a.model(), Some(""));
    }

    #[test]
    fn approval_decide_response_grant_has_token_deny_does_not() {
        let granted: ApprovalDecideResponse = serde_json::from_str(
            r#"{"approval_id":"ap_1","decision":"grant","approval_token":"abc.def"}"#,
        )
        .expect("valid grant response");
        assert_eq!(granted.approval_token.as_deref(), Some("abc.def"));

        let denied: ApprovalDecideResponse =
            serde_json::from_str(r#"{"approval_id":"ap_1","decision":"deny"}"#)
                .expect("valid deny response, approval_token omitted");
        assert_eq!(denied.approval_token, None);
    }

    #[test]
    fn approval_verdict_serializes_to_grant_or_deny() {
        assert_eq!(ApprovalVerdict::Grant.as_wire(), "grant");
        assert_eq!(ApprovalVerdict::Deny.as_wire(), "deny");
    }

    // ---- error classification ----------------------------------------------

    #[test]
    fn classifies_404_approval_not_found_distinctly_from_404_policy_not_found() {
        let approval_404 = classify_error(
            reqwest::StatusCode::NOT_FOUND,
            br#"{"error":"approval not found"}"#,
        );
        assert!(matches!(approval_404, WardryxError::ApprovalNotFound));

        let policy_404 = classify_error(
            reqwest::StatusCode::NOT_FOUND,
            br#"{"error":"policy not found"}"#,
        );
        match policy_404 {
            WardryxError::Api { status, body } => {
                assert_eq!(status, 404);
                assert!(body.contains("policy not found"));
            }
            other => panic!("expected a generic Api error for policy 404, got {other:?}"),
        }
    }

    #[test]
    fn classifies_409_as_already_decided() {
        let err = classify_error(
            reqwest::StatusCode::CONFLICT,
            br#"{"error":"approval was already decided"}"#,
        );
        assert!(matches!(err, WardryxError::ApprovalAlreadyDecided));
    }

    #[test]
    fn classifies_403_as_forbidden() {
        let err = classify_error(
            reqwest::StatusCode::FORBIDDEN,
            br#"{"error":"admin role required"}"#,
        );
        assert!(matches!(err, WardryxError::Forbidden));
    }

    #[test]
    fn classifies_500_no_approval_secret_distinctly_from_other_500s() {
        let no_secret = classify_error(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            br#"{"error":"WARDRYX_APPROVAL_SECRET is not configured; cannot grant"}"#,
        );
        assert!(matches!(no_secret, WardryxError::NoApprovalSecret));

        let generic = classify_error(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            br#"{"error":"failed to record approval hold: connection reset"}"#,
        );
        match generic {
            WardryxError::Api { status, body } => {
                assert_eq!(status, 500);
                assert!(body.contains("connection reset"));
            }
            other => panic!("expected a generic Api error, got {other:?}"),
        }
    }

    #[test]
    fn classifies_unrecognized_status_and_garbage_body_without_panicking() {
        let err = classify_error(
            reqwest::StatusCode::BAD_REQUEST,
            br#"{"error":"bad input"}"#,
        );
        assert!(matches!(err, WardryxError::Api { status: 400, .. }));

        let err = classify_error(reqwest::StatusCode::BAD_GATEWAY, b"<html>502</html>");
        match err {
            WardryxError::Api { status, body } => {
                assert_eq!(status, 502);
                assert!(body.contains("502"));
            }
            other => panic!("expected a generic Api error, got {other:?}"),
        }
    }

    #[test]
    fn new_trims_a_trailing_slash_from_base_url() {
        let client = WardryxClient::new("http://127.0.0.1:8090/", "tk_test").expect("client");
        assert_eq!(client.base_url, "http://127.0.0.1:8090");
    }

    #[test]
    fn debug_redacts_bearer_token() {
        let client =
            WardryxClient::new("http://127.0.0.1:8090", "super-secret-token").expect("client");
        let printed = format!("{client:?}");
        assert!(!printed.contains("super-secret-token"));
        assert!(printed.contains("<redacted>"));
    }

    // ---- ApprovalTokenClaims: decode/expiry/ttl, no network ----------------

    /// Build a syntactically valid but unverified token string (a real
    /// HMAC signature is not needed for `decode`, which never checks it -
    /// see the struct doc comment): base64url-no-pad the claims JSON, then
    /// append an arbitrary `.`-separated second segment, exactly like a
    /// real `payload.sig` token's shape.
    fn build_token(claims_json: &str, sig: &str) -> String {
        let payload = URL_SAFE_NO_PAD.encode(claims_json.as_bytes());
        format!("{payload}.{sig}")
    }

    #[test]
    fn decode_reads_claims_without_verifying_signature() {
        let token = build_token(
            r#"{"agent_id":"agent://acme/payments","run_id":"run-1","tools":["charge"],
                "max_cost_usd":50.0,"exp":1784267010,"nonce":"c44e0eeb9f769db5"}"#,
            "not-a-real-hmac-signature",
        );
        let claims = ApprovalTokenClaims::decode(&token).expect("decode ignores the signature");
        assert_eq!(claims.agent_id, "agent://acme/payments");
        assert_eq!(claims.run_id, "run-1");
        assert_eq!(claims.tools, vec!["charge".to_string()]);
        assert!((claims.cost_ceiling_usd() - 50.0).abs() < f64::EPSILON);
        assert_eq!(claims.exp, 1_784_267_010);
        assert_eq!(claims.nonce, "c44e0eeb9f769db5");
    }

    #[test]
    fn decode_rejects_malformed_tokens_without_panicking() {
        assert!(matches!(
            ApprovalTokenClaims::decode("no-dot-separator"),
            Err(WardryxError::BadToken(_))
        ));
        assert!(matches!(
            ApprovalTokenClaims::decode(".sig-only"),
            Err(WardryxError::BadToken(_))
        ));
        assert!(matches!(
            ApprovalTokenClaims::decode("not-valid-base64url!!.sig"),
            Err(WardryxError::BadToken(_))
        ));
        let bad_json = format!("{}.sig", URL_SAFE_NO_PAD.encode(b"not json"));
        assert!(matches!(
            ApprovalTokenClaims::decode(&bad_json),
            Err(WardryxError::BadToken(_))
        ));
    }

    #[test]
    fn is_expired_boundary_matches_server_semantics_exactly() {
        let exp: i64 = 1_784_267_010;
        let token = build_token(
            &format!(
                r#"{{"agent_id":"a","run_id":"r","tools":[],"max_cost_usd":0,"exp":{exp},"nonce":"n"}}"#
            ),
            "sig",
        );
        let claims = ApprovalTokenClaims::decode(&token).expect("decode");

        let at_exp = UNIX_EPOCH + Duration::from_secs(exp as u64);
        let one_sec_before = at_exp - Duration::from_secs(1);
        let one_sec_after = at_exp + Duration::from_secs(1);

        assert!(
            !claims.is_expired(one_sec_before),
            "one second before expiry must not be expired"
        );
        assert!(
            !claims.is_expired(at_exp),
            "exactly at expiry must not be expired (mirrors `now.Unix() > c.Exp`, strict)"
        );
        assert!(
            claims.is_expired(one_sec_after),
            "one second after expiry must be expired"
        );
    }

    #[test]
    fn ttl_remaining_counts_down_and_saturates_at_zero() {
        let exp: i64 = 1_784_267_010;
        let token = build_token(
            &format!(
                r#"{{"agent_id":"a","run_id":"r","tools":[],"max_cost_usd":0,"exp":{exp},"nonce":"n"}}"#
            ),
            "sig",
        );
        let claims = ApprovalTokenClaims::decode(&token).expect("decode");
        let at_exp = UNIX_EPOCH + Duration::from_secs(exp as u64);

        assert_eq!(
            claims.ttl_remaining(at_exp - Duration::from_secs(120)),
            Duration::from_secs(120)
        );
        assert_eq!(claims.ttl_remaining(at_exp), Duration::ZERO);
        assert_eq!(
            claims.ttl_remaining(at_exp + Duration::from_secs(60)),
            Duration::ZERO,
            "ttl_remaining must saturate to zero once expired, never underflow/panic"
        );
    }
}
