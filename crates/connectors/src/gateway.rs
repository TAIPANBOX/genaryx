//! `GatewayClient`: a typed REST client for the TokenFuse gateway's key
//! lifecycle report (`GET /v1/keys`) - the FIRST direct gateway REST read
//! this console makes (I15, "key lifecycle health"). Every other place this
//! codebase talks to the gateway either fires traffic AT it (Mockryx,
//! docs/PHASE4.md W2) or just names its URL for something else to resolve
//! (`drills::env`/`money::env` read `services.gateway.url` off a `taipan up`
//! descriptor without ever calling it directly). Contract source:
//! `docs/22-key-lifecycle.md` in the tokenfuse repo, built in parallel
//! against this exact wire shape.
//!
//! ## An optional bearer, and the secret never appears in a log
//!
//! The gateway is loopback/perimeter-bound by default (same posture idryx's
//! own module doc argues for, `crates/connectors/src/idryx.rs`), and on that
//! default bind this client still sends no bearer at all - `/v1/keys` is an
//! operator-facing admin read, mirroring idryx's connector shape. But
//! tokenfuse main (`crates/gateway/src/adminkeys.rs`) also gates `/v1/keys`
//! and four sibling routes behind `TOKENFUSE_ADMIN_KEYS` once the gateway is
//! bound off loopback, and a console reaching a gated gateway has to present
//! a key or be refused. [`GatewayClient::with_admin_key`] carries that key;
//! `None` (the default from [`GatewayClient::new`]) sends no
//! `Authorization` header at all, matching a gateway with nothing
//! configured. The key is never logged and never appears in `{:?}` output
//! (see the manual [`std::fmt::Debug`] impl below) - unlike a
//! `TOKENFUSE_CLIENT_KEYS` entry (`<secret>:<key_id>`, docs/ONBOARD.md), the
//! report this client reads carries only `key_id`, the non-secret half, so
//! the only secret this module ever touches is the admin key it sends, never
//! one it receives.
//!
//! ## Fail-closed (06 §0.5) and forward-tolerant
//!
//! A transport failure becomes [`GatewayError::Transport`]; a 401 (the
//! gateway is keyed and refused the presented key, or none was presented)
//! becomes [`GatewayError::Unauthorized`]; a 403 with
//! `{"error":"admin_keys_required"}` (the gateway is unkeyed and bound off
//! loopback, so it refuses every request regardless of what is presented)
//! becomes [`GatewayError::AdminKeysRequired`]; any other non-2xx becomes
//! [`GatewayError::Api`] with the raw status/body; a 2xx body that will not
//! deserialize becomes [`GatewayError::Json`]. No panics, no `unwrap`.
//! Every DTO tolerates unknown extra JSON fields (plain `#[serde(default)]`
//! throughout, no `deny_unknown_fields` anywhere) - the tokenfuse side of
//! this contract is being built in parallel against the same shape, so a
//! field this client does not yet know about must never break parsing.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

// ---- error -----------------------------------------------------------------

/// Every failure mode a [`GatewayClient`] call can surface. Fail-closed
/// throughout, mirroring `IdryxError`'s identical three-way split.
#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    /// The request never got a response (DNS, connect, timeout, or a body
    /// that failed to read).
    #[error("http transport: {0}")]
    Transport(#[from] reqwest::Error),

    /// A 2xx body that failed to deserialize into the expected shape - this
    /// client's DTOs have drifted from the live gateway, or it sent
    /// something unexpected.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// 401, body `{"error":"unauthorized"}`: the gateway has
    /// `TOKENFUSE_ADMIN_KEYS` configured and the key this client presented
    /// (or the absence of one) does not match. A key problem, not a network
    /// one - `admission::state`/`credentials::state`'s `connect` names it
    /// that way to the operator.
    #[error("the gateway refused the presented key (401 unauthorized)")]
    Unauthorized,

    /// 403, body `{"error":"admin_keys_required"}`: the gateway is bound off
    /// loopback with no `TOKENFUSE_ADMIN_KEYS` configured at all, so it
    /// refuses every request to the five admin routes regardless of what is
    /// presented.
    #[error("the gateway requires TOKENFUSE_ADMIN_KEYS and refuses every request (403)")]
    AdminKeysRequired,

    /// Any other non-2xx response: the status and raw body text (UTF-8 lossy).
    #[error("gateway returned HTTP {status}: {body}")]
    Api { status: u16, body: String },
}

// ---- DTOs (exact wire shape, docs/22-key-lifecycle.md in tokenfuse) --------

/// `GET /v1/keys`'s top-level shape.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GatewayKeysReport {
    /// `"off" | "warn" | "enforce"` - the identity-map enforcement mode
    /// (tokenfuse docs/20). Not a closed enum on the wire: an unrecognized
    /// value still deserializes, so a future mode never breaks this client;
    /// callers compare against the literal strings they care about.
    pub strict_mode: String,
    /// Whether this environment has an identity map configured at all.
    /// `false` means every `bound`/`unit` field below is vacuously empty -
    /// see `genaryx_api::credentials`'s frontend consumer for how that
    /// gates the "unbound" key-hygiene check.
    pub identity_map_configured: bool,
    /// Whether `keys[].history` is populated at all in this response (a
    /// gateway with no persisted call-history store still answers this
    /// report, just with `history: null` on every key).
    pub history_available: bool,
    pub unauthorized_since_startup: GatewayUnauthorized,
    pub keys: Vec<GatewayKeyEntry>,
}

/// Failed-auth attempts against the gateway since it started - no `key_id`
/// or caller identity attached (an unauthorized request never resolved to
/// one), just a count and the last time it happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct GatewayUnauthorized {
    pub attempts: u64,
    #[serde(default)]
    pub last_millis: Option<i64>,
}

/// One row of [`GatewayKeysReport::keys`]: a client key's configuration,
/// identity-map binding, and call activity. `configured`/`bound` are
/// independent booleans (a key can be either, both, or neither) - see
/// `apps/web/src/lib/credentials.ts::deriveKeyStatus` on the frontend
/// for the exact precedence that turns these four fields plus the two
/// [`GatewayKeyStats`] blocks into one human status.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct GatewayKeyEntry {
    pub key_id: String,
    /// Present in `TOKENFUSE_CLIENT_KEYS` right now.
    pub configured: bool,
    /// Matched by an `agents` pattern in the identity map right now
    /// (docs/20) - always `false` when `identity_map_configured` is
    /// `false`, never a fabricated match.
    pub bound: bool,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub agents: Vec<String>,
    /// `"YYYY-MM-DD"` when the onboard wizard stamped one (see
    /// `crate::onboard`'s identity-map fragment), `None` for an
    /// older/hand-written map entry.
    #[serde(default)]
    pub created: Option<String>,
    pub since_startup: GatewayKeyStats,
    /// `None` when `history_available` is `false` for this report, or this
    /// specific key has no persisted history yet (e.g. onboarded after the
    /// history store's retention window, or never called before this
    /// process start).
    #[serde(default)]
    pub history: Option<GatewayKeyStats>,
}

/// The shape `since_startup` and `history` share. One struct for both rather
/// than two near-identical ones: `since_startup`'s wire object simply omits
/// `first_seen_millis` (this process has no notion of when a key was FIRST
/// seen, only across its own lifetime so far), which `#[serde(default)]`
/// covers - "absent" and "present as null" both resolve to `None`, never a
/// parse failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct GatewayKeyStats {
    pub calls: u64,
    #[serde(default)]
    pub identity_mismatches: u64,
    /// `history`-only in practice (absent on `since_startup`'s wire shape,
    /// per the doc comment above).
    #[serde(default)]
    pub first_seen_millis: Option<i64>,
    #[serde(default)]
    pub last_seen_millis: Option<i64>,
}

// ---- response parsing -------------------------------------------------------

/// Parse one REST response: a 2xx body deserializes as `T`; a 401 becomes
/// [`GatewayError::Unauthorized`]; a 403 carrying `admin_keys_required`
/// becomes [`GatewayError::AdminKeysRequired`]; anything else non-2xx becomes
/// [`GatewayError::Api`] with the raw status/body (never a panic on an
/// unexpected status) - the two named variants are the two the
/// `adminkeys.rs` gate can actually return (see the module doc), every other
/// status falls through to the generic shape `idryx::parse_response` also
/// uses.
async fn parse_response<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T, GatewayError> {
    let status = resp.status();
    let bytes = resp.bytes().await?;
    if status.is_success() {
        return Ok(serde_json::from_slice(&bytes)?);
    }
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(GatewayError::Unauthorized);
    }
    if status == reqwest::StatusCode::FORBIDDEN
        && String::from_utf8_lossy(&bytes).contains("admin_keys_required")
    {
        return Err(GatewayError::AdminKeysRequired);
    }
    Err(GatewayError::Api {
        status: status.as_u16(),
        body: String::from_utf8_lossy(&bytes).into_owned(),
    })
}

// ---- client ------------------------------------------------------------

/// A typed client for the gateway's key-lifecycle read. No bearer by
/// default (see the module doc): one method, one request/response round
/// trip over `reqwest`, awaited directly - mirrors `IdryxClient`'s identical
/// shape for its own REST reads. An admin key, when attached with
/// [`with_admin_key`](Self::with_admin_key), rides along on every request as
/// `Authorization: Bearer <key>`.
pub struct GatewayClient {
    base_url: String,
    http: reqwest::Client,
    admin_key: Option<String>,
}

/// Manual impl so a key attached with [`GatewayClient::with_admin_key`]
/// never appears in a `{:?}` log line - `#[derive(Debug)]` would print the
/// `Option<String>` field verbatim, and this is a secret the moment it is
/// `Some`.
impl std::fmt::Debug for GatewayClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayClient")
            .field("base_url", &self.base_url)
            .field("admin_key", &self.admin_key.as_ref().map(|_| "<redacted>"))
            .finish_non_exhaustive()
    }
}

impl GatewayClient {
    /// Construct a client for `base_url` (e.g. `http://127.0.0.1:4100` - a
    /// trailing slash is trimmed), with no admin key attached. Returns
    /// `Result` because building the underlying HTTP client can fail (same
    /// rationale as `IdryxClient::new`).
    pub fn new(base_url: impl Into<String>) -> Result<Self, GatewayError> {
        let http = reqwest::Client::builder().build()?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
            admin_key: None,
        })
    }

    /// Attach (or clear, with `None`) the admin bearer key: every request
    /// after this call carries `Authorization: Bearer <key>` when `Some`,
    /// and no `Authorization` header at all when `None` - the same "absent
    /// means absent, never an empty header" rule the rest of this codebase's
    /// bearer clients keep.
    #[must_use]
    pub fn with_admin_key(mut self, admin_key: Option<String>) -> Self {
        self.admin_key = admin_key;
        self
    }

    /// `GET /v1/keys` -> the whole key-lifecycle report.
    pub async fn get_keys(&self) -> Result<GatewayKeysReport, GatewayError> {
        let url = format!("{}/v1/keys", self.base_url);
        let mut req = self.http.get(&url);
        if let Some(key) = &self.admin_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await?;
        parse_response(resp).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The exact shape docs/22-key-lifecycle.md describes, parsed offline (no
    // live gateway). A live read against a real gateway is a review-stage
    // check (README "Not built yet"), not a unit test.

    #[test]
    fn full_report_parses_every_field() {
        let json = br#"{
          "strict_mode": "enforce",
          "identity_map_configured": true,
          "history_available": true,
          "unauthorized_since_startup": { "attempts": 3, "last_millis": 1753200000000 },
          "keys": [
            {
              "key_id": "billing-agent", "configured": true, "bound": true,
              "unit": "finance", "agents": ["agent://acme.local/finance/billing-agent"],
              "created": "2026-06-01",
              "since_startup": { "calls": 42, "identity_mismatches": 0, "last_seen_millis": 1753199000000 },
              "history": { "calls": 900, "identity_mismatches": 1, "first_seen_millis": 1748000000000, "last_seen_millis": 1753199000000 }
            }
          ]
        }"#;
        let report: GatewayKeysReport = serde_json::from_slice(json).expect("parse report");
        assert_eq!(report.strict_mode, "enforce");
        assert!(report.identity_map_configured);
        assert!(report.history_available);
        assert_eq!(report.unauthorized_since_startup.attempts, 3);
        assert_eq!(
            report.unauthorized_since_startup.last_millis,
            Some(1753200000000)
        );
        assert_eq!(report.keys.len(), 1);
        let k = &report.keys[0];
        assert_eq!(k.key_id, "billing-agent");
        assert!(k.configured && k.bound);
        assert_eq!(k.unit.as_deref(), Some("finance"));
        assert_eq!(k.created.as_deref(), Some("2026-06-01"));
        assert_eq!(k.since_startup.calls, 42);
        assert_eq!(
            k.since_startup.first_seen_millis, None,
            "since_startup never carries first_seen_millis on the wire"
        );
        let h = k.history.as_ref().expect("history present");
        assert_eq!(h.calls, 900);
        assert_eq!(h.identity_mismatches, 1);
        assert_eq!(h.first_seen_millis, Some(1748000000000));
    }

    #[test]
    fn minimal_key_defaults_every_optional_field() {
        // No unit, no agents, no created, no history - a freshly-configured
        // key with nothing bound yet.
        let json = br#"{
          "strict_mode": "off",
          "identity_map_configured": false,
          "history_available": false,
          "unauthorized_since_startup": { "attempts": 0, "last_millis": null },
          "keys": [
            { "key_id": "onboard-fresh", "configured": true, "bound": false,
              "since_startup": { "calls": 0, "identity_mismatches": 0, "last_seen_millis": null } }
          ]
        }"#;
        let report: GatewayKeysReport = serde_json::from_slice(json).expect("parse report");
        let k = &report.keys[0];
        assert!(k.unit.is_none());
        assert!(k.agents.is_empty());
        assert!(k.created.is_none());
        assert!(k.history.is_none());
        assert_eq!(k.since_startup.calls, 0);
        assert_eq!(report.unauthorized_since_startup.last_millis, None);
    }

    #[test]
    fn unknown_extra_fields_are_tolerated() {
        // No deny_unknown_fields anywhere: a field this client does not know
        // about yet (the tokenfuse side is being built in parallel) must
        // never break parsing.
        let json = br#"{
          "strict_mode": "warn",
          "identity_map_configured": true,
          "history_available": true,
          "future_field": "ignored",
          "unauthorized_since_startup": { "attempts": 0, "last_millis": null, "future": 1 },
          "keys": [
            { "key_id": "k1", "configured": true, "bound": true, "extra_key_field": 7,
              "since_startup": { "calls": 1, "identity_mismatches": 0, "last_seen_millis": 1, "future": true } }
          ]
        }"#;
        let report: GatewayKeysReport =
            serde_json::from_slice(json).expect("tolerate unknown fields");
        assert_eq!(report.keys.len(), 1);
    }

    #[test]
    fn empty_keys_array_parses_not_an_error() {
        let json = br#"{
          "strict_mode": "off",
          "identity_map_configured": false,
          "history_available": false,
          "unauthorized_since_startup": { "attempts": 0, "last_millis": null },
          "keys": []
        }"#;
        let report: GatewayKeysReport = serde_json::from_slice(json).expect("parse report");
        assert!(report.keys.is_empty());
    }
    // ---- admin key (T2, PLAN-GATEWAY-ADMIN-KEY-2026-09-07.md) --------------
    //
    // A minimal raw HTTP/1.1 mock server: bind an ephemeral port, read one
    // request's headers off the socket, write back a fixed status/body.
    // Mirrors `uapi_tls_pinning.rs`'s own raw `TcpListener` + manual
    // read/write approach rather than adding a mock-HTTP-server dependency.

    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Spawn a one-shot mock server on an ephemeral port. Returns the port
    /// and a join handle yielding the raw request text it received (headers
    /// only - `GET /v1/keys` never sends a body).
    fn spawn_mock_server(
        status_line: &'static str,
        body: &'static str,
    ) -> (u16, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let port = listener.local_addr().expect("local_addr").port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept one connection");
            let mut buf = [0u8; 4096];
            let mut request = Vec::new();
            loop {
                let n = stream.read(&mut buf).expect("read the request");
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let response = format!(
                "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write the response");
            let _ = stream.flush();
            String::from_utf8_lossy(&request).into_owned()
        });
        (port, handle)
    }

    /// A minimal, valid `GET /v1/keys` 200 body - only used to prove the
    /// request reached the server with (or without) a bearer, never to
    /// exercise DTO parsing again.
    const MINIMAL_REPORT: &str = r#"{
        "strict_mode": "off",
        "identity_map_configured": false,
        "history_available": false,
        "unauthorized_since_startup": { "attempts": 0, "last_millis": null },
        "keys": []
    }"#;

    #[tokio::test]
    async fn a_configured_key_is_sent_as_a_bearer() {
        let (port, handle) = spawn_mock_server("HTTP/1.1 200 OK", MINIMAL_REPORT);
        let client = GatewayClient::new(format!("http://127.0.0.1:{port}"))
            .expect("build client")
            .with_admin_key(Some("sk-console-admin-key".to_string()));

        let result = client.get_keys().await;
        let request = handle.join().expect("server thread must not panic");

        assert!(result.is_ok(), "a keyed 200 must parse: {result:?}");
        assert!(
            request
                .to_lowercase()
                .contains("authorization: bearer sk-console-admin-key"),
            "request must carry the configured key as a bearer, got: {request}"
        );
    }

    #[tokio::test]
    async fn no_key_means_no_authorization_header() {
        let (port, handle) = spawn_mock_server("HTTP/1.1 200 OK", MINIMAL_REPORT);
        let client = GatewayClient::new(format!("http://127.0.0.1:{port}")).expect("build client");

        let result = client.get_keys().await;
        let request = handle.join().expect("server thread must not panic");

        assert!(
            result.is_ok(),
            "an unkeyed 200 must still parse: {result:?}"
        );
        assert!(
            !request.to_lowercase().contains("authorization"),
            "no admin key attached must mean no Authorization header at all, got: {request}"
        );
    }

    #[tokio::test]
    async fn a_401_is_classified_as_unauthorized_not_a_generic_api_error() {
        let (port, handle) =
            spawn_mock_server("HTTP/1.1 401 Unauthorized", r#"{"error":"unauthorized"}"#);
        let client = GatewayClient::new(format!("http://127.0.0.1:{port}"))
            .expect("build client")
            .with_admin_key(Some("sk-wrong-key".to_string()));

        let err = client.get_keys().await.expect_err("401 must be an error");
        let _request = handle.join().expect("server thread must not panic");

        assert!(
            matches!(err, GatewayError::Unauthorized),
            "expected GatewayError::Unauthorized, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn a_403_admin_keys_required_is_classified_distinctly_from_other_403s() {
        let (port, handle) = spawn_mock_server(
            "HTTP/1.1 403 Forbidden",
            r#"{"error":"admin_keys_required"}"#,
        );
        let client = GatewayClient::new(format!("http://127.0.0.1:{port}")).expect("build client");

        let err = client.get_keys().await.expect_err("403 must be an error");
        let _request = handle.join().expect("server thread must not panic");

        assert!(
            matches!(err, GatewayError::AdminKeysRequired),
            "expected GatewayError::AdminKeysRequired, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn an_unrelated_403_falls_through_to_the_generic_api_error() {
        // Guards against `AdminKeysRequired` swallowing every 403: only the
        // gateway's own body shape gets the named variant.
        let (port, handle) =
            spawn_mock_server("HTTP/1.1 403 Forbidden", r#"{"error":"something_else"}"#);
        let client = GatewayClient::new(format!("http://127.0.0.1:{port}")).expect("build client");

        let err = client.get_keys().await.expect_err("403 must be an error");
        let _request = handle.join().expect("server thread must not panic");

        assert!(
            matches!(err, GatewayError::Api { status: 403, .. }),
            "an unrelated 403 body must stay the generic Api error, got: {err:?}"
        );
    }

    #[test]
    fn the_key_never_appears_in_debug_output() {
        let client = GatewayClient::new("http://127.0.0.1:4100")
            .expect("build client")
            .with_admin_key(Some("sk-super-secret-do-not-print-me".to_string()));

        let debug = format!("{client:?}");
        assert!(
            !debug.contains("sk-super-secret-do-not-print-me"),
            "the admin key must never appear in Debug output, got: {debug}"
        );
        assert!(
            debug.contains("redacted"),
            "Debug output should say the key is present-but-redacted, got: {debug}"
        );
    }

    #[test]
    fn debug_output_says_none_when_no_key_is_attached() {
        let client = GatewayClient::new("http://127.0.0.1:4100").expect("build client");
        let debug = format!("{client:?}");
        assert!(
            debug.contains("None"),
            "no admin key attached must show as None, got: {debug}"
        );
    }
}
