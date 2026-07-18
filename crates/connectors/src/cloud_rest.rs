//! `CloudClient`: a typed REST client for TokenFuse Cloud's money-plane API
//! (07 §4.2) - the data spine the Phase-1 Money panel renders from. Reads the
//! seven endpoints listed in `docs/PHASE1.md`'s scope line
//! (`summary`/`runs`/`agents`/`savings`/`incidents`/`alerts`/`audit/verify`)
//! and performs the three admin mutations (`kill`/`budget`/`ack`) as
//! ES256-signed, device-authenticated requests, reusing [`CloudSse`]'s sibling
//! signing crate (`genaryx-signing`) rather than reimplementing any of it.
//!
//! Every wire shape below was read directly from
//! `~/Development/tokenfuse/crates/cloud/src/{store,http,devices}.rs` (the
//! authority per this task's ground-truth instructions), not guessed; see the
//! per-struct doc comments for the exact source. The signed-mutation
//! header/canonical contract (`X-Fuse-Device/TS/Nonce/Sig` over
//! `METHOD\nPATH\nsha256(body)hex\nTS\nNONCE`) is already proven end to end
//! against a live Cloud by Phase-0 spike #2 (`crates/signing/examples/pair_ack.rs`);
//! this module is the same contract wrapped in a typed, reusable client instead
//! of a one-shot driver.
//!
//! ## Shape: plain `async fn`, not a background thread
//!
//! [`CloudSse`] (`crates/connectors/src/cloud_sse.rs`) bridges a long-lived
//! async stream to a synchronous `EventSource::poll()` contract, so it needs
//! its own dedicated thread and runtime. `CloudClient` has no such mismatch to
//! bridge: every method is one request/response round trip, so it is a plain
//! `async fn` over `reqwest`, awaited directly by whatever async context calls
//! it (a Tauri command, or the same async-to-sync bridge `crates/ffi` already
//! uses at the UniFFI boundary, docs/PHASE0.md F-04). This keeps the client
//! itself free of extra channel/thread machinery, and matches how
//! `crates/signing/examples/pair_ack.rs` already drives the identical wire
//! protocol (`#[tokio::main]` + direct `.await`s) - proof this shape is
//! sufficient for the real protocol, not just convenient here.
//!
//! Response bytes are parsed with `serde_json::from_slice` rather than
//! `reqwest`'s `json` feature (`Response::json`) so no new `reqwest` feature
//! flag is needed beyond the `stream` one `CloudSse` already activates; this
//! mirrors `pair_ack.rs`'s own `resp.text()` + manual `serde_json::from_str`
//! pattern.
//!
//! ## Fail-closed mutation contract (06 §0.5)
//!
//! [`CloudClient::kill_run`], [`CloudClient::set_budget`] and
//! [`CloudClient::ack_incident`] all require a device attached via
//! [`CloudClient::attach_device`] (typically right after [`CloudClient::pair`]).
//! With no device attached they return [`ConnectorError::NoDeviceSigner`]
//! *before* building or sending any request - never a silent unsigned POST.
//!
//! ## Typical flow
//!
//! ```no_run
//! # async fn go() -> Result<(), genaryx_connectors::ConnectorError> {
//! use genaryx_connectors::CloudClient;
//! use genaryx_signing::SoftwareSigner;
//!
//! let mut client = CloudClient::new("http://127.0.0.1:8080", "key:acme:admin")?;
//!
//! // One-time device pairing (needs a separate admin bearer able to mint
//! // codes; often the same key as above, but `pair` never assumes that).
//! let signer = SoftwareSigner::generate().map_err(genaryx_connectors::ConnectorError::from)?;
//! let paired = client.pair("key:acme:admin", &signer).await?;
//! client.attach_device(paired.device_id, paired.device_token, Box::new(signer));
//!
//! let summary = client.summary().await?;
//! println!("org has spent {} microusd", summary.spent_microusd);
//!
//! client.kill_run("runaway-run-1").await?;
//! # Ok(())
//! # }
//! ```

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use genaryx_signing::{Es256Signer, SigningError, sign_mutation};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

// ---- error -----------------------------------------------------------------

/// Every failure mode a [`CloudClient`] call can surface. Fail-closed
/// throughout (06 §0.5): no panics, no `unwrap`; a non-2xx response always
/// becomes one of these, never a silently-ignored or generic failure.
#[derive(Debug, thiserror::Error)]
pub enum ConnectorError {
    /// The request never got a response at all (DNS, connect, TLS, timeout,
    /// or a response body that failed to read).
    #[error("http transport: {0}")]
    Transport(#[from] reqwest::Error),

    /// A 2xx body that failed to deserialize into the expected shape - either
    /// this client's DTOs have drifted from the live Cloud, or the server sent
    /// something unexpected.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// Any non-2xx, non-{402,403-signature_invalid} response: the status and
    /// raw body text (UTF-8 lossy), for callers that want to inspect it
    /// themselves (e.g. a `404 unknown incident` from `ack_incident`).
    #[error("cloud returned HTTP {status}: {body}")]
    Api { status: u16, body: String },

    /// `402` with the Cloud's `plan_required` envelope
    /// (`{"error":{"type":"plan_required",feature,org,upgrade_url}}`,
    /// `http.rs::plan_required`) - a distinct, inspectable variant so the
    /// Money panel can render an upsell tile instead of a generic error. This
    /// is the ONLY 402 shape `tokenfuse-cloud`'s control-plane API ever
    /// returns; it is unrelated to the gateway's own runtime budget Breaker
    /// (a different service this client never talks to).
    #[error("plan required: feature={feature} org={org} (upgrade: {upgrade_url})")]
    PlanRequired {
        feature: String,
        org: String,
        upgrade_url: String,
    },

    /// A mutation was attempted with no device attached
    /// ([`CloudClient::attach_device`]). Returned before any request is
    /// built or sent - fail-closed, never an unsigned POST.
    #[error("mutation requires a paired device signer; call attach_device (or pair) first")]
    NoDeviceSigner,

    /// Signing the mutation itself failed (entropy, clock, or key export -
    /// see `genaryx_signing::SigningError`). Distinct from a rejection BY the
    /// server, which is [`ConnectorError::SignatureRejected`].
    #[error("signing failed: {0}")]
    Signing(#[from] SigningError),

    /// The server rejected a signed mutation with `403 {"error":"signature_invalid"}`
    /// (`devices.rs::verify_signature` / `http.rs::AuthError::SignatureInvalid`):
    /// bad signature, wrong device, replayed nonce, or stale timestamp. Kept
    /// distinct from a generic `403` (e.g. `"admin role required"` for a
    /// viewer-role device, which stays [`ConnectorError::Api`]) so a caller can
    /// tell "your signature didn't verify" apart from "you're not allowed to
    /// do this at all".
    #[error("device signature rejected (403 signature_invalid)")]
    SignatureRejected,
}

// ---- client ------------------------------------------------------------------

/// A signed-in device, attached via [`CloudClient::attach_device`], that
/// authorizes [`CloudClient::kill_run`] / [`CloudClient::set_budget`] /
/// [`CloudClient::ack_incident`]. Never `Debug`-printed with its secret intact
/// (see [`CloudClient`]'s manual `Debug` impl).
struct PairedDeviceHandle {
    device_id: String,
    device_token: String,
    signer: Box<dyn Es256Signer>,
}

/// A typed client for TokenFuse Cloud's control-plane REST API (`http.rs`).
/// Reads use `bearer_token` (an org API key, `key:org[:role][:plan]`, or a
/// device token); mutations additionally require a device attached via
/// [`CloudClient::attach_device`]. See the module docs for the full
/// pair-then-mutate flow and the fail-closed contract.
pub struct CloudClient {
    base_url: String,
    bearer_token: String,
    http: reqwest::Client,
    device: Option<PairedDeviceHandle>,
}

// Manual Debug: never print `bearer_token` or a device's `device_token`
// verbatim (06 §0.5 logging hygiene - the same rule `CloudSseConfig` follows
// in `cloud_sse.rs`). A stray `eprintln!("{client:?}")` must not leak either
// credential.
impl std::fmt::Debug for CloudClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudClient")
            .field("base_url", &self.base_url)
            .field("bearer_token", &"<redacted>")
            .field("device_id", &self.device.as_ref().map(|d| &d.device_id))
            .finish()
    }
}

impl CloudClient {
    /// Construct a client for `base_url` (e.g. `http://127.0.0.1:8080`, no
    /// trailing slash needed - one is trimmed if present) authenticating reads
    /// with `bearer_token`. No device is attached yet; mutation calls fail
    /// closed with [`ConnectorError::NoDeviceSigner`] until
    /// [`CloudClient::attach_device`] is called.
    ///
    /// Returns `Result` (rather than panicking, as `reqwest::Client::new()`
    /// effectively does internally) because building the underlying HTTP
    /// client can fail (e.g. TLS backend init) and library code must never
    /// unwrap past that (06 §0.5) - mirrors how `cloud_sse.rs::run_loop`
    /// handles the identical `reqwest::Client::builder().build()` call.
    pub fn new(
        base_url: impl Into<String>,
        bearer_token: impl Into<String>,
    ) -> Result<Self, ConnectorError> {
        let http = reqwest::Client::builder().build()?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            bearer_token: bearer_token.into(),
            http,
            device: None,
        })
    }

    /// Attach a paired device (fresh from [`CloudClient::pair`], or restored
    /// from a previously persisted `device_id`/`device_token`) so subsequent
    /// mutation calls sign with it. Replaces any previously attached device.
    pub fn attach_device(
        &mut self,
        device_id: impl Into<String>,
        device_token: impl Into<String>,
        signer: Box<dyn Es256Signer>,
    ) {
        self.device = Some(PairedDeviceHandle {
            device_id: device_id.into(),
            device_token: device_token.into(),
            signer,
        });
    }

    /// Whether a device is currently attached (mutation calls will attempt to
    /// sign rather than failing closed immediately).
    #[must_use]
    pub fn has_device(&self) -> bool {
        self.device.is_some()
    }

    // ---- reads (store.rs shapes, bearer-authenticated) --------------------

    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ConnectorError> {
        let resp = self
            .http
            .get(format!("{}{path}", self.base_url))
            .bearer_auth(&self.bearer_token)
            .send()
            .await?;
        parse_response(resp).await
    }

    /// `GET /v1/summary` - org-wide totals (`http.rs::summary`, `Summary` in
    /// `store.rs`).
    pub async fn summary(&self) -> Result<Summary, ConnectorError> {
        self.get_json("/v1/summary").await
    }

    /// `GET /v1/runs` - the caller org's aggregated runs (`http.rs::runs`,
    /// `RunAgg` in `store.rs`).
    pub async fn runs(&self) -> Result<Vec<RunAgg>, ConnectorError> {
        self.get_json("/v1/runs").await
    }

    /// `GET /v1/agents` - per-agent spend rollup, highest spend first
    /// (`http.rs::agents`, `AgentAgg` in `store.rs`).
    pub async fn agents(&self) -> Result<Vec<AgentAgg>, ConnectorError> {
        self.get_json("/v1/agents").await
    }

    /// `GET /v1/savings` - FinOps savings totals (`http.rs::savings`,
    /// `SavingsSummary` in `store.rs`).
    pub async fn savings(&self) -> Result<SavingsSummary, ConnectorError> {
        self.get_json("/v1/savings").await
    }

    /// `GET /v1/incidents` - open incidents, newest first (`http.rs::incidents`,
    /// `Incident` in `store.rs`).
    pub async fn incidents(&self) -> Result<Vec<Incident>, ConnectorError> {
        self.get_json("/v1/incidents").await
    }

    /// `GET /v1/alerts` - runs at or above the alert threshold of their
    /// central budget, at the server's default `alert_pct` (`http.rs::alerts`,
    /// `Alert` in `store.rs`).
    pub async fn alerts(&self) -> Result<Vec<Alert>, ConnectorError> {
        self.get_json("/v1/alerts").await
    }

    /// `GET /v1/audit/verify` - whether the caller org's tamper-evident audit
    /// chain verifies end-to-end right now (`http.rs::audit_verify`).
    pub async fn audit_verify(&self) -> Result<AuditVerifyResponse, ConnectorError> {
        self.get_json("/v1/audit/verify").await
    }

    /// `GET /v1/compliance/evidence` - the org's EU AI Act / SR 11-7 / SOC 2
    /// control-coverage report (`compliance.rs::ComplianceReport`), for the
    /// Evidence Center (docs/PHASE4.md W3). Kept as raw JSON: the pack captures
    /// the bytes verbatim and the panel renders a summary; the console does not
    /// re-model the full control catalog here.
    pub async fn compliance_evidence(&self) -> Result<serde_json::Value, ConnectorError> {
        self.get_json("/v1/compliance/evidence").await
    }

    /// Sign an Evidence-Center manifest with the attached device's ES256 key,
    /// producing a self-describing [`genaryx_core::evidence::SignatureBlock`]
    /// the pack embeds as `manifest.sig.json` (docs/PHASE4.md W3). This is the
    /// SAME console device key the money mutations sign with - the operator's
    /// identity attesting "I assembled this pack". Fails closed with
    /// [`ConnectorError::NoDeviceSigner`] when no device is attached, so the
    /// caller builds an honestly-UNSIGNED pack rather than one that claims to be
    /// signed. A genuine signing failure (not "no device") propagates as
    /// [`ConnectorError::Signing`], never a silent unsigned pack.
    pub fn sign_evidence_manifest(
        &self,
        manifest_bytes: &[u8],
    ) -> Result<genaryx_core::evidence::SignatureBlock, ConnectorError> {
        let device = self.device.as_ref().ok_or(ConnectorError::NoDeviceSigner)?;
        let signature = device.signer.sign_raw(manifest_bytes)?;
        Ok(genaryx_core::evidence::SignatureBlock {
            alg: "ES256".to_string(),
            signature_b64: B64.encode(signature),
            public_key_b64: device.signer.public_key_b64()?,
            over: "manifest.json".to_string(),
        })
    }

    // ---- pairing (devices.rs) ----------------------------------------------

    /// Mint a one-time pairing code: `POST /v1/pair/new` with `admin_bearer`
    /// (an admin org key - `http.rs::admin_org_key` requires role `admin`).
    /// Deliberately separate from [`CloudClient::pair`] (Phase 5 W2,
    /// itrat-console/13 D12.2a step 1): the desktop's Pocket panel mints a
    /// code for a PHONE to redeem later (over the relay, W3), so it must stop
    /// after this step rather than immediately generating its own signer and
    /// redeeming the code itself the way `pair`'s single-shot flow does.
    pub async fn pair_new(&self, admin_bearer: &str) -> Result<PairNewResponse, ConnectorError> {
        let resp = self
            .http
            .post(format!("{}/v1/pair/new", self.base_url))
            .bearer_auth(admin_bearer)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body("{}")
            .send()
            .await?;
        parse_response(resp).await
    }

    /// Pair a new device: mints a code via [`CloudClient::pair_new`], then
    /// `POST /v1/pair` redeeming it with `signer.public_key_b64()` (no bearer
    /// on this second call - the code itself is the credential, exactly as
    /// `devices.rs`'s module doc and `http.rs::pair`'s doc specify). This is
    /// the desktop's OWN device self-pairing flow (Money/Overview, Phase 1);
    /// the Pocket panel's phone-pairing flow (Phase 5 W2) uses `pair_new`
    /// directly instead, since the code there is redeemed by the PHONE, over
    /// the relay, not by this client.
    ///
    /// Returns the full [`PairResponse`]; the caller decides whether (and
    /// when) to hand its `device_id`/`device_token` to
    /// [`CloudClient::attach_device`] alongside `signer` - `pair` itself never
    /// mutates `self`, so a caller that wants to persist pairing state before
    /// wiring up mutations is free to do so.
    pub async fn pair(
        &self,
        admin_bearer: &str,
        signer: &dyn Es256Signer,
    ) -> Result<PairResponse, ConnectorError> {
        let issued = self.pair_new(admin_bearer).await?;

        let pubkey_b64 = signer.public_key_b64()?;
        let redeem_body = serde_json::to_vec(&PairRequestBody {
            code: issued.code,
            pubkey_b64,
            platform: std::env::consts::OS.to_string(),
            name: format!("genaryx-connectors ({})", signer.assurance().label()),
        })?;
        let resp = self
            .http
            .post(format!("{}/v1/pair", self.base_url))
            // Deliberately no `.bearer_auth(...)` here: `http.rs::pair` takes
            // no `HeaderMap` at all - the one-time code is the credential.
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(redeem_body)
            .send()
            .await?;
        parse_response(resp).await
    }

    // ---- signed mutations (fail-closed, devices.rs contract) ---------------

    /// Sign and POST one mutation: builds the ES256 signature over
    /// `METHOD\nPATH\nsha256(body)hex\nTS\nNONCE` via
    /// `genaryx_signing::sign_mutation` (never reimplemented here), attaches
    /// `X-Fuse-Device/TS/Nonce/Sig` plus the device token as the bearer, and
    /// sends exactly the `body` bytes that were hashed into the signature -
    /// re-serializing between signing and sending would desync the two and
    /// make every signature invalid.
    ///
    /// Fails closed with [`ConnectorError::NoDeviceSigner`] before building
    /// or sending anything when no device is attached.
    async fn signed_post<T: DeserializeOwned>(
        &self,
        path: String,
        body: Vec<u8>,
    ) -> Result<T, ConnectorError> {
        let device = self.device.as_ref().ok_or(ConnectorError::NoDeviceSigner)?;
        let m = sign_mutation(device.signer.as_ref(), "POST", &path, &body)?;
        let resp = self
            .http
            .post(format!("{}{path}", self.base_url))
            .bearer_auth(&device.device_token)
            .header("X-Fuse-Device", device.device_id.as_str())
            .header("X-Fuse-TS", m.ts.as_str())
            .header("X-Fuse-Nonce", m.nonce.as_str())
            .header("X-Fuse-Sig", m.sig_b64.as_str())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await?;
        parse_response(resp).await
    }

    /// `POST /v1/runs/{run_id}/kill` (admin, ES256 device-signed, empty body)
    /// - `http.rs::kill`. Gateways poll `/v1/kills` and hard-stop the run.
    pub async fn kill_run(&self, run_id: &str) -> Result<KillResponse, ConnectorError> {
        self.signed_post(format!("/v1/runs/{run_id}/kill"), Vec::new())
            .await
    }

    /// `POST /v1/runs/{run_id}/budget` (admin, ES256 device-signed, body
    /// `{"budget_usd": f64}`) - `http.rs::set_budget`. The server stores
    /// `(budget_usd * 1e6) as i64` micros; `BudgetResponse::budget_micros`
    /// reflects the value actually stored.
    pub async fn set_budget(
        &self,
        run_id: &str,
        budget_usd: f64,
    ) -> Result<BudgetResponse, ConnectorError> {
        let body = serde_json::to_vec(&BudgetBody { budget_usd })?;
        self.signed_post(format!("/v1/runs/{run_id}/budget"), body)
            .await
    }

    /// `POST /v1/incidents/{incident_id}/ack` (admin, ES256 device-signed,
    /// empty body) - `http.rs::ack_incident`. `404` (surfaced as
    /// [`ConnectorError::Api`] with `status: 404`) for an unknown incident id.
    pub async fn ack_incident(&self, incident_id: &str) -> Result<AckResponse, ConnectorError> {
        self.signed_post(format!("/v1/incidents/{incident_id}/ack"), Vec::new())
            .await
    }
}

/// Parse one HTTP response: a 2xx body deserializes as `T`; anything else
/// becomes a classified [`ConnectorError`] (never a panic on an unexpected
/// status or an unparseable error body - [`classify_error`] falls back to the
/// raw status/body when the shape doesn't match what it expected).
async fn parse_response<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T, ConnectorError> {
    let status = resp.status();
    let bytes = resp.bytes().await?;
    if status.is_success() {
        Ok(serde_json::from_slice(&bytes)?)
    } else {
        Err(classify_error(status, &bytes))
    }
}

/// Turn a non-2xx status + raw body into the most specific [`ConnectorError`]
/// that applies: `402` -> [`ConnectorError::PlanRequired`] (`http.rs::plan_required`'s
/// envelope); `403` whose body is exactly `{"error":"signature_invalid"}` ->
/// [`ConnectorError::SignatureRejected`] (`http.rs::AuthError::SignatureInvalid`,
/// as opposed to `403 {"error":"admin role required"}`, which stays a generic
/// [`ConnectorError::Api`]); everything else, or a body that doesn't parse as
/// the envelope its status implies, falls back to [`ConnectorError::Api`]
/// with the raw status/body - a malformed or unexpected error body must never
/// itself cause a panic.
fn classify_error(status: reqwest::StatusCode, bytes: &[u8]) -> ConnectorError {
    if status.as_u16() == 402
        && let Ok(envelope) = serde_json::from_slice::<PlanRequiredResponse>(bytes)
    {
        return ConnectorError::PlanRequired {
            feature: envelope.error.feature,
            org: envelope.error.org,
            upgrade_url: envelope.error.upgrade_url,
        };
    }
    if status.as_u16() == 403
        && let Ok(e) = serde_json::from_slice::<ErrorResponse>(bytes)
        && e.error == "signature_invalid"
    {
        return ConnectorError::SignatureRejected;
    }
    ConnectorError::Api {
        status: status.as_u16(),
        body: String::from_utf8_lossy(bytes).into_owned(),
    }
}

// ---- read DTOs (exact shapes from store.rs) --------------------------------

/// `GET /v1/summary` body. Exact shape of `store.rs::Summary`: `runs`/`calls`
/// are exact across the org's whole ingest history; `spent_microusd` is real
/// spend only (blocked/avoided-spend rows excluded).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Summary {
    pub runs: u64,
    pub calls: u64,
    pub spent_microusd: i64,
}

/// One element of `GET /v1/runs`. Exact shape of `store.rs::RunAgg` -
/// `last_seen_millis` on the wire (the Rust field there is `last_seen`, `#[serde(rename)]`'d;
/// named `last_seen_millis` here directly since only the wire name matters to a reader).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunAgg {
    pub run_id: String,
    pub model: String,
    pub agent_id: String,
    pub spent_microusd: i64,
    pub calls: u64,
    pub cache_hits: u64,
    pub steps: u32,
    pub last_seen_millis: i64,
    pub killed: bool,
}

/// One element of `GET /v1/agents`. Exact shape of `store.rs::AgentAgg`; the
/// empty-string `agent_id` is the "unattributed" bucket.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentAgg {
    pub agent_id: String,
    pub spent_microusd: i64,
    pub calls: u64,
    pub runs: u64,
    pub last_seen_millis: i64,
}

/// `GET /v1/savings` body. Exact shape of `store.rs::SavingsSummary` - the
/// FinOps headline number is `total_saved_microusd` (blocked + cache + router
/// savings, summed server-side).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavingsSummary {
    pub blocked_spend_microusd: i64,
    pub cache_saved_microusd: i64,
    pub router_saved_microusd: i64,
    pub budget_breaks: u64,
    pub total_saved_microusd: i64,
}

/// One element of `GET /v1/alerts`. Exact shape of `store.rs::Alert`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub run_id: String,
    pub spent_microusd: i64,
    pub budget_micros: i64,
    pub fraction: f64,
    pub killed: bool,
}

/// `tokenfuse_core::Severity`, mirrored here rather than pulled in as a
/// dependency (this crate has no other reason to depend on `tokenfuse-core`,
/// and the wire form - lowercase string - is a two-line mirror). Confirmed
/// against `tokenfuse/crates/core/src/mcpreport.rs`: `#[serde(rename_all =
/// "lowercase")]` over exactly these five variants, low-to-high.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

/// One element of `GET /v1/incidents`. Exact shape of `store.rs::Incident`.
/// `run_id`/`agent_id` are always present as a JSON key (possibly `null`) on
/// the wire, never omitted, so no `#[serde(default)]` is needed on either.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incident {
    pub id: String,
    pub org: String,
    pub run_id: Option<String>,
    pub agent_id: Option<String>,
    pub kind: String,
    pub severity: Severity,
    pub first_seen_millis: i64,
    pub last_seen_millis: i64,
    pub occurrences: u64,
    pub acknowledged: bool,
    pub last_notified_millis: i64,
}

/// `GET /v1/audit/verify` body. Exact shape of `http.rs::AuditVerifyResponse`.
/// `break_index` needs `#[serde(default)]`: the server's own
/// `#[serde(skip_serializing_if = "Option::is_none")]` OMITS the key entirely
/// when `ok` is `true`, rather than sending `null`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditVerifyResponse {
    pub ok: bool,
    #[serde(default)]
    pub break_index: Option<usize>,
}

// ---- pairing DTOs (devices.rs / http.rs) -----------------------------------

/// `POST /v1/pair/new` response. Exact shape of `http.rs::PairNewResponse`
/// (`devices.rs`/`http.rs:470-473`): `code` is the redeemable one-time code
/// (an 8-char unambiguous-alphabet string, `devices::pairing_code()`),
/// `expires_unix` is when it stops being redeemable (currently a fixed 600s
/// from mint, `http.rs::pair_new`). Public since Phase 5 W2: the Pocket
/// panel's [`CloudClient::pair_new`] hands this straight back to its caller
/// (the desktop needs `code` for the QR and `expires_unix` to know how long
/// the relay's pairing window should stay armed) rather than consuming it
/// internally the way [`CloudClient::pair`] does.
#[derive(Debug, Clone, Deserialize)]
pub struct PairNewResponse {
    pub code: String,
    pub expires_unix: i64,
}

/// `POST /v1/pair` request body. Exact shape of `http.rs::PairRequest`.
#[derive(Debug, Serialize)]
struct PairRequestBody {
    code: String,
    pubkey_b64: String,
    platform: String,
    name: String,
}

/// `POST /v1/pair` response: the paired device's identity and its read/mutate
/// bearer token. Exact shape of `http.rs::PairResponse`.
#[derive(Clone, Deserialize)]
pub struct PairResponse {
    pub device_id: String,
    pub org: String,
    pub role: String,
    pub device_token: String,
}

// Manual Debug: `device_token` is a bearer secret (same rule as `CloudClient`'s
// own redaction, and `CloudSseConfig`'s in `cloud_sse.rs`) - never printed verbatim.
impl std::fmt::Debug for PairResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairResponse")
            .field("device_id", &self.device_id)
            .field("org", &self.org)
            .field("role", &self.role)
            .field("device_token", &"<redacted>")
            .finish()
    }
}

// ---- mutation DTOs (http.rs) ------------------------------------------------

/// `POST /v1/runs/{run}/budget` request body. Exact shape of `http.rs::BudgetBody`.
#[derive(Debug, Serialize)]
struct BudgetBody {
    budget_usd: f64,
}

/// `POST /v1/runs/{run}/kill` response. Exact shape of `http.rs::KillResponse`.
#[derive(Debug, Clone, Deserialize)]
pub struct KillResponse {
    pub killed: String,
}

/// `POST /v1/runs/{run}/budget` response. Exact shape of `http.rs::BudgetResponse`.
#[derive(Debug, Clone, Deserialize)]
pub struct BudgetResponse {
    pub run: String,
    pub budget_micros: i64,
}

/// `POST /v1/incidents/{id}/ack` response. Exact shape of `http.rs::AckResponse`.
#[derive(Debug, Clone, Deserialize)]
pub struct AckResponse {
    pub acknowledged: String,
}

// ---- error-body DTOs (http.rs) ----------------------------------------------

/// The flat `{"error": "..."}` envelope every plain error response uses
/// (`http.rs::ErrorResponse` / the `error()` helper) - 401/403/404/400 all
/// take this shape (402 is the one exception; see [`PlanRequiredResponse`]).
#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: String,
}

/// The nested `402 plan_required` envelope (`http.rs::PlanRequiredResponse` /
/// `plan_required()`). `kind` is always the literal `"plan_required"` (the
/// JSON key is `type`, `#[serde(rename)]`'d server-side); kept only to model
/// the shape faithfully, not read by [`classify_error`].
#[derive(Debug, Deserialize)]
struct PlanRequiredResponse {
    error: PlanRequiredError,
}

#[derive(Debug, Deserialize)]
struct PlanRequiredError {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    kind: String,
    feature: String,
    org: String,
    upgrade_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- DTO deserialization: real shapes, not guessed ---------------------
    // Each literal below is a real response the Cloud would send, transcribed
    // field-for-field from store.rs / http.rs (see the per-struct doc
    // comments above for the exact source line), so these tests fail loudly
    // if this module's structs ever drift from the real wire contract.

    #[test]
    fn summary_deserializes() {
        let s: Summary = serde_json::from_str(r#"{"runs":3,"calls":42,"spent_microusd":150000}"#)
            .expect("valid Summary json");
        assert_eq!(s.runs, 3);
        assert_eq!(s.calls, 42);
        assert_eq!(s.spent_microusd, 150_000);
    }

    #[test]
    fn run_agg_deserializes_with_last_seen_millis_rename() {
        let runs: Vec<RunAgg> = serde_json::from_str(
            r#"[{"run_id":"r1","model":"gpt-4o","agent_id":"","spent_microusd":9000,
                 "calls":5,"cache_hits":1,"steps":3,"last_seen_millis":1758000000000,
                 "killed":false}]"#,
        )
        .expect("valid RunAgg json");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "r1");
        assert_eq!(runs[0].last_seen_millis, 1_758_000_000_000);
        assert!(!runs[0].killed);
    }

    #[test]
    fn agent_agg_deserializes() {
        let agents: Vec<AgentAgg> = serde_json::from_str(
            r#"[{"agent_id":"planner","spent_microusd":5000,"calls":2,"runs":1,
                 "last_seen_millis":1758000000000}]"#,
        )
        .expect("valid AgentAgg json");
        assert_eq!(agents[0].agent_id, "planner");
    }

    #[test]
    fn savings_summary_deserializes() {
        let s: SavingsSummary = serde_json::from_str(
            r#"{"blocked_spend_microusd":1000,"cache_saved_microusd":2000,
                 "router_saved_microusd":500,"budget_breaks":1,
                 "total_saved_microusd":3500}"#,
        )
        .expect("valid SavingsSummary json");
        assert_eq!(s.total_saved_microusd, 3500);
    }

    #[test]
    fn alert_deserializes() {
        let alerts: Vec<Alert> = serde_json::from_str(
            r#"[{"run_id":"r1","spent_microusd":8000,"budget_micros":10000,
                 "fraction":0.8,"killed":false}]"#,
        )
        .expect("valid Alert json");
        assert!((alerts[0].fraction - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn incident_deserializes_with_lowercase_severity_and_null_ids() {
        let incidents: Vec<Incident> = serde_json::from_str(
            r#"[{"id":"spend_spike:org","org":"acme","run_id":null,"agent_id":null,
                 "kind":"spend_spike","severity":"high","first_seen_millis":1,
                 "last_seen_millis":2,"occurrences":3,"acknowledged":false,
                 "last_notified_millis":0}]"#,
        )
        .expect("valid Incident json");
        assert_eq!(incidents[0].severity, Severity::High);
        assert_eq!(incidents[0].run_id, None);
    }

    #[test]
    fn audit_verify_ok_true_omits_break_index() {
        // `ok:true` responses omit `break_index` entirely (server-side
        // `skip_serializing_if`), not send `null` - this must still parse.
        let v: AuditVerifyResponse =
            serde_json::from_str(r#"{"ok":true}"#).expect("valid AuditVerifyResponse json");
        assert!(v.ok);
        assert_eq!(v.break_index, None);
    }

    #[test]
    fn audit_verify_broken_chain_carries_break_index() {
        let v: AuditVerifyResponse = serde_json::from_str(r#"{"ok":false,"break_index":4}"#)
            .expect("valid AuditVerifyResponse json");
        assert!(!v.ok);
        assert_eq!(v.break_index, Some(4));
    }

    #[test]
    fn kill_budget_ack_responses_deserialize() {
        let k: KillResponse =
            serde_json::from_str(r#"{"killed":"r1"}"#).expect("valid KillResponse");
        assert_eq!(k.killed, "r1");
        let b: BudgetResponse = serde_json::from_str(r#"{"run":"r1","budget_micros":12500000}"#)
            .expect("valid BudgetResponse");
        assert_eq!(b.budget_micros, 12_500_000);
        let a: AckResponse =
            serde_json::from_str(r#"{"acknowledged":"inc1"}"#).expect("valid AckResponse");
        assert_eq!(a.acknowledged, "inc1");
    }

    #[test]
    fn pair_new_response_deserializes_code_and_expiry() {
        let p: PairNewResponse =
            serde_json::from_str(r#"{"code":"ABCD1234","expires_unix":1758000600}"#)
                .expect("valid PairNewResponse");
        assert_eq!(p.code, "ABCD1234");
        assert_eq!(p.expires_unix, 1_758_000_600);
    }

    #[test]
    fn pair_response_deserializes_and_redacts_token_in_debug() {
        let p: PairResponse = serde_json::from_str(
            r#"{"device_id":"d1","org":"acme","role":"admin","device_token":"super-secret"}"#,
        )
        .expect("valid PairResponse");
        assert_eq!(p.device_id, "d1");
        let printed = format!("{p:?}");
        assert!(!printed.contains("super-secret"));
        assert!(printed.contains("<redacted>"));
    }

    // ---- error classification -----------------------------------------------

    #[test]
    fn classifies_402_plan_required_envelope() {
        let body = br#"{"error":{"type":"plan_required","feature":"fleet_reads","org":"acme","upgrade_url":"https://tokenfuse.dev/pricing"}}"#;
        let err = classify_error(reqwest::StatusCode::PAYMENT_REQUIRED, body);
        match err {
            ConnectorError::PlanRequired {
                feature,
                org,
                upgrade_url,
            } => {
                assert_eq!(feature, "fleet_reads");
                assert_eq!(org, "acme");
                assert_eq!(upgrade_url, "https://tokenfuse.dev/pricing");
            }
            other => panic!("expected PlanRequired, got {other:?}"),
        }
    }

    #[test]
    fn classifies_403_signature_invalid_distinctly_from_403_forbidden() {
        let sig_invalid = classify_error(
            reqwest::StatusCode::FORBIDDEN,
            br#"{"error":"signature_invalid"}"#,
        );
        assert!(matches!(sig_invalid, ConnectorError::SignatureRejected));

        // A DIFFERENT 403 body (`http.rs::forbidden`, non-admin role) must
        // NOT be misclassified as a rejected signature.
        let admin_required = classify_error(
            reqwest::StatusCode::FORBIDDEN,
            br#"{"error":"admin role required"}"#,
        );
        match admin_required {
            ConnectorError::Api { status, body } => {
                assert_eq!(status, 403);
                assert!(body.contains("admin role required"));
            }
            other => panic!("expected a generic Api error, got {other:?}"),
        }
    }

    #[test]
    fn classifies_unrecognized_status_and_garbage_body_without_panicking() {
        let err = classify_error(
            reqwest::StatusCode::NOT_FOUND,
            br#"{"error":"unknown run"}"#,
        );
        assert!(matches!(err, ConnectorError::Api { status: 404, .. }));

        // Even a non-JSON body must never panic - it just falls back to the
        // raw text.
        let err = classify_error(reqwest::StatusCode::BAD_GATEWAY, b"<html>502</html>");
        match err {
            ConnectorError::Api { status, body } => {
                assert_eq!(status, 502);
                assert!(body.contains("502"));
            }
            other => panic!("expected a generic Api error, got {other:?}"),
        }
    }

    // ---- fail-closed: no device signer -> explicit error, no network -------

    #[tokio::test]
    async fn mutations_without_a_device_fail_closed_with_no_network_call() {
        // Points at a port nothing listens on; if any of these calls tried to
        // actually send a request, `Transport` (a connection error) would win
        // the race against `NoDeviceSigner`, not the other way around - so
        // asserting `NoDeviceSigner` here also proves the guard runs BEFORE
        // any I/O, not just that it eventually surfaces one.
        let client = CloudClient::new("http://127.0.0.1:1", "key:acme:admin")
            .expect("client construction never touches the network");
        assert!(!client.has_device());

        let err = client.kill_run("r1").await.unwrap_err();
        assert!(matches!(err, ConnectorError::NoDeviceSigner));

        let err = client.set_budget("r1", 10.0).await.unwrap_err();
        assert!(matches!(err, ConnectorError::NoDeviceSigner));

        let err = client.ack_incident("inc1").await.unwrap_err();
        assert!(matches!(err, ConnectorError::NoDeviceSigner));
    }

    #[test]
    fn new_trims_a_trailing_slash_from_base_url() {
        let client = CloudClient::new("http://127.0.0.1:8080/", "devkey").expect("client");
        assert_eq!(client.base_url, "http://127.0.0.1:8080");
    }
}
