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
//! ## No auth, and no secret ever appears here
//!
//! The gateway is loopback/perimeter-bound (same posture idryx's own module
//! doc argues for, `crates/connectors/src/idryx.rs`, though for a different
//! reason: this is TokenFuse's own request path, not an unauthenticated
//! snapshot service). This client sends no bearer and no signer - `/v1/keys`
//! is an operator-facing admin read, mirroring idryx's connector shape
//! exactly. And unlike a `TOKENFUSE_CLIENT_KEYS` entry (`<secret>:<key_id>`,
//! docs/ONBOARD.md), the report below carries only `key_id`, the non-secret
//! half - the secret itself never appears in this API and this client never
//! transmits one either.
//!
//! ## Fail-closed (06 §0.5) and forward-tolerant
//!
//! A transport failure becomes [`GatewayError::Transport`]; a non-2xx becomes
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

    /// Any non-2xx response: the status and raw body text (UTF-8 lossy).
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
/// `apps/desktop/src/lib/credentials.ts::deriveKeyStatus` on the frontend
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

/// Parse one REST response: a 2xx body deserializes as `T`; anything else
/// becomes [`GatewayError::Api`] with the raw status/body (never a panic on
/// an unexpected status) - identical shape to `idryx::parse_response`.
async fn parse_response<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T, GatewayError> {
    let status = resp.status();
    let bytes = resp.bytes().await?;
    if status.is_success() {
        Ok(serde_json::from_slice(&bytes)?)
    } else {
        Err(GatewayError::Api {
            status: status.as_u16(),
            body: String::from_utf8_lossy(&bytes).into_owned(),
        })
    }
}

// ---- client ------------------------------------------------------------

/// A typed client for the gateway's key-lifecycle read. Unauthenticated by
/// design (see the module doc): no bearer, no signer. One method, one
/// request/response round trip over `reqwest`, awaited directly - mirrors
/// `IdryxClient`'s identical shape for its own REST reads.
#[derive(Debug)]
pub struct GatewayClient {
    base_url: String,
    http: reqwest::Client,
}

impl GatewayClient {
    /// Construct a client for `base_url` (e.g. `http://127.0.0.1:4100` - a
    /// trailing slash is trimmed). Returns `Result` because building the
    /// underlying HTTP client can fail (same rationale as
    /// `IdryxClient::new`).
    pub fn new(base_url: impl Into<String>) -> Result<Self, GatewayError> {
        let http = reqwest::Client::builder().build()?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
        })
    }

    /// `GET /v1/keys` -> the whole key-lifecycle report.
    pub async fn get_keys(&self) -> Result<GatewayKeysReport, GatewayError> {
        let url = format!("{}/v1/keys", self.base_url);
        let resp = self.http.get(&url).send().await?;
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
}
