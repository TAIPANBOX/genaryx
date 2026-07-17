//! `HetznerClient`: a STRICTLY READ-ONLY Hetzner Cloud inventory connector
//! (docs/PHASE4.md W4, decision D11 §"Hetzner is read-only, v1"). It lists the
//! taipan-tagged servers a live-validation campaign is running on - their
//! status, IP, type, and price/hour - so a campaign's boxes are visible in the
//! console. It NEVER creates, resizes, or deletes a Hetzner resource: there is
//! no POST/PUT/DELETE method on this type at all, by construction, so the
//! console (and I) cannot mutate or tear down infrastructure through it - the
//! teardown of any box is Yurii's, never the console's ([[never-delete-keys-on-own-initiative]]).
//!
//! Talks to the public Hetzner Cloud API (`https://api.hetzner.cloud/v1`) with a
//! read-scoped API token as a `Bearer`. The token is the client's only
//! credential; it is never logged (manual `Debug` redaction).
//!
//! Fail-closed (06 §0.5): a transport failure is [`HetznerError::Transport`], a
//! non-2xx is [`HetznerError::Api`] with the status/body, an undeserializable
//! 2xx is [`HetznerError::Json`]. No panics. Price parsing is best-effort: a
//! server whose per-location price cannot be found reports `price_hourly_eur:
//! None` rather than failing the whole listing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The public Hetzner Cloud API base.
const DEFAULT_BASE: &str = "https://api.hetzner.cloud/v1";

// ---- error -----------------------------------------------------------------

/// Every failure mode a [`HetznerClient`] call can surface. Fail-closed.
#[derive(Debug, thiserror::Error)]
pub enum HetznerError {
    /// Building the HTTP client failed (e.g. TLS backend init).
    #[error("http client build: {0}")]
    Build(String),

    /// The request never got a response (DNS, connect, TLS, timeout).
    #[error("http transport: {0}")]
    Transport(#[from] reqwest::Error),

    /// A non-2xx response: the status and raw body (UTF-8 lossy). A `401`/`403`
    /// here means the API token is missing/invalid/over-scoped.
    #[error("hetzner returned HTTP {status}: {body}")]
    Api { status: u16, body: String },

    /// A 2xx body that failed to deserialize into the expected shape.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

// ---- public inventory row --------------------------------------------------

/// One server in the inventory, flattened from the Hetzner API's nested shape
/// to what the Remote panel shows.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct HetznerServer {
    pub id: i64,
    pub name: String,
    /// `running` | `off` | `starting` | `stopping` | `initializing` |
    /// `migrating` | `rebuilding` | `deleting` | `unknown` (Hetzner's
    /// `server.status`).
    pub status: String,
    /// The primary public IPv4, or `None` if the server has none attached.
    pub ipv4: Option<String>,
    /// The server type name, e.g. `cpx62`.
    pub server_type: String,
    pub cores: i64,
    /// RAM in GB (Hetzner reports `memory` as GB, a float).
    pub memory_gb: f64,
    /// The datacenter location, e.g. `nbg1`.
    pub location: String,
    /// Net hourly price in EUR for this server's location, best-effort (`None`
    /// if the per-location price row could not be found).
    pub price_hourly_eur: Option<f64>,
    /// The server's labels (e.g. `managed-by=taipan`).
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// ISO-8601 creation time.
    #[serde(default)]
    pub created: String,
}

// ---- wire DTOs (Hetzner Cloud API GET /v1/servers) -------------------------

#[derive(Debug, Deserialize)]
struct ServersEnvelope {
    #[serde(default)]
    servers: Vec<WireServer>,
}

#[derive(Debug, Deserialize)]
struct WireServer {
    id: i64,
    name: String,
    status: String,
    #[serde(default)]
    public_net: WirePublicNet,
    server_type: WireServerType,
    #[serde(default)]
    datacenter: WireDatacenter,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    #[serde(default)]
    created: String,
}

#[derive(Debug, Default, Deserialize)]
struct WirePublicNet {
    #[serde(default)]
    ipv4: Option<WireIpv4>,
}

#[derive(Debug, Deserialize)]
struct WireIpv4 {
    #[serde(default)]
    ip: String,
}

#[derive(Debug, Deserialize)]
struct WireServerType {
    #[serde(default)]
    name: String,
    #[serde(default)]
    cores: i64,
    #[serde(default)]
    memory: f64,
    #[serde(default)]
    prices: Vec<WirePrice>,
}

#[derive(Debug, Deserialize)]
struct WirePrice {
    #[serde(default)]
    location: String,
    #[serde(default)]
    price_hourly: WirePriceValue,
}

#[derive(Debug, Default, Deserialize)]
struct WirePriceValue {
    /// Hetzner returns prices as decimal STRINGS (e.g. `"0.0230000000"`).
    #[serde(default)]
    net: String,
}

#[derive(Debug, Default, Deserialize)]
struct WireDatacenter {
    #[serde(default)]
    location: WireLocation,
}

#[derive(Debug, Default, Deserialize)]
struct WireLocation {
    #[serde(default)]
    name: String,
}

impl From<WireServer> for HetznerServer {
    fn from(w: WireServer) -> Self {
        let location = w.datacenter.location.name;
        // Best-effort: the price row whose location matches the server's.
        let price_hourly_eur = w
            .server_type
            .prices
            .iter()
            .find(|p| p.location == location)
            .and_then(|p| p.price_hourly.net.parse::<f64>().ok());
        let ipv4 = w.public_net.ipv4.map(|v| v.ip).filter(|ip| !ip.is_empty());
        Self {
            id: w.id,
            name: w.name,
            status: w.status,
            ipv4,
            server_type: w.server_type.name,
            cores: w.server_type.cores,
            memory_gb: w.server_type.memory,
            location,
            price_hourly_eur,
            labels: w.labels,
            created: w.created,
        }
    }
}

// ---- client ----------------------------------------------------------------

/// A read-only Hetzner Cloud inventory client. Holds a read-scoped API token;
/// every method is a single `GET`. There is deliberately NO mutation method.
pub struct HetznerClient {
    base_url: String,
    token: String,
    http: reqwest::Client,
}

// Manual Debug: never print the API token (06 §0.5 logging hygiene, same rule
// `CloudClient`/`CloudSseConfig` follow).
impl std::fmt::Debug for HetznerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HetznerClient")
            .field("base_url", &self.base_url)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl HetznerClient {
    /// Construct a client for the public Hetzner Cloud API, authenticating with
    /// `token` (a READ-scoped API token; this connector never needs write
    /// scope).
    pub fn new(token: impl Into<String>) -> Result<Self, HetznerError> {
        Self::with_base(DEFAULT_BASE, token)
    }

    /// Construct a client against an explicit `base_url` (for tests, pointed at
    /// a mock server). Otherwise identical to [`Self::new`].
    pub fn with_base(
        base_url: impl Into<String>,
        token: impl Into<String>,
    ) -> Result<Self, HetznerError> {
        let http = reqwest::Client::builder()
            .build()
            .map_err(|e| HetznerError::Build(e.to_string()))?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
            http,
        })
    }

    /// `GET /v1/servers[?label_selector=<sel>]` -> the inventory. Pass a
    /// `label_selector` (e.g. `"managed-by=taipan"`) to scope the listing to a
    /// campaign's tagged boxes, or `None` for every server the token can see.
    pub async fn list_servers(
        &self,
        label_selector: Option<&str>,
    ) -> Result<Vec<HetznerServer>, HetznerError> {
        let mut url = format!("{}/servers", self.base_url);
        if let Some(sel) = label_selector {
            url.push_str("?label_selector=");
            url.push_str(&urlencode(sel));
        }
        let resp = self.http.get(&url).bearer_auth(&self.token).send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            return Err(HetznerError::Api {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).into_owned(),
            });
        }
        let env: ServersEnvelope = serde_json::from_slice(&bytes)?;
        Ok(env.servers.into_iter().map(HetznerServer::from).collect())
    }
}

/// Minimal percent-encoding for a `label_selector` query value (the selector
/// grammar uses `=`, `,`, `!`, and spaces). Avoids pulling a URL-encoding crate
/// for one query parameter.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // The exact GET /v1/servers shape (trimmed to the fields this connector
    // reads), parsed offline. A live test against a real mock sshd/hetzner is
    // out of scope (external paid API); this pins the flatten + price parse.
    const SERVERS: &[u8] = br#"{
      "servers": [
        {
          "id": 42, "name": "taipan-live-1", "status": "running",
          "public_net": { "ipv4": { "ip": "203.0.113.7" } },
          "server_type": { "name": "cpx62", "cores": 16, "memory": 32.0,
            "prices": [
              { "location": "fsn1", "price_hourly": { "net": "0.0490000000" } },
              { "location": "nbg1", "price_hourly": { "net": "0.0500000000" } }
            ] },
          "datacenter": { "location": { "name": "nbg1" } },
          "labels": { "managed-by": "taipan" },
          "created": "2026-07-17T10:00:00+00:00"
        },
        {
          "id": 43, "name": "no-ip-box", "status": "off",
          "public_net": {},
          "server_type": { "name": "cx11", "cores": 1, "memory": 2.0, "prices": [] },
          "datacenter": { "location": { "name": "hel1" } }
        }
      ]
    }"#;

    #[test]
    fn servers_flatten_with_location_matched_price_and_optional_ip() {
        let env: ServersEnvelope = serde_json::from_slice(SERVERS).expect("parse");
        let out: Vec<HetznerServer> = env.servers.into_iter().map(HetznerServer::from).collect();
        assert_eq!(out.len(), 2);

        let a = &out[0];
        assert_eq!(a.id, 42);
        assert_eq!(a.status, "running");
        assert_eq!(a.ipv4.as_deref(), Some("203.0.113.7"));
        assert_eq!(a.server_type, "cpx62");
        assert_eq!(a.location, "nbg1");
        // Price is the row matching the server's location (nbg1: 0.05), not fsn1.
        assert_eq!(a.price_hourly_eur, Some(0.05));
        assert_eq!(a.labels.get("managed-by"), Some(&"taipan".to_string()));

        // A server with no public IPv4 and no matching price row -> both None,
        // never a fabricated value or a parse failure.
        let b = &out[1];
        assert!(b.ipv4.is_none());
        assert!(b.price_hourly_eur.is_none());
        assert_eq!(b.status, "off");
    }

    #[test]
    fn empty_servers_list_parses() {
        let env: ServersEnvelope = serde_json::from_slice(br#"{"servers":[]}"#).expect("parse");
        assert!(env.servers.is_empty());
    }

    #[test]
    fn debug_redacts_the_api_token() {
        let c = HetznerClient::new("secret-token-value").expect("build");
        let dbg = format!("{c:?}");
        assert!(
            !dbg.contains("secret-token-value"),
            "token must not leak in Debug"
        );
        assert!(dbg.contains("<redacted>"));
    }

    #[test]
    fn label_selector_is_percent_encoded() {
        assert_eq!(urlencode("managed-by=taipan"), "managed-by%3Dtaipan");
        assert_eq!(urlencode("a,b"), "a%2Cb");
    }
}
