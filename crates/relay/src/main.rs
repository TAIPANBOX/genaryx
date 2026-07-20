//! genaryx-relay: the always-on headless relay (Phase 5 / D12).
//!
//! Placement + trust model (itrat-console/13 D12.1): runs on client infra
//! colocated with the firewalled TokenFuse Cloud (loopback), the single
//! deliberate internet-facing door for the operator's phone. It is a
//! least-privilege pipe: it holds a Cloud VIEWER key, so the Cloud itself
//! rejects any mutation the relay could try (`tokenfuse http.rs:276-277`). Kill
//! authority lives only in the phone's Enclave/software signer; the relay
//! forwards the phone-signed request verbatim and the Cloud verifies it end to
//! end. Everything here is defensive: it lets an operator protect their own
//! budget/agents from anywhere, never grants any autonomous destructive power.
//!
//! This file is the composition root: parse config, run the license gate, a
//! Cloud health check, bring up `CloudSse` + the `ExceptionEngine`, then
//! serve the public TLS listener (phone-facing) and the loopback-only admin
//! listener (desktop-facing) side by side. See `docs/PHASE5.md` for the wave
//! plan and `~/Development/itrat-console/13-mobile-relay-copilot-decision.md`
//! (D12.1-D12.4) for the full spec this module wires together.

// The ACME DNS-01 client (cert broker, design A): the relay obtains its own
// publicly-trusted certificate through the Pocket broker (see `acme.rs`).
mod acme;
mod admin;
mod config;
mod exceptions;
mod license;
mod pairing;
mod proxy;
mod push;
mod ratelimit;
mod registry;
mod tls;
mod triage;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use genaryx_connectors::{CloudClient, CloudSse, CloudSseConfig};

/// State shared by every handler on the PUBLIC (phone-facing, TLS) listener.
#[derive(Clone)]
pub struct PublicState {
    pub registry: Arc<registry::Registry>,
    pub engine: Arc<exceptions::ExceptionEngine>,
    /// Plain client for the read-proxy and mutation pass-through: carries no
    /// baked-in credential (each call forwards the CALLER's own headers),
    /// unlike the viewer-keyed clients the SSE/reconcile paths use.
    pub http: reqwest::Client,
    pub cloud_base_url: String,
    pub public_advertise_url: String,
    pub mutation_rate_limiter: Arc<ratelimit::RateLimiter>,
    pub pairing_rate_limiter: Arc<ratelimit::RateLimiter>,
}

/// State shared by every handler on the ADMIN (loopback-only) listener.
#[derive(Clone)]
pub struct AdminState {
    pub registry: Arc<registry::Registry>,
    /// The public listener's SPKI-SHA256 pin, base64 -- see
    /// `admin::PairingInfoResponse`'s doc for why this trio is served here
    /// rather than making the desktop re-derive it from `RelayConfig`/
    /// `RelayIdentity` itself (Phase 5 W2).
    pub pin: String,
    pub relay_url: String,
    pub org: String,
    /// Present only in PUBLIC-CA trust mode (cert broker, design A): the
    /// `<relay-id>.pocket.it-rat.com` hostname the phone connects to with
    /// ordinary system trust. `None` = self-signed + SPKI-pin mode. Flows into
    /// `admin::PairingInfoResponse::hostname` and, from there, the QR.
    pub hostname: Option<String>,
}

#[derive(Debug, thiserror::Error)]
enum RelayError {
    #[error("{0}")]
    Config(#[from] config::ConfigError),
    #[error("{0}")]
    License(#[from] license::LicenseError),
    #[error("{0}")]
    Tls(#[from] tls::TlsError),
    #[error("acme (cert broker): {0}")]
    Acme(#[from] acme::AcmeError),
    #[error("{0}")]
    Registry(#[from] registry::RegistryError),
    #[error("{0}")]
    Connector(#[from] genaryx_connectors::ConnectorError),
    #[error("cloud health check failed: {0}")]
    HealthCheck(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to start the Cloud stream: {0}")]
    Sse(String),
}

fn main() -> ExitCode {
    // A hand-rolled multi-thread runtime (rather than `#[tokio::main]`) only
    // so `run`'s `Result` can drive the process exit code the same fail-
    // closed way every other stage here does -- `main` itself does no async
    // work of its own.
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("genaryx-relay: fatal: failed to start the async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("genaryx-relay: fatal: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), RelayError> {
    let config = config::RelayConfig::load()?;
    eprintln!(
        "genaryx-relay: starting: org={} cloud={} public={} admin={}",
        config.org, config.cloud_base_url, config.public_bind_addr, config.admin_bind_addr
    );

    // License gate (docs/PHASE5.md: "Do not block the sim build on real
    // licensing"). Sim uses the permissive stub; R1 wires the real ML-DSA
    // check behind the exact same `.check()` call.
    license::LicenseGate::permissive().check()?;

    // Fail-closed: refuse to start without a Cloud health check, BEFORE
    // touching TLS/registry/SSE (itrat-console/13 D12.3: "relay refuses to
    // start without a valid license, a readable `.p8`, and a Cloud health
    // check").
    let plain_http = reqwest::Client::builder()
        .build()
        .map_err(|e| RelayError::HealthCheck(e.to_string()))?;
    health_check(&plain_http, &config).await?;
    eprintln!(
        "genaryx-relay: Cloud health check OK ({})",
        config.cloud_base_url
    );

    // Resolve the phone-facing TLS identity for whichever trust mode config
    // selected: self-signed + SPKI-pin (default), or a publicly-trusted
    // certificate obtained through the cert broker (PUBLIC-CA mode, design A).
    let PublicTls {
        identity,
        rustls_config,
        advertise_url,
        hostname,
    } = setup_public_tls(&config).await?;
    // In PUBLIC-CA mode, keep the cert renewed in the background and hot-reload
    // it into the live listener, well before the CA's ~90-day expiry -- no
    // restart, no pin churn.
    if let Some(acme_settings) = config.acme.clone() {
        tokio::spawn(run_cert_renewal(
            rustls_config.clone(),
            acme_settings,
            config.tls_cert_dir.clone(),
        ));
    }

    let registry = Arc::new(registry::Registry::open(&config.db_path)?);
    {
        let paired = registry.devices()?;
        if paired.is_empty() {
            eprintln!("genaryx-relay: registry: no device paired yet (phone and watch slots free)");
        } else {
            for d in &paired {
                eprintln!(
                    "genaryx-relay: registry: {} already paired ({})",
                    d.kind.as_str(),
                    d.device_id
                );
            }
            for kind in registry::DeviceKind::ALL {
                if !paired.iter().any(|d| d.kind == kind) {
                    eprintln!("genaryx-relay: registry: {} slot free", kind.as_str());
                }
            }
        }
    }

    let engine = Arc::new(exceptions::ExceptionEngine::new(
        config.org.clone(),
        config.alert_pct,
        config.dedup_secs,
    ));
    let push_sender: Arc<dyn push::ApnsSender> = Arc::new(push::NullSender);

    // Initial reconcile BEFORE serving (D12.2b step 1's "on connect" case),
    // so the very first `GET /relay/v1/exceptions` never answers from an
    // empty, not-yet-synced queue.
    let viewer_client = CloudClient::new(&config.cloud_base_url, &config.cloud_viewer_key)?;
    engine.reconcile(&viewer_client).await?;
    eprintln!("genaryx-relay: initial exception-queue reconcile OK");

    let sse_config = CloudSseConfig {
        url: format!("{}/v1/stream", config.cloud_base_url),
        bearer_token: config.cloud_viewer_key.clone(),
        initial_backoff: Duration::from_millis(250),
        max_backoff: Duration::from_secs(30),
        // An always-on relay retries forever rather than giving up after a
        // fixed budget (contrast `CloudSseConfig::new`'s Phase-0 default of
        // 10 attempts, right for a short-lived console session, wrong for a
        // headless server); this is also WHY reconcile-on-reconnect is
        // approximated by a periodic sweep rather than an exhaustion signal
        // (see `exceptions.rs`'s module docs).
        max_attempts: None,
    };
    let sse = CloudSse::spawn("relay", sse_config).map_err(|e| RelayError::Sse(e.to_string()))?;

    let sweep_client = CloudClient::new(&config.cloud_base_url, &config.cloud_viewer_key)?;
    // C3 (docs/PHASE6-C3.md): the triage stage in front of the push path. HARD
    // events push immediately (deterministic floor); an optional, budgeted Felyx
    // annotation enriches the polled snapshot; SOFT events batch into a digest.
    // Copilot is off unless a provider is configured (GENARYX_RELAY_COPILOT_*),
    // so by default the relay pages exactly as before C3.
    let triage = Arc::new(triage::Triage::new(
        engine.clone(),
        registry.clone(),
        push_sender.clone(),
        triage::build_copilot_from_env(),
        triage::TriageConfig::from_env(),
    ));
    tokio::spawn(exceptions::run_event_loop(triage, sse));
    tokio::spawn(exceptions::run_reconcile_sweep(
        engine.clone(),
        sweep_client,
        Duration::from_secs(60),
    ));

    let public_state = PublicState {
        registry: registry.clone(),
        engine: engine.clone(),
        http: plain_http,
        cloud_base_url: config.cloud_base_url.clone(),
        public_advertise_url: advertise_url.clone(),
        // 30/min per device on mutations (kill/budget/ack): generous for a
        // real Face-ID-gated human tapping a button, tight for a bug or a
        // hostile relay-side actor trying to hammer the Cloud through it.
        mutation_rate_limiter: Arc::new(ratelimit::RateLimiter::new(30, Duration::from_secs(60))),
        // 10/min per source IP on the pre-auth pairing route.
        pairing_rate_limiter: Arc::new(ratelimit::RateLimiter::new(10, Duration::from_secs(60))),
    };
    let admin_state = AdminState {
        registry: registry.clone(),
        pin: identity.spki_sha256_b64().to_string(),
        relay_url: advertise_url.clone(),
        org: config.org.clone(),
        hostname: hostname.clone(),
    };

    let public_app = axum::Router::new()
        .route("/relay/v1/pair", axum::routing::post(pairing::pair_handler))
        .route(
            "/relay/v1/exceptions",
            axum::routing::get(exceptions::exceptions_handler),
        )
        .route("/v1/summary", axum::routing::get(proxy::summary_handler))
        .route(
            "/v1/runs/{run}/kill",
            axum::routing::post(proxy::mutation_passthrough),
        )
        .route(
            "/v1/runs/{run}/budget",
            axum::routing::post(proxy::mutation_passthrough),
        )
        .route(
            "/v1/incidents/{id}/ack",
            axum::routing::post(proxy::mutation_passthrough),
        )
        .with_state(public_state);

    let admin_app = axum::Router::new()
        .route(
            "/admin/pairing-info",
            axum::routing::get(admin::pairing_info),
        )
        .route(
            "/admin/pairing-window",
            axum::routing::post(admin::arm_pairing_window),
        )
        .route("/admin/devices", axum::routing::get(admin::get_devices))
        .route("/admin/disconnect", axum::routing::post(admin::disconnect))
        .with_state(admin_state);

    eprintln!(
        "genaryx-relay: public TLS listener on {} ; admin (loopback) listener on {}",
        config.public_bind_addr, config.admin_bind_addr
    );

    let public_server = axum_server::bind_rustls(config.public_bind_addr, rustls_config)
        .serve(public_app.into_make_service_with_connect_info::<SocketAddr>());
    let admin_listener = tokio::net::TcpListener::bind(config.admin_bind_addr).await?;
    let admin_server = axum::serve(admin_listener, admin_app.into_make_service());

    tokio::select! {
        res = public_server => {
            if let Err(e) = res {
                eprintln!("genaryx-relay: public listener ended: {e}");
            }
        }
        res = admin_server => {
            if let Err(e) = res {
                eprintln!("genaryx-relay: admin listener ended: {e}");
            }
        }
    }
    Ok(())
}

/// `GET /v1/summary` with the relay's own viewer key: the one probe
/// `main`'s fail-closed startup gate needs (config is valid AND the Cloud is
/// actually reachable and accepting this key), reusing no other machinery.
async fn health_check(
    http: &reqwest::Client,
    config: &config::RelayConfig,
) -> Result<(), RelayError> {
    let resp = http
        .get(format!("{}/v1/summary", config.cloud_base_url))
        .bearer_auth(&config.cloud_viewer_key)
        .send()
        .await
        .map_err(|e| {
            RelayError::HealthCheck(format!("connecting to {}: {e}", config.cloud_base_url))
        })?;
    if !resp.status().is_success() {
        return Err(RelayError::HealthCheck(format!(
            "Cloud responded HTTP {} to the viewer-key /v1/summary probe",
            resp.status()
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Public-listener TLS: pick the trust mode, obtain/renew the certificate.
// ---------------------------------------------------------------------------

/// A certificate is renewed once it is older than this. Let's Encrypt certs
/// live ~90 days; 60 leaves a month of retry headroom before expiry.
const CERT_RENEW_AFTER: Duration = Duration::from_secs(60 * 24 * 3600);
/// How often the background task re-checks the certificate's age.
const CERT_CHECK_EVERY: Duration = Duration::from_secs(12 * 3600);
/// Bound on a single ACME order, and the gap between its status polls.
const ACME_POLL_TIMEOUT: Duration = Duration::from_secs(120);
const ACME_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// The resolved phone-facing TLS identity plus what to advertise for it.
struct PublicTls {
    identity: tls::RelayIdentity,
    rustls_config: axum_server::tls_rustls::RustlsConfig,
    /// What the relay tells a pairing phone to connect to.
    advertise_url: String,
    /// `Some(hostname)` in PUBLIC-CA mode, `None` when self-signed/pinned.
    hostname: Option<String>,
}

/// Build the public listener's TLS identity for the configured trust mode.
async fn setup_public_tls(config: &config::RelayConfig) -> Result<PublicTls, RelayError> {
    match &config.acme {
        None => {
            let identity = tls::RelayIdentity::load_or_generate(&config.tls_cert_dir)?;
            eprintln!(
                "genaryx-relay: trust mode = self-signed + SPKI pin; pin = {} (the QR's trust root)",
                identity.spki_sha256_b64()
            );
            let rustls_config = identity.rustls_config().await?;
            Ok(PublicTls {
                identity,
                rustls_config,
                advertise_url: config.public_advertise_url.clone(),
                hostname: None,
            })
        }
        Some(acme) => {
            let identity = obtain_or_load_acme_identity(acme, &config.tls_cert_dir).await?;
            eprintln!(
                "genaryx-relay: trust mode = PUBLIC-CA; serving {} with a publicly-trusted \
                 certificate (the phone needs no pin and no ATS exception)",
                acme.hostname
            );
            let rustls_config = identity.rustls_config().await?;
            Ok(PublicTls {
                identity,
                rustls_config,
                advertise_url: format!("https://{}", acme.hostname),
                hostname: Some(acme.hostname.clone()),
            })
        }
    }
}

/// Reuse the persisted certificate if it is still comfortably fresh; otherwise
/// obtain a new one through the broker. PUBLIC-CA mode NEVER falls back to self-
/// signed: if the order fails on first boot the relay fails to start (fail-
/// closed), rather than quietly serving a certificate the phone would reject.
async fn obtain_or_load_acme_identity(
    acme: &config::AcmeSettings,
    dir: &Path,
) -> Result<tls::RelayIdentity, RelayError> {
    let have = dir.join("cert.pem").exists() && dir.join("key.pem").exists();
    let fresh = acme_cert_age(dir).is_some_and(|age| age < CERT_RENEW_AFTER);
    if have && fresh {
        eprintln!("genaryx-relay: PUBLIC-CA: reusing the persisted certificate (still fresh)");
        return Ok(tls::RelayIdentity::load_existing(dir)?);
    }
    eprintln!(
        "genaryx-relay: PUBLIC-CA: obtaining a certificate for {} via the broker...",
        acme.hostname
    );
    let bundle = run_acme_order(acme, dir).await?;
    let identity =
        tls::RelayIdentity::install(dir, bundle.cert_pem.as_bytes(), bundle.key_pem.as_bytes())?;
    write_issued_now(dir)?;
    eprintln!("genaryx-relay: PUBLIC-CA: certificate obtained and installed");
    Ok(identity)
}

/// One ACME DNS-01 order for `acme.hostname`, mediated by the broker. The
/// account key is persisted (mode 0600) and reused across restarts; the
/// certificate key is freshly generated for each order.
async fn run_acme_order(
    acme: &config::AcmeSettings,
    dir: &Path,
) -> Result<acme::CertBundle, RelayError> {
    let mut builder = reqwest::Client::builder();
    if let Some(ca_path) = &acme.ca_cert_path {
        let pem = std::fs::read(ca_path)?;
        let ca = reqwest::Certificate::from_pem(&pem)
            .map_err(|e| acme::AcmeError::Http(format!("bad acme_ca_cert {ca_path}: {e}")))?;
        builder = builder.add_root_certificate(ca);
    }
    let http = builder
        .build()
        .map_err(|e| acme::AcmeError::Http(e.to_string()))?;
    let broker = acme::BrokerClient::new(
        http.clone(),
        acme.broker_url.clone(),
        acme.broker_user.clone(),
        acme.broker_token.clone(),
    );
    let client = acme::AcmeClient::new(
        http,
        broker,
        acme::AcmeConfig {
            directory_url: acme.directory_url.clone(),
            hostname: acme.hostname.clone(),
            contact_email: acme.contact_email.clone(),
            poll_timeout: ACME_POLL_TIMEOUT,
            poll_interval: ACME_POLL_INTERVAL,
        },
    );
    let account = load_or_make_account_key(dir)?;
    let cert_key = rcgen::KeyPair::generate().map_err(|e| acme::AcmeError::Csr(e.to_string()))?;
    Ok(client.obtain_certificate(&account, &cert_key).await?)
}

/// Renew the certificate in the background and hot-reload it into the live
/// listener. An obtain failure keeps the current (still-valid) certificate and
/// retries next cycle -- a transient CA/broker blip must never take the phone
/// channel down.
async fn run_cert_renewal(
    rustls: axum_server::tls_rustls::RustlsConfig,
    acme: config::AcmeSettings,
    dir: PathBuf,
) {
    loop {
        tokio::time::sleep(CERT_CHECK_EVERY).await;
        if acme_cert_age(&dir).is_some_and(|age| age < CERT_RENEW_AFTER) {
            continue; // still fresh
        }
        eprintln!(
            "genaryx-relay: cert renewal: certificate for {} is due; obtaining a fresh one",
            acme.hostname
        );
        match run_acme_order(&acme, &dir).await {
            Ok(bundle) => {
                if let Err(e) = tls::RelayIdentity::install(
                    &dir,
                    bundle.cert_pem.as_bytes(),
                    bundle.key_pem.as_bytes(),
                ) {
                    eprintln!("genaryx-relay: cert renewal: install failed: {e}");
                    continue;
                }
                let _ = write_issued_now(&dir);
                match rustls
                    .reload_from_pem(bundle.cert_pem.into_bytes(), bundle.key_pem.into_bytes())
                    .await
                {
                    Ok(()) => eprintln!(
                        "genaryx-relay: cert renewal: new certificate is live (hot-reloaded, no restart)"
                    ),
                    Err(e) => eprintln!("genaryx-relay: cert renewal: reload failed: {e}"),
                }
            }
            Err(e) => eprintln!(
                "genaryx-relay: cert renewal: obtain failed, keeping the current certificate: {e}"
            ),
        }
    }
}

/// Load the persisted ACME account key, or generate + persist one (mode 0600).
/// Reusing it across restarts keeps one stable ACME account instead of
/// registering a fresh one on every boot.
fn load_or_make_account_key(
    dir: &Path,
) -> Result<genaryx_signing::es256::SoftwareSigner, RelayError> {
    use base64::Engine as _;
    use genaryx_signing::es256::SoftwareSigner;
    let b64 = base64::engine::general_purpose::STANDARD;
    let path = dir.join("acme-account.key");
    if path.exists() {
        let text = std::fs::read_to_string(&path)?;
        let bytes = b64
            .decode(text.trim())
            .map_err(|e| acme::AcmeError::Protocol(format!("acme-account.key not base64: {e}")))?;
        Ok(SoftwareSigner::from_scalar(&bytes).map_err(acme::AcmeError::from)?)
    } else {
        std::fs::create_dir_all(dir)?;
        let signer = SoftwareSigner::generate().map_err(acme::AcmeError::from)?;
        write_owner_only(&path, b64.encode(signer.to_scalar_bytes()).as_bytes())?;
        Ok(signer)
    }
}

/// Seconds since the certificate was issued (from the `acme-issued` sidecar),
/// or `None` if it was never written (no certificate obtained yet).
fn acme_cert_age(dir: &Path) -> Option<Duration> {
    let text = std::fs::read_to_string(dir.join("acme-issued")).ok()?;
    let issued: u64 = text.trim().parse().ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(Duration::from_secs(now.saturating_sub(issued)))
}

fn write_issued_now(dir: &Path) -> std::io::Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| std::io::Error::other("system clock before the unix epoch"))?
        .as_secs();
    std::fs::write(dir.join("acme-issued"), now.to_string())
}

#[cfg(unix)]
fn write_owner_only(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_owner_only(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The FULL wired PUBLIC-CA path -- config settings -> broker + account-key
    /// persistence -> ACME order -> install -> `RelayIdentity`, then a second
    /// call proving the fresh cert is reused rather than reissued -- against a
    /// real Pebble + the broker. Ignored like the acme-module network test;
    /// same `RELAY_ACME_*` env, run over the SSH tunnel. This exercises the glue
    /// in `main.rs` (the extra-CA reqwest client, account-key round-trip, the
    /// reuse branch), not just the `acme` module in isolation.
    #[tokio::test]
    #[ignore = "needs a reachable Pebble ACME server + broker (see acme.rs)"]
    async fn public_ca_identity_obtains_then_reuses() {
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("set {k}"));
        let host = env("RELAY_ACME_HOST");
        let settings = config::AcmeSettings {
            directory_url: env("RELAY_ACME_DIR"),
            hostname: host.clone(),
            contact_email: String::new(),
            broker_url: env("RELAY_ACME_BROKER"),
            broker_user: env("RELAY_ACME_BROKER_USER"),
            broker_token: env("RELAY_ACME_BROKER_TOKEN"),
            ca_cert_path: Some(env("RELAY_ACME_CA")),
        };
        let dir = std::env::temp_dir().join(format!(
            "genaryx-relay-acme-it-{}",
            genaryx_signing::es256::random_hex(6).unwrap()
        ));

        // First call obtains and installs a real certificate.
        let id1 = obtain_or_load_acme_identity(&settings, &dir)
            .await
            .expect("first call must obtain a certificate");
        let cert = std::fs::read_to_string(dir.join("cert.pem")).unwrap();
        assert!(cert.contains("BEGIN CERTIFICATE"), "cert.pem installed");
        assert!(dir.join("acme-account.key").exists(), "account key persisted");
        assert!(dir.join("acme-issued").exists(), "issued sidecar written");
        let pin1 = id1.spki_sha256_b64().to_string();

        // Second call: the fresh cert on disk is reused (no new order, same pin).
        let id2 = obtain_or_load_acme_identity(&settings, &dir)
            .await
            .expect("second call must reuse the fresh certificate");
        assert_eq!(
            id2.spki_sha256_b64(),
            pin1,
            "a still-fresh certificate must be reused, not reissued"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
