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
}

#[derive(Debug, thiserror::Error)]
enum RelayError {
    #[error("{0}")]
    Config(#[from] config::ConfigError),
    #[error("{0}")]
    License(#[from] license::LicenseError),
    #[error("{0}")]
    Tls(#[from] tls::TlsError),
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

    let identity = tls::RelayIdentity::load_or_generate(&config.tls_cert_dir)?;
    eprintln!(
        "genaryx-relay: public listener SPKI-SHA256 pin = {} (this is the QR's trust root)",
        identity.spki_sha256_b64()
    );
    let rustls_config = identity.rustls_config().await?;

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
        public_advertise_url: config.public_advertise_url.clone(),
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
        relay_url: config.public_advertise_url.clone(),
        org: config.org.clone(),
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
