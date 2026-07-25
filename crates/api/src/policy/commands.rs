//! Console commands for the Policy view: two reads (`policy_list_approvals`/
//! `policy_list_policies`), one privileged mutation
//! (`policy_decide_approval`), plus `policy_status` so the frontend can
//! render a clean "no policy plane" / "unreachable" state up front instead
//! of guessing from a read command's error shape.
//!
//! Every DTO here is a UI-facing serde mirror of a `genaryx_connectors`
//! Wardryx type, same convention `money::commands`'s `RunDto`/`IncidentDto`
//! already use for `genaryx_connectors::CloudClient` DTOs: the connector's
//! own types only derive `Deserialize` (they exist to parse Wardryx's
//! responses), never `Serialize`, so they cannot be handed to the frontend
//! as-is.
//!
//! Fail-closed mutation contract, identical in spirit to
//! `money::commands::finish_mutation`: `policy_decide_approval` calls the
//! connector first, then ALWAYS attempts `genaryx_core::command::record` -
//! even when the Wardryx call itself failed or was rejected, since a
//! rejected privileged attempt is itself part of the audit trail (06 §0.4).
//! The two outcomes are reported back separately and honestly: Wardryx's
//! verdict decides `Ok`/`Err` for the frontend, while
//! `DecideOutcome::bus_recorded`/`bus_error` says whether the local journal
//! write succeeded, never conflating the two.

use super::env::EnvSource;
use super::state::{PolicyClient, PolicyInner, PolicyState};
use genaryx_connectors::{
    Approval, ApprovalDecideResponse, ApprovalTokenClaims, ApprovalVerdict, PolicyRecord,
    WardryxError,
};
use genaryx_core::{CommandRecord, command};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::SystemTime;

// ============================================================================
// DTOs
// ============================================================================

/// One row of the Approvals Inbox. Flattens `Approval`'s untyped `context`
/// map into named fields via `Approval`'s own typed accessors (never
/// exposing the raw `context` object to the frontend) - same "typed DTO
/// over an untyped wire shape" convention `money::commands::IncidentDto`
/// follows.
#[derive(Debug, Clone, Serialize)]
pub struct ApprovalDto {
    pub approval_id: String,
    pub agent_id: String,
    pub run_id: String,
    pub requested_at: String,
    pub decided_at: Option<String>,
    pub decided_by: Option<String>,
    /// `"grant"` / `"deny"`, or `None` while still pending.
    pub decision: Option<String>,
    pub pending: bool,
    pub tool_names: Vec<String>,
    pub est_cost_usd: Option<f64>,
    pub reason: Option<String>,
    /// The delegation chain, root-first; `None` when the triggering request
    /// declared none (see `Approval::on_behalf_of`'s doc comment on the
    /// `null` vs `[]` distinction).
    pub on_behalf_of: Option<Vec<String>>,
    pub policy_version: Option<String>,
    pub org: Option<String>,
    pub model: Option<String>,
}

impl From<&Approval> for ApprovalDto {
    fn from(a: &Approval) -> Self {
        Self {
            approval_id: a.approval_id.clone(),
            agent_id: a.agent_id.clone(),
            run_id: a.run_id.clone(),
            requested_at: a.requested_at.clone(),
            decided_at: a.decided_at.clone(),
            decided_by: a.decided_by.clone(),
            decision: a.decision.clone(),
            pending: a.pending,
            tool_names: a.tool_names().unwrap_or_default(),
            est_cost_usd: a.est_cost_usd(),
            reason: a.reason().map(str::to_string),
            on_behalf_of: a.on_behalf_of(),
            policy_version: a.policy_version().map(str::to_string),
            org: a.org().map(str::to_string),
            model: a.model().map(str::to_string),
        }
    }
}

/// One row of the Policy view. Flattened mirror of `PolicyRecord` (itself
/// already flattened over `Policy` on Wardryx's own wire, see
/// `genaryx_connectors::PolicyRecord`'s doc comment) - `#[serde(flatten)]`
/// on the connector side becomes plain named fields here, since this app's
/// own wire format has no reason to reproduce Go's embedded-struct wire quirk
/// (true of the former Tauri shell's IPC, and just as true of JSON over HTTP
/// today).
#[derive(Debug, Clone, Serialize)]
pub struct PolicyRecordDto {
    pub id: String,
    pub name: String,
    pub target: String,
    pub deny_tool: Vec<String>,
    pub allow_domains: Vec<String>,
    pub require_human_above_usd: f64,
    pub deny_above_usd: f64,
    pub max_steps: i64,
    pub deny_if_unattested: bool,
    pub updated_at: Option<String>,
}

impl From<&PolicyRecord> for PolicyRecordDto {
    fn from(r: &PolicyRecord) -> Self {
        Self {
            id: r.id.clone(),
            name: r.policy.name.clone(),
            target: r.policy.target.clone(),
            deny_tool: r.policy.deny_tool.clone(),
            allow_domains: r.policy.allow_domains.clone(),
            require_human_above_usd: r.policy.require_human_above_usd,
            deny_above_usd: r.policy.deny_above_usd,
            max_steps: r.policy.max_steps,
            deny_if_unattested: r.policy.deny_if_unattested,
            updated_at: r.updated_at.clone(),
        }
    }
}

/// The operator's verdict on a pending approval - a UI-facing mirror of
/// `genaryx_connectors::ApprovalVerdict`, which carries no serde derives of
/// its own (it is a caller-supplied Rust-side enum, not a wire DTO in the
/// connector).
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionDto {
    Grant,
    Deny,
}

impl DecisionDto {
    fn to_verdict(self) -> ApprovalVerdict {
        match self {
            DecisionDto::Grant => ApprovalVerdict::Grant,
            DecisionDto::Deny => ApprovalVerdict::Deny,
        }
    }

    /// `console_command` `action` value - PHASE2.md Wave-2 spec, verbatim.
    fn action(self) -> &'static str {
        match self {
            DecisionDto::Grant => "console.grant_approval",
            DecisionDto::Deny => "console.deny_approval",
        }
    }
}

/// The decoded claims of a freshly minted `approval_token`, shown to the
/// operator exactly once (Wardryx never lets the token be retrieved again
/// after this response - see `ApprovalDecideResponse::approval_token`'s doc
/// comment). Carries `exp_unix` (not a pre-computed "seconds remaining") so
/// the frontend can drive a live countdown that keeps ticking after this
/// DTO was received, rather than freezing at the value computed the instant
/// the Rust side answered.
#[derive(Debug, Clone, Serialize)]
pub struct DecodedTokenDto {
    pub agent_id: String,
    pub run_id: String,
    pub tools: Vec<String>,
    pub cost_ceiling_usd: f64,
    /// Unix seconds (`ApprovalTokenClaims::exp`).
    pub exp_unix: i64,
}

impl From<&ApprovalTokenClaims> for DecodedTokenDto {
    fn from(c: &ApprovalTokenClaims) -> Self {
        Self {
            agent_id: c.agent_id.clone(),
            run_id: c.run_id.clone(),
            tools: c.tools.clone(),
            cost_ceiling_usd: c.cost_ceiling_usd(),
            exp_unix: c.exp,
        }
    }
}

/// Whole-panel connection state, for the frontend to render up front (never
/// inferred from a read command's error shape) - mirrors
/// `money::commands::MoneyStatusDto`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PolicyStatusDto {
    Bootstrapping,
    NoEnvironment,
    Unreachable {
        source: EnvSource,
        wardryx_url: String,
        reason: String,
    },
    Ready {
        source: EnvSource,
        wardryx_url: String,
        org_domain: String,
    },
}

/// Every error a policy command can return - mirrors
/// `money::commands::MoneyError`'s shape, minus `PlanRequired` (a
/// TokenFuse-Cloud billing concept with no Wardryx equivalent).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PolicyError {
    /// Discovery/liveness-check has not finished yet; retry shortly.
    Bootstrapping,
    NoEnvironment,
    Unreachable {
        reason: String,
    },
    /// Any other Wardryx-side failure: transport, a plain non-2xx, or a
    /// response that failed to parse. `status` is `None` when the request
    /// never got far enough to have one.
    Wardryx {
        status: Option<u16>,
        message: String,
    },
}

impl From<WardryxError> for PolicyError {
    fn from(e: WardryxError) -> Self {
        match e {
            WardryxError::Transport(err) => PolicyError::Wardryx {
                status: None,
                message: format!("could not reach wardryx: {err}"),
            },
            WardryxError::Json(err) => PolicyError::Wardryx {
                status: None,
                message: format!("unexpected response shape from wardryx: {err}"),
            },
            WardryxError::Api { status, body } => PolicyError::Wardryx {
                status: Some(status),
                message: body,
            },
            WardryxError::ApprovalNotFound => PolicyError::Wardryx {
                status: Some(404),
                message: "approval not found".to_string(),
            },
            WardryxError::ApprovalAlreadyDecided => PolicyError::Wardryx {
                status: Some(409),
                message: "approval was already decided".to_string(),
            },
            WardryxError::NoApprovalSecret => PolicyError::Wardryx {
                status: Some(500),
                message: "WARDRYX_APPROVAL_SECRET is not configured on the server; grant refused"
                    .to_string(),
            },
            WardryxError::Forbidden => PolicyError::Wardryx {
                status: Some(403),
                message: "admin role required".to_string(),
            },
            WardryxError::BadToken(msg) => PolicyError::Wardryx {
                status: None,
                message: format!("could not decode approval_token: {msg}"),
            },
            // Refused client-side, before the request was built: an id that
            // cannot be one URL path segment would otherwise address a
            // different route and read as a plain "not found".
            WardryxError::InvalidPathSegment(err) => PolicyError::Wardryx {
                status: None,
                message: format!("this id cannot be used in a request path: {err}"),
            },
        }
    }
}

/// What a successful (or failed-but-journaled) `policy_decide_approval`
/// call returns - mirrors `money::commands::MutationOutcome`'s shape, plus
/// `token` for the grant path.
#[derive(Debug, Clone, Serialize)]
pub struct DecideOutcome {
    pub summary: String,
    pub http_status: u16,
    pub verify_result: String,
    pub sig_alg: String,
    pub sig_fpr: String,
    /// `Some` only on a successful grant whose `approval_token` decoded
    /// cleanly - see [`describe_decision_result`] for the (rare, honestly
    /// reported) cases where a grant succeeds but no token can be shown.
    pub token: Option<DecodedTokenDto>,
    pub bus_recorded: bool,
    pub bus_error: Option<String>,
}

// ============================================================================
// helpers
// ============================================================================

/// The HTTP status implied by a failed connector call, `0` when the request
/// never reached a point where one exists - never fabricated, mirrors
/// `money::commands::status_of`'s identical "an honest 0 beats a made-up
/// 500" rule. [`WardryxError::BadToken`] is never returned by an HTTP call
/// (`ApprovalTokenClaims::decode` is local, offline parsing - see its own
/// doc comment), so it is grouped with the other no-HTTP-status variants.
/// [`WardryxError::InvalidPathSegment`] is grouped there for the same
/// reason: the id is rejected before the request is built, so no HTTP
/// exchange ever happens.
fn status_of(e: &WardryxError) -> u16 {
    match e {
        WardryxError::Api { status, .. } => *status,
        WardryxError::ApprovalNotFound => 404,
        WardryxError::ApprovalAlreadyDecided => 409,
        WardryxError::Forbidden => 403,
        WardryxError::NoApprovalSecret => 500,
        WardryxError::Transport(_)
        | WardryxError::Json(_)
        | WardryxError::InvalidPathSegment(_)
        | WardryxError::BadToken(_) => 0,
    }
}

/// Resolve the current [`PolicyClient`] out of managed state, or the
/// appropriate [`PolicyError`] when the panel is not ready. Only holds the
/// state lock long enough to clone the (cheap, `Arc`-backed) client out -
/// mirrors `money::commands::ready_client` exactly.
async fn ready_client(state: &&PolicyState) -> Result<PolicyClient, PolicyError> {
    let guard = state.inner.lock().await;
    match &*guard {
        PolicyInner::Ready(client) => Ok(client.clone()),
        PolicyInner::Bootstrapping => Err(PolicyError::Bootstrapping),
        PolicyInner::NoEnvironment => Err(PolicyError::NoEnvironment),
        PolicyInner::Unreachable { reason, .. } => Err(PolicyError::Unreachable {
            reason: reason.clone(),
        }),
    }
}

/// Journal one `CommandRecord` (best-effort: a journal failure is reported,
/// never panics and never blocks the caller from learning Wardryx's own
/// verdict) - mirrors `money::commands::journal` exactly, typed against
/// [`PolicyClient`].
fn journal(client: &PolicyClient, rec: &CommandRecord) -> (bool, Option<String>) {
    let Some(bus) = &client.bus else {
        return (
            false,
            Some("no live event bus available (startup seeding did not complete)".to_string()),
        );
    };
    match genaryx_core::store::Store::open(&bus.store_db_path) {
        Ok(store) => match command::record(
            &store,
            &bus.console_events_path,
            &client.org_domain,
            &client.host,
            rec,
        ) {
            Ok(()) => (true, None),
            Err(e) => (false, Some(e.to_string())),
        },
        Err(e) => (false, Some(e.to_string())),
    }
}

/// Turn the connector's raw `decide_approval` result into everything
/// `policy_decide_approval` needs to both journal and answer the frontend:
/// the `CommandRecord`'s `http_status`/`verify_result`, the decoded token
/// (grant only, and only when decodable), and a human-readable summary for
/// the UI's post-mutation notice banner (matching
/// `MutationOutcome::summary`'s role; left empty on error, since
/// `PolicyView`'s notice banner is only ever built from the `Ok` path -
/// same convention `money::commands::finish_mutation` follows).
///
/// Fail-closed throughout: a grant that succeeds at the HTTP layer but
/// returns no token, or a token that fails to decode, is still reported as
/// `http_status: 200` (Wardryx's own verdict was a genuine grant) with an
/// honest `verify_result` explaining exactly what could not be shown -
/// never silently dropped, never escalated into a fake error.
fn describe_decision_result(
    decision: DecisionDto,
    id: &str,
    result: &Result<ApprovalDecideResponse, WardryxError>,
) -> (u16, String, Option<DecodedTokenDto>, String) {
    match result {
        Ok(resp) => match decision {
            DecisionDto::Grant => match &resp.approval_token {
                Some(tok) => match ApprovalTokenClaims::decode(tok) {
                    Ok(claims) => {
                        let ttl_s = claims.ttl_remaining(SystemTime::now()).as_secs();
                        let verify_result = format!(
                            "granted ceiling_usd:{:.2} ttl_s:{ttl_s}",
                            claims.cost_ceiling_usd()
                        );
                        (
                            200,
                            verify_result,
                            Some(DecodedTokenDto::from(&claims)),
                            format!("approval {id} granted"),
                        )
                    }
                    Err(e) => (
                        200,
                        format!("granted (token undecodable: {e})"),
                        None,
                        format!("approval {id} granted (token undecodable)"),
                    ),
                },
                None => (
                    200,
                    "granted (no token returned)".to_string(),
                    None,
                    format!("approval {id} granted (no token returned)"),
                ),
            },
            DecisionDto::Deny => (
                200,
                "denied".to_string(),
                None,
                format!("approval {id} denied"),
            ),
        },
        Err(e) => (status_of(e), format!("error: {e}"), None, String::new()),
    }
}

// ============================================================================
// commands: status + reads
// ============================================================================

/// Whole-panel connection state. Never fails: every outcome of
/// [`super::state::bootstrap`] is a renderable [`PolicyStatusDto`] variant.
pub async fn policy_status(state: &PolicyState) -> Result<PolicyStatusDto, ()> {
    let guard = state.inner.lock().await;
    Ok(match &*guard {
        PolicyInner::Bootstrapping => PolicyStatusDto::Bootstrapping,
        PolicyInner::NoEnvironment => PolicyStatusDto::NoEnvironment,
        PolicyInner::Unreachable {
            source,
            wardryx_url,
            reason,
        } => PolicyStatusDto::Unreachable {
            source: source.clone(),
            wardryx_url: wardryx_url.clone(),
            reason: reason.clone(),
        },
        PolicyInner::Ready(client) => PolicyStatusDto::Ready {
            source: client.source.clone(),
            wardryx_url: client.wardryx_url.clone(),
            org_domain: client.org_domain.clone(),
        },
    })
}

/// The Approvals Inbox's full queue (pending holds plus decided history -
/// the frontend splits on `pending`, mirroring how `money`'s `IncidentsList`
/// splits acknowledged/unacknowledged client-side rather than via two
/// separate commands). `WardryxClient::list_approvals`'s own doc comment
/// guarantees ascending `requested_at` order, preserved here.
pub async fn policy_list_approvals(state: &PolicyState) -> Result<Vec<ApprovalDto>, PolicyError> {
    let client = ready_client(&state).await?;
    let approvals = client
        .client
        .list_approvals()
        .await
        .map_err(PolicyError::from)?;
    Ok(approvals.iter().map(ApprovalDto::from).collect())
}

/// The Policy view's read-only policy list (PHASE2.md Wave 2: "Read-only in
/// MVP - the guarded PUT/DELETE editor is v1"). `GET /v1/policies` carries
/// no set-level `policy_version` of its own (see
/// `genaryx_connectors::PolicyRecord`'s doc comment - the response is a
/// bare array with nowhere to put one); the frontend derives a best-effort
/// `policy_version` from the most recent entry in the approvals list it
/// already fetches for the inbox, rather than this command inventing a
/// second, possibly-inconsistent source for the same value.
pub async fn policy_list_policies(
    state: &PolicyState,
) -> Result<Vec<PolicyRecordDto>, PolicyError> {
    let client = ready_client(&state).await?;
    let policies = client
        .client
        .list_policies()
        .await
        .map_err(PolicyError::from)?;
    Ok(policies.iter().map(PolicyRecordDto::from).collect())
}

/// What the policy plane is ACTUALLY enforcing, from Wardryx's own
/// `/v1/status`.
///
/// This is not a nicer [`policy_list_policies`]. That command lists the
/// STORE's operator-managed policies, which is an empty list on every
/// deployment whose rules come from a `-policy` file - while all of those
/// rules are being enforced on `/v1/decide`. A posture check reading the list
/// concludes the fleet is unguarded and says so, which is worse than saying
/// nothing: an operator who verifies that claim once stops believing the rest
/// of the panel. `effective_policies` is the number that answers the question
/// the check is actually asking.
pub async fn policy_enforcement_status(
    state: &PolicyState,
) -> Result<genaryx_connectors::WardryxStatus, PolicyError> {
    let client = ready_client(&state).await?;
    client.client.status().await.map_err(PolicyError::from)
}

// ============================================================================
// commands: the one privileged mutation
// ============================================================================

/// Grant or deny a pending approval. A privileged mutation: the frontend
/// gates this behind an explicit confirm ceremony (`ConfirmButton`, the web
/// console's confirm-dialog substitute for the former desktop Touch ID gate -
/// PHASE2.md).
///
/// Journals a `console_command` via `genaryx_core::command::record`
/// regardless of Wardryx's verdict (`action`
/// `console.grant_approval`/`console.deny_approval`, `decision` always
/// `"allow"` - PHASE2.md is explicit that granting/denying through this
/// sanctioned human-in-the-loop path is not a `break_glass` override,
/// `target` = `approval_id`), then returns Wardryx's own verdict as
/// `Ok`/`Err` - see [`describe_decision_result`] and this module's doc
/// comment for the fail-closed split between the two.
pub async fn policy_decide_approval(
    id: String,
    decision: DecisionDto,
    state: &PolicyState,
) -> Result<DecideOutcome, PolicyError> {
    let client = ready_client(&state).await?;
    // The web shell's signed-in principal when set (docs/CONSOLE-IDP.md), else
    // this client's own default. Used both as the approver Wardryx records and
    // as the journaled operator, so the two never disagree.
    let operator = crate::console_actor::operator_or(&client.operator);
    let result = client
        .client
        .decide_approval(&id, decision.to_verdict(), &operator)
        .await;

    let (http_status, verify_result, token, summary) =
        describe_decision_result(decision, &id, &result);

    // The web shell's per-action WebAuthn ceremony when one confirmed this
    // request (docs/CONSOLE-IDP.md B3/2), else this client's own
    // transport-signing fields - same override pattern as the operator above.
    let (sig_alg, sig_fpr) = crate::console_actor::signature_or(client.sig_alg, client.sig_fpr);
    let rec = CommandRecord {
        operator: operator.clone(),
        env: "local".to_string(),
        action: decision.action().to_string(),
        target: id.clone(),
        params: json!({}),
        decision: "allow".to_string(),
        sig_alg,
        sig_fpr,
        http_status,
        verify_result: verify_result.clone(),
    };
    let (bus_recorded, bus_error) = journal(&client, &rec);

    match result {
        Ok(_) => Ok(DecideOutcome {
            summary,
            http_status,
            verify_result,
            sig_alg: client.sig_alg.to_string(),
            sig_fpr: client.sig_fpr.to_string(),
            token,
            bus_recorded,
            bus_error,
        }),
        Err(e) => Err(PolicyError::from(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A minimal, dependency-free base64url-no-pad encoder for building a
    // syntactically valid (never cryptographically verified - see
    // `ApprovalTokenClaims::decode`'s own doc comment) synthetic
    // `approval_token` in tests. This crate does not otherwise depend on a
    // base64 crate, and pulling one in just for this one test seemed like
    // the wrong trade - this is ~15 lines and only ever compiled for
    // `cfg(test)`.
    fn b64url_nopad(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b0 = u32::from(chunk[0]);
            let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
            let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
            if chunk.len() > 1 {
                out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
            }
            if chunk.len() > 2 {
                out.push(ALPHABET[(n & 0x3f) as usize] as char);
            }
        }
        out
    }

    fn synthetic_token(claims_json: &str) -> String {
        format!(
            "{}.fake-sig-never-verified",
            b64url_nopad(claims_json.as_bytes())
        )
    }

    #[test]
    fn describe_decision_result_deny_success() {
        let resp = ApprovalDecideResponse {
            approval_id: "ap_1".to_string(),
            decision: "deny".to_string(),
            approval_token: None,
        };
        let (status, verify_result, token, summary) =
            describe_decision_result(DecisionDto::Deny, "ap_1", &Ok(resp));
        assert_eq!(status, 200);
        assert_eq!(verify_result, "denied");
        assert!(token.is_none());
        assert_eq!(summary, "approval ap_1 denied");
    }

    #[test]
    fn describe_decision_result_grant_with_no_token_is_reported_honestly() {
        let resp = ApprovalDecideResponse {
            approval_id: "ap_1".to_string(),
            decision: "grant".to_string(),
            approval_token: None,
        };
        let (status, verify_result, token, summary) =
            describe_decision_result(DecisionDto::Grant, "ap_1", &Ok(resp));
        assert_eq!(status, 200);
        assert_eq!(verify_result, "granted (no token returned)");
        assert!(token.is_none());
        assert!(summary.contains("no token returned"));
    }

    #[test]
    fn describe_decision_result_grant_with_undecodable_token_is_reported_honestly() {
        let resp = ApprovalDecideResponse {
            approval_id: "ap_1".to_string(),
            decision: "grant".to_string(),
            approval_token: Some("not-a-valid-token-no-dot".to_string()),
        };
        let (status, verify_result, token, summary) =
            describe_decision_result(DecisionDto::Grant, "ap_1", &Ok(resp));
        assert_eq!(status, 200);
        assert!(
            verify_result.starts_with("granted (token undecodable:"),
            "got {verify_result:?}"
        );
        assert!(token.is_none());
        assert!(summary.contains("token undecodable"));
    }

    #[test]
    fn describe_decision_result_grant_with_valid_token_decodes_and_formats() {
        let claims_json = r#"{"agent_id":"agent://acme/payments","run_id":"run-1","tools":["charge"],
            "max_cost_usd":50.0,"exp":9999999999,"nonce":"n1"}"#;
        let token = synthetic_token(claims_json);
        let resp = ApprovalDecideResponse {
            approval_id: "ap_1".to_string(),
            decision: "grant".to_string(),
            approval_token: Some(token),
        };

        let (status, verify_result, token_dto, summary) =
            describe_decision_result(DecisionDto::Grant, "ap_1", &Ok(resp));
        assert_eq!(status, 200);
        assert!(
            verify_result.starts_with("granted ceiling_usd:50.00 ttl_s:"),
            "got {verify_result:?}"
        );
        let dto = token_dto.expect("a valid token must decode");
        assert_eq!(dto.agent_id, "agent://acme/payments");
        assert_eq!(dto.run_id, "run-1");
        assert_eq!(dto.tools, vec!["charge".to_string()]);
        assert!((dto.cost_ceiling_usd - 50.0).abs() < f64::EPSILON);
        assert_eq!(dto.exp_unix, 9_999_999_999);
        assert_eq!(summary, "approval ap_1 granted");
    }

    #[test]
    fn describe_decision_result_error_is_never_a_fabricated_ok() {
        let err = WardryxError::ApprovalAlreadyDecided;
        let (status, verify_result, token, summary) =
            describe_decision_result(DecisionDto::Grant, "ap_1", &Err(err));
        assert_eq!(status, 409);
        assert!(verify_result.starts_with("error:"), "got {verify_result:?}");
        assert!(token.is_none());
        assert!(
            summary.is_empty(),
            "no UI summary is built on the error path"
        );
    }

    #[test]
    fn status_of_maps_every_variant_to_an_honest_status() {
        assert_eq!(
            status_of(&WardryxError::Api {
                status: 418,
                body: String::new()
            }),
            418
        );
        assert_eq!(status_of(&WardryxError::ApprovalNotFound), 404);
        assert_eq!(status_of(&WardryxError::ApprovalAlreadyDecided), 409);
        assert_eq!(status_of(&WardryxError::Forbidden), 403);
        assert_eq!(status_of(&WardryxError::NoApprovalSecret), 500);
        assert_eq!(status_of(&WardryxError::BadToken("x".to_string())), 0);
    }

    #[test]
    fn policy_error_from_wardryx_error_preserves_status_and_message() {
        let e = PolicyError::from(WardryxError::Forbidden);
        assert!(matches!(
            e,
            PolicyError::Wardryx {
                status: Some(403),
                ..
            }
        ));

        let e = PolicyError::from(WardryxError::ApprovalNotFound);
        assert!(matches!(
            e,
            PolicyError::Wardryx {
                status: Some(404),
                ..
            }
        ));

        let e = PolicyError::from(WardryxError::Api {
            status: 400,
            body: "bad input".to_string(),
        });
        match e {
            PolicyError::Wardryx {
                status: Some(400),
                message,
            } => assert_eq!(message, "bad input"),
            other => panic!("expected Wardryx{{400,..}}, got {other:?}"),
        }
    }

    #[test]
    fn decision_dto_action_and_verdict_mapping() {
        assert_eq!(DecisionDto::Grant.action(), "console.grant_approval");
        assert_eq!(DecisionDto::Deny.action(), "console.deny_approval");
        assert_eq!(DecisionDto::Grant.to_verdict(), ApprovalVerdict::Grant);
        assert_eq!(DecisionDto::Deny.to_verdict(), ApprovalVerdict::Deny);
    }

    #[test]
    fn approval_dto_from_flattens_context_via_typed_accessors() {
        let json = serde_json::json!({
            "approval_id": "ap_1",
            "agent_id": "agent://acme/payments",
            "run_id": "run-1",
            "requested_at": "2026-07-17T00:00:00Z",
            "pending": true,
            "context": {
                "org": "acme",
                "est_cost_usd": 12.5,
                "reason": "over threshold",
                "tool_names": ["charge"],
                "policy_version": "abc123",
                "on_behalf_of": null
            }
        });
        let approval: Approval = serde_json::from_value(json).expect("valid Approval fixture");
        let dto = ApprovalDto::from(&approval);
        assert_eq!(dto.approval_id, "ap_1");
        assert_eq!(dto.tool_names, vec!["charge".to_string()]);
        assert_eq!(dto.est_cost_usd, Some(12.5));
        assert_eq!(dto.reason.as_deref(), Some("over threshold"));
        assert_eq!(dto.policy_version.as_deref(), Some("abc123"));
        assert_eq!(dto.org.as_deref(), Some("acme"));
        assert_eq!(dto.on_behalf_of, None);
        assert!(dto.pending);
    }

    #[test]
    fn policy_record_dto_from_flattens_the_embedded_policy() {
        let json = serde_json::json!({
            "id": "demo",
            "target": "agent://acme/*",
            "deny_tool": ["shell_exec"],
            "require_human_above_usd": 1.0,
            "deny_above_usd": 1000.0,
            "updated_at": "2026-07-17T05:33:20Z"
        });
        let record: PolicyRecord =
            serde_json::from_value(json).expect("valid PolicyRecord fixture");
        let dto = PolicyRecordDto::from(&record);
        assert_eq!(dto.id, "demo");
        assert_eq!(dto.target, "agent://acme/*");
        assert_eq!(dto.deny_tool, vec!["shell_exec".to_string()]);
        assert!((dto.require_human_above_usd - 1.0).abs() < f64::EPSILON);
        assert_eq!(dto.updated_at.as_deref(), Some("2026-07-17T05:33:20Z"));
    }
}
