//! D8 commands: list the gateway's retained reservations, and close one with
//! evidence.
//!
//! - [`money_list_retained`] is a read: `GET /v1/retained` on the gateway,
//!   through the Credentials plane's `GatewayClient` and its admin key, the
//!   same client `credentials_gateway_retained_runs` reads `GET /v1/runs`
//!   with.
//! - [`money_close_retained`] is sensitive: the web shell runs the passkey
//!   ceremony before dispatching it (the ceremony column), with
//!   `money_set_budget`'s role. It validates the evidence, generates the
//!   operation id, mints a two-minute token ([`super::closure`]), and posts
//!   `POST /v1/retained/close` with the admin key and the token in
//!   `x-fuse-closure-token`.
//!
//! **Every attempt is journaled**, whatever happens to it: refused here
//! (invalid evidence, no secret, no gateway), refused by the gateway, not
//! confirmed, unreachable, or closed. One `console.close_retained` record per
//! attempt in the console's own command journal, carrying the acting
//! principal (`console_actor::operator_or`), the passkey that confirmed it
//! (`console_actor::signature_or`), the gateway's REAL status (0 when no
//! answer came), and the evidence in `params`. Never the secret, never the
//! token.
//!
//! **A 200 is not proof by itself**: only a body carrying `"closed":true` is.
//! Anything else answering 200 (a wrong URL, a proxy page) is
//! [`RetainedError::Unconfirmed`], journaled as such, the rule
//! `delegation::commands` already keeps for vouchryx.
//!
//! **Not here**: an administrative adjustment, or releasing a reservation on
//! a timer. The gateway takes neither, and this console offers neither.

use super::closure::{self, ClosureConfig};
use super::state::BusHandle;
use crate::credentials::state::{CredentialsInner, CredentialsState};
use genaryx_connectors::{GatewayError, GatewayRetainedReservation};
use genaryx_core::CommandRecord;
use serde::{Deserialize, Serialize};

/// The gateway's bounds on the caller's evidence reference
/// (`MAX_EVIDENCE_REF_BYTES` in tokenfuse's `closure.rs`).
const MAX_EVIDENCE_REF_BYTES: usize = 512;

/// The two forms a closure may take, exactly the gateway's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// The provider's returned usage, or the invoice line covering the call:
    /// settles at `observed_microusd`.
    Observed,
    /// The provider confirms nothing was billed: releases at zero.
    NoCharge,
}

impl EvidenceKind {
    fn as_str(self) -> &'static str {
        match self {
            EvidenceKind::Observed => "observed",
            EvidenceKind::NoCharge => "no_charge",
        }
    }
}

/// What the operator supplies to close one reservation. The operation id is
/// NOT here: the console generates it, so a retry from the browser is a new
/// attempt rather than a replay the operator has to manage.
#[derive(Debug, Clone, Deserialize)]
pub struct CloseRetainedArgs {
    pub reservation_id: u64,
    pub expected_revision: String,
    pub evidence_kind: EvidenceKind,
    #[serde(default)]
    pub observed_microusd: Option<i64>,
    pub evidence_ref: String,
}

/// Every way listing or closing can fail, each reported as itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RetainedError {
    /// `GENARYX_CLOSURE_SECRET_FILE` is unset: this console signs nothing.
    NotConfigured,
    /// The gateway plane is still resolving.
    Bootstrapping,
    /// No gateway is configured for this environment.
    NoEnvironment,
    /// The gateway plane resolved but its probe failed.
    GatewayUnreachable { reason: String },
    /// The evidence does not fit its kind, or a field is outside the
    /// gateway's bounds; refused before any call.
    InvalidArgs { reason: String },
    /// The gateway answered a bare 404: its closure routes are off
    /// (`TOKENFUSE_CLOSURE_SECRET` unset there).
    ClosureOff,
    /// The gateway answered and declined. `error` is its own refusal kind
    /// (`stale_revision`, `already_closed`, `closure_token_expired`,
    /// `unauthorized`, ...), `message` its own words.
    Refused {
        status: u16,
        error: String,
        message: String,
    },
    /// A 2xx that did not confirm the closure, or a list that would not
    /// parse. `body` is what actually answered.
    Unconfirmed {
        status: u16,
        body: serde_json::Value,
    },
    /// No usable answer: connection, timeout, or a body too large to read.
    Unreachable { detail: String },
}

/// A closure the gateway confirmed.
#[derive(Debug, Clone, Serialize)]
pub struct CloseOutcome {
    pub http_status: u16,
    pub verify_result: String,
    /// The id this console generated for the attempt.
    pub operation_id: String,
    /// The gateway answered from its books: this operation id had already
    /// closed this reservation with the same content.
    pub replayed: bool,
    pub settled_microusd: Option<i64>,
    /// The gateway's own answer, forwarded whole.
    pub gateway_response: serde_json::Value,
    pub bus_recorded: bool,
    pub bus_error: Option<String>,
}

/// `GET /v1/retained`: the reservations a person may close, with the
/// revision each token must bind. A gateway that cannot answer says so; it
/// is never shown as an empty list.
pub async fn money_list_retained(
    gateway: &CredentialsState,
) -> Result<Vec<GatewayRetainedReservation>, RetainedError> {
    let client = ready_gateway(gateway).await?;
    client.get_retained().await.map_err(list_error)
}

/// Close one retained reservation with evidence. See the module doc.
pub async fn money_close_retained(
    args: CloseRetainedArgs,
    config: &ClosureConfig,
    gateway: &CredentialsState,
    bus: Option<&BusHandle>,
) -> Result<CloseOutcome, RetainedError> {
    let operation_id = new_operation_id();
    let attempt = Attempt {
        args: &args,
        operation_id: &operation_id,
    };

    if let Err(reason) = validate(&args) {
        let err = RetainedError::InvalidArgs { reason };
        attempt.journal(bus, 0, "refused:invalid_args".to_string());
        return Err(err);
    }
    let secret = match config {
        ClosureConfig::NotConfigured => {
            attempt.journal(bus, 0, "refused:not_configured".to_string());
            return Err(RetainedError::NotConfigured);
        }
        ClosureConfig::Configured { secret } => secret,
    };
    let client = match ready_gateway(gateway).await {
        Ok(c) => c,
        Err(e) => {
            attempt.journal(bus, 0, format!("refused:{}", kind_of(&e)));
            return Err(e);
        }
    };

    let token = closure::mint_token(
        secret.as_bytes(),
        args.reservation_id,
        &args.expected_revision,
        closure::expiry_for(now_millis()),
    );
    let body = serde_json::json!({
        "reservation_id": args.reservation_id,
        "expected_revision": args.expected_revision,
        "operation_id": operation_id,
        "evidence_kind": args.evidence_kind.as_str(),
        "observed_microusd": args.observed_microusd,
        "evidence_ref": args.evidence_ref,
    });
    let answer = client.close_retained(&body, &token).await;
    drop(token);

    let answer = match answer {
        Ok(a) => a,
        Err(e) => {
            let detail = e.to_string();
            attempt.journal(bus, 0, format!("unreachable: {detail}"));
            return Err(RetainedError::Unreachable { detail });
        }
    };

    let status = answer.status;
    if (200..300).contains(&status) {
        let confirmed = answer.body.get("closed").and_then(|v| v.as_bool()) == Some(true);
        if !confirmed {
            attempt.journal(
                bus,
                status,
                format!("{status} without closed:true confirmation"),
            );
            return Err(RetainedError::Unconfirmed {
                status,
                body: answer.body,
            });
        }
        let replayed = answer.body.get("replayed").and_then(|v| v.as_bool()) == Some(true);
        let verify_result = if replayed {
            "closed (replayed)"
        } else {
            "closed"
        }
        .to_string();
        let (bus_recorded, bus_error) = attempt.journal(bus, status, verify_result.clone());
        return Ok(CloseOutcome {
            http_status: status,
            verify_result,
            operation_id,
            replayed,
            settled_microusd: answer.body.get("settled_microusd").and_then(|v| v.as_i64()),
            gateway_response: answer.body,
            bus_recorded,
            bus_error,
        });
    }

    let (error, message) = refusal_of(&answer.body);
    if status == 404 && error.is_none() {
        attempt.journal(bus, status, "refused:closure_off".to_string());
        return Err(RetainedError::ClosureOff);
    }
    let error = error.unwrap_or_else(|| "unknown".to_string());
    attempt.journal(bus, status, format!("refused:{error}"));
    Err(RetainedError::Refused {
        status,
        error,
        message,
    })
}

/// The evidence rules the gateway applies, checked here first so a request
/// it would refuse never carries a token.
fn validate(args: &CloseRetainedArgs) -> Result<(), String> {
    let rev = &args.expected_revision;
    if rev.len() != 16 || !rev.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(
            "expected_revision must be the 16 lowercase hex digits GET /v1/retained lists".into(),
        );
    }
    let ev = &args.evidence_ref;
    if ev.trim().is_empty() || ev.len() > MAX_EVIDENCE_REF_BYTES || ev.chars().any(char::is_control)
    {
        return Err(format!(
            "evidence_ref must name the evidence in 1 to {MAX_EVIDENCE_REF_BYTES} bytes with no \
             control characters"
        ));
    }
    match (args.evidence_kind, args.observed_microusd) {
        (EvidenceKind::Observed, None) => {
            Err("an observed closure needs observed_microusd, the cost the evidence shows".into())
        }
        (EvidenceKind::Observed, Some(c)) if c < 0 => {
            Err("observed_microusd cannot be negative".into())
        }
        (EvidenceKind::NoCharge, Some(_)) => {
            Err("a no_charge closure carries no observed_microusd: it releases at zero".into())
        }
        _ => Ok(()),
    }
}

/// The gateway's refusal kind and message: `{"type":"error","error":{"type",
/// "message"}}` from `closure.rs`, or `{"error":"unauthorized"}` from the
/// admin gate in front of it. `None` when the body names no kind at all.
fn refusal_of(body: &serde_json::Value) -> (Option<String>, String) {
    match body.get("error") {
        Some(serde_json::Value::Object(o)) => (
            o.get("type").and_then(|v| v.as_str()).map(str::to_string),
            o.get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        ),
        Some(serde_json::Value::String(s)) => (Some(s.clone()), String::new()),
        _ => (None, String::new()),
    }
}

fn list_error(e: GatewayError) -> RetainedError {
    match e {
        GatewayError::Transport(err) => RetainedError::Unreachable {
            detail: err.to_string(),
        },
        GatewayError::ResponseTooLarge { limit } => RetainedError::Unreachable {
            detail: format!("the gateway's response exceeded {limit} bytes"),
        },
        GatewayError::Json(err) => RetainedError::Unconfirmed {
            status: 200,
            body: serde_json::Value::String(format!("not a retained list: {err}")),
        },
        GatewayError::Unauthorized => RetainedError::Refused {
            status: 401,
            error: "unauthorized".into(),
            message: "the gateway refused the console's key: check TOKENFUSE_GATEWAY_ADMIN_KEY \
                      against the gateway's TOKENFUSE_ADMIN_KEYS"
                .into(),
        },
        GatewayError::AdminKeysRequired => RetainedError::Refused {
            status: 403,
            error: "admin_keys_required".into(),
            message: "the gateway requires TOKENFUSE_ADMIN_KEYS on a non-loopback bind and the \
                      console holds no key (TOKENFUSE_GATEWAY_ADMIN_KEY is unset)"
                .into(),
        },
        GatewayError::Api { status, body } => {
            let parsed = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            match (status, refusal_of(&parsed)) {
                (404, (None, _)) => RetainedError::ClosureOff,
                (_, (error, message)) => RetainedError::Refused {
                    status,
                    error: error.unwrap_or_else(|| "unknown".into()),
                    message: if message.is_empty() { body } else { message },
                },
            }
        }
    }
}

async fn ready_gateway(
    state: &CredentialsState,
) -> Result<std::sync::Arc<genaryx_connectors::GatewayClient>, RetainedError> {
    match &*state.inner.lock().await {
        CredentialsInner::Ready(c) => Ok(c.client.clone()),
        CredentialsInner::Bootstrapping => Err(RetainedError::Bootstrapping),
        CredentialsInner::NoEnvironment => Err(RetainedError::NoEnvironment),
        CredentialsInner::Unreachable { reason, .. } => Err(RetainedError::GatewayUnreachable {
            reason: reason.clone(),
        }),
    }
}

fn kind_of(e: &RetainedError) -> &'static str {
    match e {
        RetainedError::NotConfigured => "not_configured",
        RetainedError::Bootstrapping => "bootstrapping",
        RetainedError::NoEnvironment => "no_environment",
        RetainedError::GatewayUnreachable { .. } => "gateway_unreachable",
        RetainedError::InvalidArgs { .. } => "invalid_args",
        RetainedError::ClosureOff => "closure_off",
        RetainedError::Refused { .. } => "refused",
        RetainedError::Unconfirmed { .. } => "unconfirmed",
        RetainedError::Unreachable { .. } => "unreachable",
    }
}

/// `genaryx-<32 hex>`: 128 random bits, inside the gateway's 1 to 128
/// visible-ASCII bound. A failed random source falls back to the clock and
/// the process id, still unique per attempt in practice and never empty.
fn new_operation_id() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::getrandom(&mut bytes).is_err() {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        bytes[..12].copy_from_slice(&n.to_le_bytes()[..12]);
        bytes[12..].copy_from_slice(&std::process::id().to_le_bytes());
    }
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("genaryx-{hex}")
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// One closure attempt, for the journal.
struct Attempt<'a> {
    args: &'a CloseRetainedArgs,
    operation_id: &'a str,
}

impl Attempt<'_> {
    /// Journal this attempt with the gateway's real `http_status` (0 for no
    /// answer) and a short `verify_result`. Best-effort: a journal failure is
    /// reported and logged, never turned into a different outcome.
    fn journal(
        &self,
        bus: Option<&BusHandle>,
        http_status: u16,
        verify_result: String,
    ) -> (bool, Option<String>) {
        let org_domain =
            std::env::var("GENARYX_ORG_DOMAIN").unwrap_or_else(|_| "local".to_string());
        let operator = crate::console_actor::operator_or(&format!("user://{org_domain}/operator"));
        let (sig_alg, sig_fpr) =
            crate::console_actor::signature_or("software-signed", "software-signed");
        let a = self.args;
        let rec = CommandRecord {
            operator,
            env: org_domain.clone(),
            action: "console.close_retained".to_string(),
            target: format!("reservation:{}", a.reservation_id),
            params: serde_json::json!({
                "reservation_id": a.reservation_id,
                "expected_revision": clip(&a.expected_revision, 64),
                "operation_id": self.operation_id,
                "evidence_kind": a.evidence_kind.as_str(),
                "observed_microusd": a.observed_microusd,
                "evidence_ref": clip(&a.evidence_ref, MAX_EVIDENCE_REF_BYTES),
            }),
            decision: "allow".to_string(),
            sig_alg,
            sig_fpr,
            http_status,
            verify_result,
        };
        crate::delegation::commands::journal(bus, &org_domain, &rec)
    }
}

/// `s` cut to at most `max` bytes on a character boundary: a refused attempt
/// is journaled too, and its oversized field must not become an oversized row.
fn clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> CloseRetainedArgs {
        CloseRetainedArgs {
            reservation_id: 1,
            expected_revision: "0011223344556677".into(),
            evidence_kind: EvidenceKind::Observed,
            observed_microusd: Some(0),
            evidence_ref: "ref".into(),
        }
    }

    #[test]
    fn a_zero_observed_cost_and_a_512_byte_reference_are_accepted() {
        assert!(validate(&args()).is_ok());
        let edge = CloseRetainedArgs {
            evidence_ref: "x".repeat(512),
            ..args()
        };
        assert!(validate(&edge).is_ok());
    }

    #[test]
    fn the_gateways_two_refusal_shapes_are_both_read() {
        let (e, m) = refusal_of(&serde_json::json!(
            {"type":"error","error":{"type":"already_closed","message":"m"}}
        ));
        assert_eq!((e.as_deref(), m.as_str()), (Some("already_closed"), "m"));
        let (e, _) = refusal_of(&serde_json::json!({"error":"unauthorized"}));
        assert_eq!(e.as_deref(), Some("unauthorized"));
        assert_eq!(refusal_of(&serde_json::Value::Null).0, None);
    }

    #[test]
    fn operation_ids_are_fresh_and_within_the_gateways_bounds() {
        let a = new_operation_id();
        let b = new_operation_id();
        assert_ne!(a, b);
        assert!(
            a.len() <= 128 && a.bytes().all(|b| b.is_ascii_graphic()),
            "{a}"
        );
    }

    #[test]
    fn clip_never_splits_a_character() {
        assert_eq!(clip("héllo", 2), "h");
        assert_eq!(clip("abc", 10), "abc");
    }

    /// 200 seeds of hostile evidence: validation never panics, and what it
    /// accepts is always inside the gateway's own rules.
    #[test]
    fn two_hundred_seeds_of_hostile_evidence_never_panic_or_half_validate() {
        let mut x: u64 = 0xdead_beef_cafe_f00d;
        let pool = [
            "", " ", "\n", "\u{0}", "é", "a", "0", "f", "G", "-", "\u{7f}",
        ];
        for _ in 0..200 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let pick = |n: u64| pool[(n % pool.len() as u64) as usize];
            let rev: String = (0..(x % 20)).map(|i| pick(x >> i)).collect();
            let evr: String = (0..(x % 600))
                .map(|i| pick(x.rotate_left(i as u32)))
                .collect();
            let a = CloseRetainedArgs {
                reservation_id: x,
                expected_revision: rev,
                evidence_kind: if x & 1 == 0 {
                    EvidenceKind::Observed
                } else {
                    EvidenceKind::NoCharge
                },
                observed_microusd: if x & 2 == 0 {
                    None
                } else {
                    Some((x as i64) >> 3)
                },
                evidence_ref: evr,
            };
            if validate(&a).is_ok() {
                assert_eq!(a.expected_revision.len(), 16);
                assert!(!a.evidence_ref.trim().is_empty());
                assert!(a.evidence_ref.len() <= 512);
                assert!(!a.evidence_ref.chars().any(char::is_control));
                match a.evidence_kind {
                    EvidenceKind::Observed => assert!(a.observed_microusd.is_some_and(|c| c >= 0)),
                    EvidenceKind::NoCharge => assert!(a.observed_microusd.is_none()),
                }
            }
        }
    }
}
