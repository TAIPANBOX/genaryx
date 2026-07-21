//! Relay configuration: loaded from a TOML file, overlaid with environment
//! variables, then validated fail-closed (docs/PHASE5.md W1: "config" module).
//!
//! Secrets (`cloud_viewer_key`) are never logged: [`RelayConfig`] has a manual
//! `Debug` impl that redacts it, mirroring
//! `genaryx_connectors::CloudSseConfig`'s own redaction (`cloud_sse.rs`).

use serde::Deserialize;
use std::net::SocketAddr;
use std::path::PathBuf;

/// Everything the relay needs to start, once loaded and validated.
#[derive(Clone)]
pub struct RelayConfig {
    /// The org this relay serves (single-tenant per relay instance).
    pub org: String,
    /// TokenFuse Cloud base URL, reached over loopback/LAN/WG
    /// (itrat-console/13 D12.1: "colocated with it over loopback, same as
    /// the production loopback path"), e.g. `http://127.0.0.1:8080`.
    pub cloud_base_url: String,
    /// The relay's own Cloud key: `role=viewer` (D12.1's trust-boundary
    /// table: the relay "Cannot mutate anything at the Cloud" precisely
    /// because this key is a viewer, never an admin, key). Used for the SSE
    /// subscription and the reconcile reads; never forwarded to the phone
    /// and never used to sign or authorize a mutation.
    pub cloud_viewer_key: String,
    /// Public TLS listener bind address (the phone's only channel).
    pub public_bind_addr: SocketAddr,
    /// Admin API bind address. MUST be loopback (validated in
    /// [`RelayConfig::validate`]): "never the public interface"
    /// (docs/PHASE5.md "admin" module). WG-only binding is a later PR (the
    /// interface exists once the relay's own WG peer is configured, same as
    /// desktop's D11 tunnel); loopback is the always-available floor.
    pub admin_bind_addr: SocketAddr,
    /// What the relay tells a pairing phone its own base URL is
    /// (`plane_url` in the pairing response, D12.2 step 8). Distinct from
    /// `public_bind_addr`, which may be a wildcard (`0.0.0.0`) unreachable
    /// as a literal advertise target.
    pub public_advertise_url: String,
    /// Directory holding `cert.pem`/`key.pem` for the public listener
    /// (generated on first run if absent, see `tls.rs`).
    pub tls_cert_dir: PathBuf,
    /// SQLite file for the single-device registry (`registry.rs`).
    pub db_path: PathBuf,
    /// Budget fraction at which a run counts as an exception (mirrors
    /// `tokenfuse-cloud`'s `alert_pct`, default 0.8, `http.rs:113`).
    pub alert_pct: f64,
    /// Push dedup window in seconds (mirrors `push.rs::DEDUP_SECS = 600`).
    pub dedup_secs: i64,
    /// Present only in the paid PUBLIC-CA trust mode (cert broker, design A,
    /// itrat-console/14): the relay obtains a publicly-trusted certificate for
    /// its own `<relay-id>.pocket.it-rat.com` hostname via ACME DNS-01 through
    /// the broker, and the phone connects to that hostname with ordinary system
    /// trust (no SPKI pin, no ATS exception). Absent = the free/local self-
    /// signed + SPKI-pin mode (`tls.rs`), unchanged.
    pub acme: Option<AcmeSettings>,
}

/// The all-or-nothing settings that switch a relay into PUBLIC-CA trust mode.
/// The certificate and account keys never leave the relay; the broker holds
/// the DNS-zone credential (see `acme.rs`). `broker_token` is a secret and is
/// redacted from every `Debug`.
#[derive(Clone)]
pub struct AcmeSettings {
    /// The CA's ACME directory URL (Let's Encrypt in production; a Pebble
    /// server in the proof).
    pub directory_url: String,
    /// The single hostname this relay certifies, `<relay-id>.pocket.it-rat.com`.
    pub hostname: String,
    /// Registration/recovery contact for the ACME account; may be empty.
    pub contact_email: String,
    /// Base URL of the Pocket cert broker (`/present` + `/cleanup`).
    pub broker_url: String,
    /// The relay's broker identity (HTTP Basic user = relay id).
    pub broker_user: String,
    /// The relay's broker token (HTTP Basic password). Never logged.
    pub broker_token: String,
    /// Optional path to an extra CA certificate (PEM) to trust for the ACME
    /// endpoint's TLS. Empty/absent in production (Let's Encrypt chains to a
    /// system-trusted root already); set it to run against a private ACME/CA
    /// (a Pebble test server, or an enterprise's own CA).
    pub ca_cert_path: Option<String>,
}

impl std::fmt::Debug for AcmeSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcmeSettings")
            .field("directory_url", &self.directory_url)
            .field("hostname", &self.hostname)
            .field("contact_email", &self.contact_email)
            .field("broker_url", &self.broker_url)
            .field("broker_user", &self.broker_user)
            .field("broker_token", &"<redacted>")
            .finish()
    }
}

// Manual Debug: never print `cloud_viewer_key` verbatim (06 §0.5 logging
// hygiene, the same rule `CloudSseConfig`/`CloudClient` already follow).
impl std::fmt::Debug for RelayConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayConfig")
            .field("org", &self.org)
            .field("cloud_base_url", &self.cloud_base_url)
            .field("cloud_viewer_key", &"<redacted>")
            .field("public_bind_addr", &self.public_bind_addr)
            .field("admin_bind_addr", &self.admin_bind_addr)
            .field("public_advertise_url", &self.public_advertise_url)
            .field("tls_cert_dir", &self.tls_cert_dir)
            .field("db_path", &self.db_path)
            .field("alert_pct", &self.alert_pct)
            .field("dedup_secs", &self.dedup_secs)
            .field("acme", &self.acme)
            .finish()
    }
}

/// A failure loading or validating [`RelayConfig`]. Every variant is a
/// refusal-to-start reason, never a silent default (06 §0.5 fail-closed):
/// `main` prints this and exits rather than serving with a guessed config.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("reading config file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing config file {path} as TOML: {source}")]
    Toml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("invalid config: {0}")]
    Invalid(String),
}

/// The raw TOML shape: every field optional, since env vars or built-in
/// defaults may supply it instead. Field names match the TOML keys.
#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    org: Option<String>,
    cloud_base_url: Option<String>,
    cloud_viewer_key: Option<String>,
    public_bind_addr: Option<String>,
    admin_bind_addr: Option<String>,
    public_advertise_url: Option<String>,
    tls_cert_dir: Option<String>,
    db_path: Option<String>,
    alert_pct: Option<f64>,
    dedup_secs: Option<i64>,
    // PUBLIC-CA trust mode (cert broker, design A). All-or-nothing: see
    // `RelayConfig::acme_from_raw`.
    acme_directory_url: Option<String>,
    acme_hostname: Option<String>,
    acme_contact_email: Option<String>,
    broker_url: Option<String>,
    broker_user: Option<String>,
    broker_token: Option<String>,
    acme_ca_cert: Option<String>,
}

impl RawConfig {
    /// Environment variables win over the TOML file, so a secret (the viewer
    /// key) can be injected at process-launch time without ever touching
    /// disk. Prefix: `GENARYX_RELAY_`.
    fn overlay_env(mut self) -> Self {
        if let Ok(v) = std::env::var("GENARYX_RELAY_ORG") {
            self.org = Some(v);
        }
        if let Ok(v) = std::env::var("GENARYX_RELAY_CLOUD_BASE_URL") {
            self.cloud_base_url = Some(v);
        }
        if let Ok(v) = std::env::var("GENARYX_RELAY_CLOUD_VIEWER_KEY") {
            self.cloud_viewer_key = Some(v);
        }
        if let Ok(v) = std::env::var("GENARYX_RELAY_PUBLIC_BIND_ADDR") {
            self.public_bind_addr = Some(v);
        }
        if let Ok(v) = std::env::var("GENARYX_RELAY_ADMIN_BIND_ADDR") {
            self.admin_bind_addr = Some(v);
        }
        if let Ok(v) = std::env::var("GENARYX_RELAY_PUBLIC_ADVERTISE_URL") {
            self.public_advertise_url = Some(v);
        }
        if let Ok(v) = std::env::var("GENARYX_RELAY_TLS_CERT_DIR") {
            self.tls_cert_dir = Some(v);
        }
        if let Ok(v) = std::env::var("GENARYX_RELAY_DB_PATH") {
            self.db_path = Some(v);
        }
        if let Ok(v) = std::env::var("GENARYX_RELAY_ALERT_PCT")
            && let Ok(p) = v.parse()
        {
            self.alert_pct = Some(p);
        }
        if let Ok(v) = std::env::var("GENARYX_RELAY_DEDUP_SECS")
            && let Ok(p) = v.parse()
        {
            self.dedup_secs = Some(p);
        }
        if let Ok(v) = std::env::var("GENARYX_RELAY_ACME_DIRECTORY_URL") {
            self.acme_directory_url = Some(v);
        }
        if let Ok(v) = std::env::var("GENARYX_RELAY_ACME_HOSTNAME") {
            self.acme_hostname = Some(v);
        }
        if let Ok(v) = std::env::var("GENARYX_RELAY_ACME_CONTACT_EMAIL") {
            self.acme_contact_email = Some(v);
        }
        if let Ok(v) = std::env::var("GENARYX_RELAY_BROKER_URL") {
            self.broker_url = Some(v);
        }
        if let Ok(v) = std::env::var("GENARYX_RELAY_BROKER_USER") {
            self.broker_user = Some(v);
        }
        if let Ok(v) = std::env::var("GENARYX_RELAY_BROKER_TOKEN") {
            self.broker_token = Some(v);
        }
        if let Ok(v) = std::env::var("GENARYX_RELAY_ACME_CA_CERT") {
            self.acme_ca_cert = Some(v);
        }
        self
    }
}

impl RelayConfig {
    /// Default config file path: `GENARYX_RELAY_CONFIG` if set, else
    /// `./relay.toml` (relative to the process's working directory, matching
    /// how the sim harness launches the relay next to a local Cloud).
    fn config_path() -> PathBuf {
        std::env::var("GENARYX_RELAY_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("relay.toml"))
    }

    /// Load from the TOML file (if present; a missing file is not an error
    /// here, since env vars or defaults may cover every required field),
    /// overlay environment variables, fill defaults, and validate. This is
    /// the ONLY entry point `main` calls; every other constructor below is a
    /// building block for it (and for tests).
    pub fn load() -> Result<Self, ConfigError> {
        let path = Self::config_path();
        let raw = if path.exists() {
            let text = std::fs::read_to_string(&path).map_err(|source| ConfigError::Io {
                path: path.clone(),
                source,
            })?;
            toml::from_str::<RawConfig>(&text)
                .map_err(|source| ConfigError::Toml { path, source })?
        } else {
            RawConfig::default()
        };
        Self::from_raw(raw.overlay_env())
    }

    fn from_raw(raw: RawConfig) -> Result<Self, ConfigError> {
        // Resolve PUBLIC-CA settings first: `acme_from_raw` only borrows `raw`,
        // and must run before the `require(...)` calls below move fields out.
        let acme = Self::acme_from_raw(&raw)?;
        let org = require(raw.org, "org")?;
        let cloud_base_url = require(raw.cloud_base_url, "cloud_base_url")?
            .trim_end_matches('/')
            .to_string();
        let cloud_viewer_key = require(raw.cloud_viewer_key, "cloud_viewer_key")?;

        let public_bind_addr = parse_addr(
            raw.public_bind_addr.as_deref().unwrap_or("0.0.0.0:8443"),
            "public_bind_addr",
        )?;
        let admin_bind_addr = parse_addr(
            raw.admin_bind_addr.as_deref().unwrap_or("127.0.0.1:8444"),
            "admin_bind_addr",
        )?;
        let public_advertise_url = raw
            .public_advertise_url
            .unwrap_or_else(|| format!("https://127.0.0.1:{}", public_bind_addr.port()));
        let tls_cert_dir = PathBuf::from(
            raw.tls_cert_dir
                .unwrap_or_else(|| "relay-data/tls".to_string()),
        );
        let db_path = PathBuf::from(
            raw.db_path
                .unwrap_or_else(|| "relay-data/registry.sqlite3".to_string()),
        );
        let alert_pct = raw.alert_pct.unwrap_or(0.8);
        let dedup_secs = raw.dedup_secs.unwrap_or(600);

        let config = Self {
            org,
            cloud_base_url,
            cloud_viewer_key,
            public_bind_addr,
            admin_bind_addr,
            public_advertise_url,
            tls_cert_dir,
            db_path,
            alert_pct,
            dedup_secs,
            acme,
        };
        config.validate()?;
        Ok(config)
    }

    /// PUBLIC-CA mode is all-or-nothing (06 §0.5 fail-closed): if the operator
    /// set ANY `acme_*`/`broker_*` field, every REQUIRED one must be present,
    /// or the relay refuses to start rather than silently falling back to self-
    /// signed when the operator plainly meant to use a public cert. `contact_
    /// email` is the one optional field (ACME allows an empty contact).
    fn acme_from_raw(raw: &RawConfig) -> Result<Option<AcmeSettings>, ConfigError> {
        let any_set = raw.acme_directory_url.is_some()
            || raw.acme_hostname.is_some()
            || raw.acme_contact_email.is_some()
            || raw.broker_url.is_some()
            || raw.broker_user.is_some()
            || raw.broker_token.is_some();
        if !any_set {
            return Ok(None);
        }
        fn nonempty(v: &Option<String>) -> Option<&str> {
            v.as_deref().map(str::trim).filter(|s| !s.is_empty())
        }
        let mut missing = Vec::new();
        let mut take = |v: &Option<String>, name: &'static str| match nonempty(v) {
            Some(s) => s.to_string(),
            None => {
                missing.push(name);
                String::new()
            }
        };
        let directory_url = take(&raw.acme_directory_url, "acme_directory_url");
        let hostname = take(&raw.acme_hostname, "acme_hostname");
        let broker_url = take(&raw.broker_url, "broker_url");
        let broker_user = take(&raw.broker_user, "broker_user");
        let broker_token = take(&raw.broker_token, "broker_token");
        if !missing.is_empty() {
            return Err(ConfigError::Invalid(format!(
                "PUBLIC-CA mode is partially configured; missing required field(s): {}. \
                 Set all of acme_directory_url, acme_hostname, broker_url, broker_user, \
                 broker_token (contact email optional), or none of them.",
                missing.join(", ")
            )));
        }
        if !directory_url.starts_with("http://") && !directory_url.starts_with("https://") {
            return Err(ConfigError::Invalid(format!(
                "acme_directory_url must start with http:// or https://, got {directory_url}"
            )));
        }
        if !broker_url.starts_with("http://") && !broker_url.starts_with("https://") {
            return Err(ConfigError::Invalid(format!(
                "broker_url must start with http:// or https://, got {broker_url}"
            )));
        }
        Ok(Some(AcmeSettings {
            directory_url,
            hostname,
            contact_email: nonempty(&raw.acme_contact_email)
                .map(str::to_string)
                .unwrap_or_default(),
            broker_url,
            broker_user,
            broker_token,
            ca_cert_path: nonempty(&raw.acme_ca_cert).map(str::to_string),
        }))
    }

    /// Fail-closed structural checks beyond "the field parsed at all": the
    /// admin listener MUST be loopback (docs/PHASE5.md: "admin bind =
    /// loopback only"; itrat-console/13 D12.1: "never the public
    /// interface") and the alert threshold must be a real fraction.
    fn validate(&self) -> Result<(), ConfigError> {
        if !self.admin_bind_addr.ip().is_loopback() {
            return Err(ConfigError::Invalid(format!(
                "admin_bind_addr must be loopback (127.0.0.1/::1), got {}",
                self.admin_bind_addr
            )));
        }
        if !(0.0..=1.0).contains(&self.alert_pct) {
            return Err(ConfigError::Invalid(format!(
                "alert_pct must be in [0,1], got {}",
                self.alert_pct
            )));
        }
        if self.dedup_secs < 0 {
            return Err(ConfigError::Invalid(
                "dedup_secs must not be negative".to_string(),
            ));
        }
        if !self.cloud_base_url.starts_with("http://")
            && !self.cloud_base_url.starts_with("https://")
        {
            return Err(ConfigError::Invalid(format!(
                "cloud_base_url must start with http:// or https://, got {}",
                self.cloud_base_url
            )));
        }
        Ok(())
    }
}

fn require(value: Option<String>, field: &str) -> Result<String, ConfigError> {
    match value {
        Some(v) if !v.trim().is_empty() => Ok(v),
        _ => Err(ConfigError::Invalid(format!(
            "missing required config field `{field}` (set it in relay.toml or GENARYX_RELAY_{})",
            field.to_uppercase()
        ))),
    }
}

fn parse_addr(raw: &str, field: &str) -> Result<SocketAddr, ConfigError> {
    raw.parse()
        .map_err(|_| ConfigError::Invalid(format!("`{field}` is not a valid host:port: {raw}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_ok() -> RawConfig {
        RawConfig {
            org: Some("acme".to_string()),
            cloud_base_url: Some("http://127.0.0.1:8080/".to_string()),
            cloud_viewer_key: Some("key:acme:viewer:paid".to_string()),
            public_bind_addr: None,
            admin_bind_addr: None,
            public_advertise_url: None,
            tls_cert_dir: None,
            db_path: None,
            alert_pct: None,
            dedup_secs: None,
            acme_directory_url: None,
            acme_hostname: None,
            acme_contact_email: None,
            broker_url: None,
            broker_user: None,
            broker_token: None,
            acme_ca_cert: None,
        }
    }

    #[test]
    fn defaults_fill_in_and_trailing_slash_is_trimmed() {
        let cfg = RelayConfig::from_raw(raw_ok()).expect("valid minimal config");
        assert_eq!(cfg.org, "acme");
        assert_eq!(cfg.cloud_base_url, "http://127.0.0.1:8080");
        assert_eq!(cfg.public_bind_addr.port(), 8443);
        assert_eq!(cfg.admin_bind_addr.to_string(), "127.0.0.1:8444");
        assert!((cfg.alert_pct - 0.8).abs() < f64::EPSILON);
        assert_eq!(cfg.dedup_secs, 600);
        assert_eq!(cfg.public_advertise_url, "https://127.0.0.1:8443");
    }

    #[test]
    fn missing_required_field_is_invalid() {
        let mut raw = raw_ok();
        raw.cloud_viewer_key = None;
        let err = RelayConfig::from_raw(raw).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
    }

    #[test]
    fn non_loopback_admin_bind_is_rejected() {
        let mut raw = raw_ok();
        raw.admin_bind_addr = Some("0.0.0.0:9000".to_string());
        let err = RelayConfig::from_raw(raw).unwrap_err();
        match err {
            ConfigError::Invalid(msg) => assert!(msg.contains("loopback")),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn ipv6_loopback_admin_bind_is_accepted() {
        let mut raw = raw_ok();
        raw.admin_bind_addr = Some("[::1]:9000".to_string());
        let cfg = RelayConfig::from_raw(raw).expect("::1 is loopback");
        assert!(cfg.admin_bind_addr.ip().is_loopback());
    }

    #[test]
    fn out_of_range_alert_pct_is_rejected() {
        let mut raw = raw_ok();
        raw.alert_pct = Some(1.5);
        assert!(RelayConfig::from_raw(raw).is_err());
    }

    #[test]
    fn debug_never_prints_the_viewer_key() {
        let cfg = RelayConfig::from_raw(raw_ok()).expect("valid config");
        let printed = format!("{cfg:?}");
        assert!(!printed.contains("key:acme:viewer:paid"));
        assert!(printed.contains("<redacted>"));
    }

    #[test]
    fn bad_cloud_base_url_scheme_is_rejected() {
        let mut raw = raw_ok();
        raw.cloud_base_url = Some("ftp://example.com".to_string());
        assert!(RelayConfig::from_raw(raw).is_err());
    }

    #[test]
    fn no_acme_fields_means_pinned_mode() {
        let cfg = RelayConfig::from_raw(raw_ok()).expect("valid minimal config");
        assert!(
            cfg.acme.is_none(),
            "absent acme config = self-signed/pinned mode"
        );
    }

    fn raw_with_full_acme() -> RawConfig {
        let mut raw = raw_ok();
        raw.acme_directory_url = Some("https://acme-v02.api.letsencrypt.org/directory".to_string());
        raw.acme_hostname = Some("abc123.pocket.it-rat.com".to_string());
        raw.broker_url = Some("https://broker.pocket.it-rat.com".to_string());
        raw.broker_user = Some("abc123".to_string());
        raw.broker_token = Some("relay-secret".to_string());
        raw
    }

    #[test]
    fn full_acme_config_switches_to_public_ca_mode() {
        let cfg = RelayConfig::from_raw(raw_with_full_acme()).expect("valid public-ca config");
        let acme = cfg.acme.expect("public-ca mode active");
        assert_eq!(acme.hostname, "abc123.pocket.it-rat.com");
        assert_eq!(acme.broker_user, "abc123");
        assert!(acme.contact_email.is_empty(), "contact is optional");
    }

    #[test]
    fn partial_acme_config_is_rejected_not_silently_ignored() {
        // Hostname set but broker creds missing: the operator meant public-CA,
        // so falling back to self-signed silently would be a fail-OPEN.
        let mut raw = raw_ok();
        raw.acme_hostname = Some("abc123.pocket.it-rat.com".to_string());
        let err = RelayConfig::from_raw(raw).unwrap_err();
        match err {
            ConfigError::Invalid(msg) => {
                assert!(
                    msg.contains("acme_directory_url"),
                    "names the missing field"
                );
                assert!(msg.contains("broker_url"));
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn acme_debug_never_prints_the_broker_token() {
        let cfg = RelayConfig::from_raw(raw_with_full_acme()).expect("valid config");
        let printed = format!("{cfg:?}");
        assert!(
            !printed.contains("relay-secret"),
            "broker token must be redacted"
        );
        assert!(printed.contains("<redacted>"));
    }

    #[test]
    fn acme_directory_url_scheme_is_validated() {
        let mut raw = raw_with_full_acme();
        raw.acme_directory_url = Some("ftp://ca.example/dir".to_string());
        assert!(RelayConfig::from_raw(raw).is_err());
    }
}
