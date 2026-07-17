//! `IdryxClient`: a typed client for Idryx's identity/access-graph API
//! (07 §4.4) - the identity plane the console's Identity panel and Agent 360
//! render from. Two transports, both grounded in the idryx Go source
//! (`~/Development/idryx`, read 2026-07-17):
//!
//! 1. **REST snapshot** over `idryx serve`: `GET /api/identities`,
//!    `GET /api/alerts`, `GET /api/remediations`, `GET /healthz`. Every route
//!    and JSON shape lives in `internal/server/server.go` (there is NO
//!    `internal/api` package - the old note's path was right, the package name
//!    was wrong; grep-confirmed `server.go` is the only file registering
//!    routes). See each DTO's doc comment for its exact `server.go:line`.
//! 2. **CLI batch** ([`IdryxClient::rescan`]): `idryx detect --format json`,
//!    whose output is a byte-identical `[]jsonAlert` to `/api/alerts`
//!    (`internal/report/report.go:48-72`), so one [`Alert`] DTO covers both.
//!
//! ## No auth, and a snapshot that never reloads
//!
//! Idryx `serve` has **no authentication of any kind** (`SECURITY.md:121-123`;
//! the handlers discard the `*http.Request` entirely, `server.go:78,207,242`).
//! So this client carries no bearer, no signer - unlike [`crate::CloudClient`]
//! (ES256 device pairing) and [`crate::WardryxClient`] (bearer). Two
//! consequences the panels MUST respect:
//!
//! - `serve` is **load-once**: `runServe` runs `buildGraph -> runDetectors ->
//!   server.New` exactly once at startup, then serves an immutable snapshot
//!   forever (`server.go:16-17` says so verbatim; no file-watch / SIGHUP /
//!   reload route / poll / TTL exists, grep-verified). Polling `/api/*` returns
//!   byte-identical data for the process lifetime. The live delegation graph is
//!   therefore genaryx-core's job (built from the bus), NOT this snapshot; the
//!   UI labels this data "as of load" and offers [`IdryxClient::rescan`] to
//!   recompute the detectors.
//! - `--addr` defaults to `:8080` (all interfaces), not loopback; the startup
//!   log's `localhost` is cosmetic (`main.go:534-537`). `taipan up` remaps it to
//!   `127.0.0.1:8081`. A non-loopback idryx URL is a real posture signal.
//!
//! ## Fail-closed (06 §0.5)
//!
//! No panics, no `unwrap`/`expect`. Every non-2xx becomes [`IdryxError::Api`]
//! with the raw status/body; a 2xx body that will not deserialize becomes
//! [`IdryxError::Json`]; a `detect` spawn/exit failure becomes
//! [`IdryxError::Cli`]. `attestation` is NOT a field on the identity object
//! (deliberately, `server.go`) - it reaches a client only as free text inside
//! an `attestation_missing` alert's `summary`, so the panel derives attestation
//! status from that alert, never from [`Identity`].

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

// ---- error -----------------------------------------------------------------

/// Every failure mode an [`IdryxClient`] call can surface. Fail-closed
/// throughout: a non-2xx REST response, an undeserializable body, or a failed
/// `detect` spawn each become a specific variant, never a panic or a
/// silently-ignored failure.
#[derive(Debug, thiserror::Error)]
pub enum IdryxError {
    /// The request never got a response (DNS, connect, timeout, or a body that
    /// failed to read).
    #[error("http transport: {0}")]
    Transport(#[from] reqwest::Error),

    /// A 2xx REST body (or `detect --format json` stdout) that failed to
    /// deserialize into the expected shape - this client's DTOs have drifted
    /// from the live idryx, or idryx sent something unexpected.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// Any non-2xx REST response: the status and raw body text (UTF-8 lossy).
    /// Idryx's JSON handlers ignore the request and always answer 200 with an
    /// array, so in practice this only surfaces from `/` (the dashboard path,
    /// 404 on any non-root path) or a transport-adjacent gateway error.
    #[error("idryx returned HTTP {status}: {body}")]
    Api { status: u16, body: String },

    /// The `idryx detect` batch ([`IdryxClient::rescan`]) failed to spawn or
    /// exited nonzero. Note idryx's exit code does NOT signal findings (it
    /// exits 0 whether or not there are alerts; only a real error - bad flags,
    /// a parse failure - exits 1, `cmd/idryx/main.go:36-41`), so a nonzero exit
    /// here is a genuine failure, carrying idryx's own stderr.
    #[error("idryx detect: {0}")]
    Cli(String),
}

// ---- DTOs (exact wire shapes, idryx internal/server/server.go) --------------

/// One identity from `GET /api/identities` (`apiIdentity`,
/// `internal/server/server.go:119-134`). Serializes as a bare array element
/// (the endpoint returns `[]`, never `null`).
///
/// `events` and `alerts` are integer COUNTS (`len(id.Events)` /
/// matched-alert count, `server.go:200-201`), NOT the objects; the alert
/// objects come only from [`IdryxClient::list_alerts`]. The permission ARN is
/// deliberately not exposed. `attestation` is not present here at all (see the
/// module doc).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Identity {
    pub id: String,
    /// `human | service_account | key | agent | mcp_server`. An empty
    /// `model.IdentityType` is defaulted to the literal `"human"`
    /// (`server.go:163-166`).
    #[serde(rename = "type")]
    pub identity_type: String,
    pub privileged: bool,
    /// The connector/source name, e.g. `aws_iam`, `gcp_iam`, `agents`, `mcp`,
    /// `okta`, `tokenfuse`, `wardryx` (`server.go:191`).
    pub source: String,
    pub owner: String,
    /// `"YYYY-MM-DD HH:MM:SS UTC"` when non-zero, else absent (note: a
    /// different format from [`Alert::time`]). `server.go:154-161,193-194`.
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub last_used: String,
    #[serde(default)]
    pub runtime: String,
    /// The delegation chain, root-first, max depth 32 (agent-passport SPEC §5).
    #[serde(default)]
    pub on_behalf_of: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    /// A right-sizing suggestion, present only when idryx generated one
    /// (`kind == "right_size"`). `server.go:168-176`.
    #[serde(default)]
    pub remediation: Option<Remediation>,
    /// A rotation suggestion, present only when idryx generated one
    /// (`kind == "rotation"`). `server.go:177-185`.
    #[serde(default)]
    pub rotation: Option<Remediation>,
    /// COUNT of this identity's events, not the objects (`server.go:200`).
    pub events: u64,
    /// COUNT of alerts on this identity, not the objects (`server.go:201`).
    pub alerts: u64,
}

/// One permission on an [`Identity`] (`apiPermission`, `server.go:82-86`). The
/// underlying `model.Permission.ARN` is intentionally not serialized.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Permission {
    pub name: String,
    pub admin: bool,
    pub used: bool,
}

/// A remediation or rotation suggestion (`apiRemediation`, `server.go:88-93`,
/// reused for both the `remediation` and `rotation` fields of an [`Identity`]
/// and as the body of a [`Recommendation`]).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Remediation {
    /// `"right_size"` or `"rotation"`.
    pub kind: String,
    pub explanation: String,
    pub code: String,
    #[serde(default)]
    pub created_at: String,
}

/// One detector alert from `GET /api/alerts` (`apiAlert`, `server.go:50-56`)
/// AND from `idryx detect --format json` (`jsonAlert`, byte-identical,
/// `internal/report/report.go:48-56`). Server-sorted severity-desc then
/// time-asc.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Alert {
    /// One of the 21 detector ids (e.g. `attestation_missing`, `runaway_agent`,
    /// `impossible_travel`; full list in `cmd/idryx/main.go:314-336`).
    pub detector: String,
    /// The identity id this alert is about (joins to [`Identity::id`]).
    pub identity: String,
    /// `critical | high | medium | low | info | none`. Dynamic per detector (a
    /// base escalated by privileged/admin/chain-length), so filter on
    /// `detector` AND `severity`, never a hard-coded per-detector severity.
    pub severity: String,
    /// `"YYYY-MM-DDTHH:MM:SSZ"` (UTC, no fractional; `server.go:65`).
    pub time: String,
    /// Free text. For `attestation_missing` this embeds `attestation=<value>` -
    /// the only place attestation status reaches a client.
    pub summary: String,
}

/// One row from `GET /api/remediations` (`apiRecommendation`,
/// `server.go:111-117`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Recommendation {
    pub identity: String,
    pub kind: String,
    pub explanation: String,
    pub code: String,
    #[serde(default)]
    pub created_at: String,
}

// ---- response parsing ------------------------------------------------------

/// Parse one REST response: a 2xx body deserializes as `T`; anything else
/// becomes [`IdryxError::Api`] with the raw status/body (never a panic on an
/// unexpected status).
async fn parse_response<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T, IdryxError> {
    let status = resp.status();
    let bytes = resp.bytes().await?;
    if status.is_success() {
        Ok(serde_json::from_slice(&bytes)?)
    } else {
        Err(IdryxError::Api {
            status: status.as_u16(),
            body: String::from_utf8_lossy(&bytes).into_owned(),
        })
    }
}

// ---- client ----------------------------------------------------------------

/// A typed client for Idryx's read-only identity API (`internal/server`).
/// Unauthenticated by design (see the module doc): no bearer, no signer, no
/// paired-device state. Every method is one request/response round trip over
/// `reqwest`, awaited directly (mirroring [`crate::CloudClient`]'s reads).
#[derive(Debug)]
pub struct IdryxClient {
    base_url: String,
    http: reqwest::Client,
}

impl IdryxClient {
    /// Construct a client for `base_url` (e.g. `http://127.0.0.1:8081` - a
    /// trailing slash is trimmed). Returns `Result` because building the
    /// underlying HTTP client can fail (same rationale as
    /// [`crate::CloudClient::new`]).
    pub fn new(base_url: impl Into<String>) -> Result<Self, IdryxError> {
        let http = reqwest::Client::builder().build()?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
        })
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, IdryxError> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.http.get(&url).send().await?;
        parse_response(resp).await
    }

    /// `GET /api/identities` -> every identity in the loaded snapshot.
    pub async fn list_identities(&self) -> Result<Vec<Identity>, IdryxError> {
        self.get("/api/identities").await
    }

    /// `GET /api/alerts` -> every detector alert in the loaded snapshot,
    /// server-sorted severity-desc then time-asc.
    pub async fn list_alerts(&self) -> Result<Vec<Alert>, IdryxError> {
        self.get("/api/alerts").await
    }

    /// `GET /api/remediations` -> every right-size/rotation suggestion.
    pub async fn list_remediations(&self) -> Result<Vec<Recommendation>, IdryxError> {
        self.get("/api/remediations").await
    }

    /// `GET /healthz` -> `true` on HTTP 200 (body is the literal `ok`, NOT
    /// JSON, so this checks the status, not a parse; `server.go:42-45`).
    pub async fn healthz(&self) -> Result<bool, IdryxError> {
        let url = format!("{}/healthz", self.base_url);
        let resp = self.http.get(&url).send().await?;
        Ok(resp.status().is_success())
    }

    /// Recompute the 21 detectors by shelling out to
    /// `idryx detect --format json --min-severity <sev> --load <src:path>...`
    /// and parsing the resulting `[]`[`Alert`] (byte-identical to
    /// `/api/alerts`). This is the **Rescan** path: `serve` is load-once, so a
    /// fresh `detect` over the current bus files is how the console picks up new
    /// findings without restarting idryx.
    ///
    /// `idryx_bin` is the resolved idryx binary path (the caller supplies it
    /// from the taipan descriptor or a located checkout - descriptor-based
    /// discovery is the env layer's job, not the connector's). `loads` are
    /// `(source, path)` pairs (e.g. `("tokenfuse", "/…/tokenfuse.ndjson")`);
    /// each `--load` carries its own source, so no `--source` flag is needed.
    /// `min_severity` is one of `low|medium|high|critical`.
    ///
    /// Synchronous (a batch job the caller runs off the UI thread): idryx's
    /// exit code does not signal findings, so a nonzero exit is a real failure
    /// and carries idryx's stderr as [`IdryxError::Cli`].
    pub fn rescan(
        idryx_bin: &std::path::Path,
        loads: &[(&str, &str)],
        min_severity: &str,
    ) -> Result<Vec<Alert>, IdryxError> {
        let mut cmd = std::process::Command::new(idryx_bin);
        cmd.arg("detect")
            .arg("--format")
            .arg("json")
            .arg("--min-severity")
            .arg(min_severity);
        for (src, path) in loads {
            cmd.arg("--load").arg(format!("{src}:{path}"));
        }
        let out = cmd
            .output()
            .map_err(|e| IdryxError::Cli(format!("spawn {}: {e}", idryx_bin.display())))?;
        if !out.status.success() {
            return Err(IdryxError::Cli(format!(
                "`idryx detect` exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(serde_json::from_slice(&out.stdout)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The exact shapes idryx's server.go emits, parsed offline (no live idryx).
    // A live snapshot + Rescan against a real `taipan up --with idryx` lives in
    // tests/idryx_test.rs, skip-gracefully when idryx is absent.

    #[test]
    fn identities_parse_full_and_minimal() {
        // A privileged agent with a delegation chain, permissions, and a
        // rotation suggestion; plus a minimal identity exercising every
        // `omitempty`/`default` field being absent.
        let json = br#"[
          {
            "id":"agent://acme.local/support/tier1","type":"agent","privileged":true,
            "source":"agents","owner":"platform","created":"2026-01-02 03:04:05 UTC",
            "runtime":"python","on_behalf_of":["user://acme.local/alice","agent://acme.local/orchestrator"],
            "permissions":[{"name":"charge","admin":false,"used":true},{"name":"iam:PassRole","admin":true,"used":false}],
            "rotation":{"kind":"rotation","explanation":"key is 400 days old","code":"resource \"x\" {}"},
            "events":42,"alerts":3
          },
          {"id":"svc://acme.local/reporter","type":"","privileged":false,"source":"aws_iam","owner":"","events":0,"alerts":0}
        ]"#;
        let ids: Vec<Identity> = serde_json::from_slice(json).expect("parse identities");
        assert_eq!(ids.len(), 2);

        let a = &ids[0];
        assert_eq!(a.identity_type, "agent");
        assert!(a.privileged);
        assert_eq!(a.on_behalf_of.len(), 2);
        assert_eq!(a.permissions.len(), 2);
        assert!(a.permissions[1].admin && !a.permissions[1].used);
        assert_eq!(
            a.rotation.as_ref().map(|r| r.kind.as_str()),
            Some("rotation")
        );
        assert!(a.remediation.is_none());
        assert_eq!(a.events, 42);
        assert_eq!(a.alerts, 3);

        // Minimal: absent optionals default cleanly, `type:""` stays "" on the
        // wire (the Go server defaults empty->"human" before serializing, so a
        // real payload never carries ""; we just prove absence does not panic).
        let b = &ids[1];
        assert_eq!(b.identity_type, "");
        assert!(b.on_behalf_of.is_empty());
        assert!(b.permissions.is_empty());
        assert!(b.remediation.is_none() && b.rotation.is_none());
        assert_eq!(b.created, "");
    }

    #[test]
    fn alerts_parse_same_shape_as_detect() {
        // This shape is emitted identically by GET /api/alerts and by
        // `idryx detect --format json`, so it is what rescan() parses too.
        let json = br#"[
          {"detector":"attestation_missing","identity":"agent://acme.local/support/tier1","severity":"high","time":"2026-01-02T03:04:05Z","summary":"privileged agent, attestation=none"},
          {"detector":"runaway_agent","identity":"agent://acme.local/batch","severity":"critical","time":"2026-01-02T03:05:00Z","summary":"budget_exhausted within 30d"}
        ]"#;
        let alerts: Vec<Alert> = serde_json::from_slice(json).expect("parse alerts");
        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[0].detector, "attestation_missing");
        assert!(alerts[0].summary.contains("attestation=none"));
        assert_eq!(alerts[1].severity, "critical");
    }

    #[test]
    fn remediations_parse() {
        let json = br#"[{"identity":"svc://acme.local/reporter","kind":"right_size","explanation":"3 of 9 permissions unused","code":"resource \"aws_iam_policy\" {}","created_at":"2026-01-02 03:04:05 UTC"}]"#;
        let recs: Vec<Recommendation> = serde_json::from_slice(json).expect("parse remediations");
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].kind, "right_size");
    }

    #[test]
    fn empty_arrays_parse_not_null() {
        // idryx pre-allocates with make([]T, 0, ...), so an empty result is
        // `[]`, never `null` - all three endpoints deserialize to an empty Vec.
        assert!(
            serde_json::from_slice::<Vec<Identity>>(b"[]")
                .unwrap()
                .is_empty()
        );
        assert!(
            serde_json::from_slice::<Vec<Alert>>(b"[]")
                .unwrap()
                .is_empty()
        );
        assert!(
            serde_json::from_slice::<Vec<Recommendation>>(b"[]")
                .unwrap()
                .is_empty()
        );
    }
}
