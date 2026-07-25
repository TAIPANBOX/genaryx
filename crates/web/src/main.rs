//! `genaryx-web`: the Genaryx console, served over HTTP from inside the
//! customer's own perimeter.
//!
//! This process runs beside the customer's stack, on their box. It reads the
//! same `~/.taipan/environments/` descriptors the former desktop console once
//! read, and answers every request by calling `genaryx-api` directly - the
//! same functions the Tauri shell used to wrap. No run, spend figure,
//! identity or policy decision leaves the customer's network to render this
//! UI, and it-rat.com has no route to it: the site sells and licenses the
//! product, it never sees the data.
//!
//! Reaching it is the operator's own tunnel (D11). The default bind is
//! loopback for exactly that reason, and binding wider says so out loud.

mod auth;
mod config;
mod ctx;
mod dispatch;
mod doctor;
mod lifecycle;
mod oidc;
mod roles;
mod webauthn;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use clap::{Parser, Subcommand};
use config::Config;
use ctx::Ctx;
use futures_util::stream::Stream;
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Parser)]
#[command(
    name = "genaryx-web",
    about = "Genaryx console, served on your own box"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Serve the console.
    Serve {
        /// Address to listen on. Default is loopback: bind your tunnel's
        /// address (for example 10.9.0.1:7420) to reach it from a paired
        /// device, never 0.0.0.0 unless something in front terminates TLS.
        #[arg(long, default_value = "127.0.0.1:7420")]
        bind: SocketAddr,
        /// Where the operator record lives.
        #[arg(long)]
        state_dir: Option<PathBuf>,
        /// Directory of the built web UI. Omit to serve the API only, which
        /// is what a `vite dev` front end wants.
        #[arg(long)]
        ui: Option<PathBuf>,
        /// Mark the session cookie Secure. Only with TLS in front, since the
        /// browser then refuses to send it over plain HTTP.
        #[arg(long)]
        secure_cookies: bool,
    },
    /// Say, per plane, whether it resolved your stack and if not exactly what
    /// is missing. Exits non-zero when something is wrong, so it can gate a
    /// deploy.
    Doctor,
    /// Set the single operator account. The password is read from stdin, so
    /// it never appears in the process list.
    SetPassword {
        #[arg(long)]
        username: String,
        #[arg(long)]
        state_dir: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "genaryx_web=info,tower_http=warn".into()),
        )
        .init();

    match Cli::parse().cmd {
        Cmd::Doctor => {
            let report = doctor::run();
            doctor::print(&report);
            if !report.all_ok() {
                std::process::exit(1);
            }
        }
        Cmd::SetPassword {
            username,
            state_dir,
        } => {
            let dir = state_dir.unwrap_or_else(Config::default_state_dir);
            let path = dir.join("operator.json");
            let mut pw = String::new();
            if std::io::stdin()
                .lines()
                .next()
                .map(|l| {
                    pw = l.unwrap_or_default();
                })
                .is_none()
            {
                eprintln!("genaryx-web: no password on stdin");
                std::process::exit(2);
            }
            match auth::set_operator(&path, &username, pw.trim_end_matches(['\r', '\n'])) {
                Ok(()) => println!("operator '{username}' set in {}", path.display()),
                Err(e) => {
                    eprintln!("genaryx-web: {e}");
                    std::process::exit(1);
                }
            }
        }
        Cmd::Serve {
            bind,
            state_dir,
            ui,
            secure_cookies,
        } => {
            let cfg = Config {
                bind,
                state_dir: state_dir.unwrap_or_else(Config::default_state_dir),
                ui_dir: ui,
                secure_cookies,
            };
            serve(cfg).await;
        }
    }
}

async fn serve(cfg: Config) {
    cfg.warn_if_exposed();
    if auth::load(&cfg.operator_file()).is_none() {
        tracing::warn!(
            file = %cfg.operator_file().display(),
            "no operator account yet: run `genaryx-web set-password --username <name>` \
             (password on stdin). Until then every sign-in is refused."
        );
    }

    // Say which panels will be empty and why, before serving a single request.
    // An operator who never runs `doctor` still gets told.
    doctor::log(&doctor::run());

    // The console's own bus, fed exactly as the desktop shell feeds it. The
    // sender is created first and handed to the feeder, so every event the
    // feeder produces lands on the same channel the SSE streams read.
    let (events_tx, _) = tokio::sync::broadcast::channel(512);
    let bus = match genaryx_api::bus::feed::bootstrap(SseSink(events_tx.clone())) {
        Ok(b) => genaryx_api::bus::AppState {
            events_dir: Some(b.events_dir),
            mode: b.mode,
        },
        Err(e) => {
            // Fail-closed, exactly as the desktop shell does: a bus that will
            // not open degrades the Bus Explorer, it never stops the console
            // serving the planes that are fine.
            tracing::error!(error = %e, "bus startup failed; the Bus Explorer will be empty");
            genaryx_api::bus::AppState {
                events_dir: None,
                mode: genaryx_api::bus::BusMode::Unavailable {
                    reason: e.to_string(),
                },
            }
        }
    };
    let ctx = Arc::new(Ctx::bootstrap(cfg, bus, events_tx));
    ctx.resolve();

    let app = app(Arc::clone(&ctx));
    let listener = match tokio::net::TcpListener::bind(ctx.cfg.bind).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(bind = %ctx.cfg.bind, error = %e, "cannot bind");
            std::process::exit(1);
        }
    };
    tracing::info!(bind = %ctx.cfg.bind, "genaryx-web ready");
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!(error = %e, "server stopped");
    }
}

/// Build the router for a resolved [`Ctx`]. Extracted from [`serve`] so the
/// same routes are exercised by tests (via `oneshot`) rather than only bound
/// to a socket. The static-file fallback is attached only when a UI dir is
/// configured, exactly as in production.
fn app(ctx: Arc<Ctx>) -> Router {
    let api = Router::new()
        .route("/auth/session", get(session))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/webauthn/passkeys", get(webauthn_list))
        .route("/webauthn/register/start", post(webauthn_register_start))
        .route("/webauthn/register/finish", post(webauthn_register_finish))
        .route("/webauthn/action/start", post(webauthn_action_start))
        .route("/command/{name}", post(command))
        .route("/events", get(events));

    let mut app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/api", api);

    if let Some(dir) = ctx.cfg.ui_dir.clone() {
        // A single-page app: unknown paths are routes, not missing files, so
        // they fall back to index.html rather than 404ing.
        let index = dir.join("index.html");
        app = app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)));
    }

    app.with_state(ctx)
}

/// Bridges the shared bus feeder onto this process's broadcast channel.
struct SseSink(tokio::sync::broadcast::Sender<genaryx_api::events::UiEvent>);

impl genaryx_api::bus::feed::EventSink for SseSink {
    fn emit(&self, event: genaryx_api::events::UiEvent) {
        // A send with no subscribers is not an error: nobody has the console
        // open, and the store still has the event when they do.
        let _ = self.0.send(event);
    }
}

// ---------------------------------------------------------------------------
// auth
// ---------------------------------------------------------------------------

/// Either login shape: a local `username`+`password`, or an OIDC `id_token`.
/// All fields optional so one endpoint accepts both; `login` decides which
/// path by what is present (a token takes precedence).
#[derive(Deserialize)]
struct Credentials {
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

/// Who the caller is, and whether this box has an operator at all.
///
/// Answering "not configured" to an anonymous caller is deliberate: it is the
/// difference between a first-run box that needs setting up and a box whose
/// password you do not know, and the operator standing in front of it needs
/// to be told which.
async fn session(State(ctx): State<Arc<Ctx>>, jar: CookieJar) -> Response {
    let configured = ctx.operator.read().expect("operator lock").is_some();
    let info = jar
        .get(auth::COOKIE)
        .and_then(|c| ctx.sessions.touch(c.value()));
    Json(json!({
        "configured": configured,
        "signed_in": info.is_some(),
        "user": info.as_ref().map(|i| i.user.clone()),
        "role": info.as_ref().map(|i| i.role.as_str()),
        "method": info.as_ref().map(|i| i.method.as_str()),
        // Whether the "Sign in with your organization" option should show.
        "oidc_available": ctx.oidc.is_some(),
    }))
    .into_response()
}

async fn login(
    State(ctx): State<Arc<Ctx>>,
    jar: CookieJar,
    Json(body): Json<Credentials>,
) -> Response {
    // An OIDC token takes precedence when present (and configured). The local
    // account stays as break-glass either way.
    let resolved = if let Some(token) = body.id_token.as_deref().filter(|t| !t.trim().is_empty()) {
        let Some(cfg) = ctx.oidc.as_ref() else {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "OIDC is not configured on this box"})),
            )
                .into_response();
        };
        match oidc::verify(cfg, token) {
            // Never log the token (a bearer secret); log only the mapped user.
            Some(v) => {
                tracing::info!(user = %v.username, role = %v.role.as_str(), "signed in (oidc)");
                (v.username, v.role, auth::Method::Oidc)
            }
            None => {
                tracing::warn!("oidc sign-in refused");
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "the ID token was not accepted"})),
                )
                    .into_response();
            }
        }
    } else {
        let op = ctx.operator.read().expect("operator lock").clone();
        let Some(op) = op else {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": "no operator account on this box yet"})),
            )
                .into_response();
        };
        let username = body.username.as_deref().unwrap_or_default();
        let password = body.password.as_deref().unwrap_or_default();
        if !auth::verify(&op, username, password) {
            tracing::warn!(user = %username, "sign-in refused");
            // One message for both failures: the endpoint must not say which
            // half was wrong.
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "wrong username or password"})),
            )
                .into_response();
        }
        tracing::info!(user = %op.username, "signed in (local)");
        // The local owner account is the box's break-glass admin.
        (op.username, roles::Role::Admin, auth::Method::Local)
    };

    let (user, role, method) = resolved;
    let id = ctx.sessions.create(&user, role, method);
    let mut cookie = Cookie::new(auth::COOKIE, id);
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Strict);
    cookie.set_path("/");
    cookie.set_secure(ctx.cfg.secure_cookies);
    (
        jar.add(cookie),
        Json(json!({
            "signed_in": true, "user": user,
            "role": role.as_str(), "method": method.as_str(),
        })),
    )
        .into_response()
}

async fn logout(State(ctx): State<Arc<Ctx>>, jar: CookieJar) -> Response {
    if let Some(c) = jar.get(auth::COOKIE) {
        ctx.sessions.revoke(c.value());
    }
    let mut gone = Cookie::from(auth::COOKIE);
    gone.set_path("/");
    (jar.remove(gone), Json(json!({"signed_in": false}))).into_response()
}

/// Reject anything without a live session.
// axum's `Response` is a large value, and clippy would rather it were boxed.
// It is deliberately not: a ready-made `Response` IS the error here (the exact
// status and JSON body the browser should get), and boxing it would add an
// allocation and a deref at every call site to satisfy a lint about a type we
// do not control.
#[allow(clippy::result_large_err)]
fn guard(ctx: &Arc<Ctx>, jar: &CookieJar) -> Result<auth::SessionInfo, Response> {
    jar.get(auth::COOKIE)
        .and_then(|c| ctx.sessions.touch(c.value()))
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "not signed in"})),
            )
                .into_response()
        })
}

// ---------------------------------------------------------------------------
// WebAuthn ceremony (docs/CONSOLE-IDP.md, B3/2)
// ---------------------------------------------------------------------------

/// The commands whose dispatch REQUIRES a fresh per-action assertion once the
/// caller has a passkey enrolled: the kill and the budget mutation (the two
/// break-glass carriers) and the approval grant/deny. The policy PUT/DELETE
/// editor joins this list the day it becomes routable (it is "v1, not built"
/// today); an unknown name is already fail-closed to admin by the role gate.
const SENSITIVE_COMMANDS: &[&str] = &[
    "money_kill_run",
    "money_set_budget",
    "policy_decide_approval",
    // Issuing a WireGuard peer hands out a road into the control plane, and
    // revoking one takes an operator's access away mid-incident. Both are the
    // same class of act as a kill: the role gate says who MAY, the ceremony
    // says the human is present for THIS one. Without this, a stolen console
    // session could quietly mint itself a permanent tunnel.
    "remote_operator_wg_config",
    "remote_operator_wg_revoke",
];

/// `POST /api/webauthn/action/start`'s body: which command the operator is
/// about to confirm, with the exact args the later dispatch will carry.
#[derive(Deserialize)]
struct ActionStart {
    command: String,
    #[serde(default)]
    args: Value,
}

/// The assertion envelope the browser sends back in `x-genaryx-webauthn`
/// (base64url of this JSON): the WebAuthn response's raw fields, each
/// base64url exactly as the frontend encodes the API's ArrayBuffers.
#[derive(Deserialize)]
struct AssertionEnvelope {
    credential_id: String,
    client_data_json: String,
    authenticator_data: String,
    signature: String,
}

/// `POST /api/webauthn/register/finish`'s body.
#[derive(Deserialize)]
struct RegisterFinish {
    #[serde(default)]
    label: String,
    credential_id: String,
    client_data_json: String,
    attestation_object: String,
}

/// The caller's enrolled passkeys (public metadata only) plus whether the
/// ceremony is required for them - the frontend's one probe before rendering
/// either the "confirm with your passkey" flow or the software-signed badge.
async fn webauthn_list(State(ctx): State<Arc<Ctx>>, jar: CookieJar) -> Response {
    let session = match guard(&ctx, &jar) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let store = match ctx.passkeys.as_ref() {
        Ok(s) => s,
        Err(e) => return store_unavailable(e),
    };
    let keys: Vec<_> = store
        .for_user(&session.user)
        .into_iter()
        .map(|k| {
            json!({
                "credential_id": k.credential_id,
                "label": k.label,
                "created_at": k.created_at,
            })
        })
        .collect();
    Json(json!({ "passkeys": keys, "webauthn_required": !keys.is_empty() })).into_response()
}

/// Mint a registration challenge and return the exact
/// `PublicKeyCredentialCreationOptions` the frontend spreads into
/// `navigator.credentials.create` (decoding challenge/user.id browser-side).
async fn webauthn_register_start(State(ctx): State<Arc<Ctx>>, jar: CookieJar) -> Response {
    let session = match guard(&ctx, &jar) {
        Ok(s) => s,
        Err(r) => return r,
    };
    if let Err(e) = ctx.passkeys.as_ref() {
        return store_unavailable(e);
    }
    let challenge = match ctx
        .webauthn_pending
        .mint(&session.user, webauthn::Purpose::Register)
    {
        Ok(c) => c,
        Err(e) => return ceremony_refused(&e),
    };
    Json(json!({
        "challenge": challenge,
        "rp": { "id": ctx.webauthn_rp.rp_id, "name": "Genaryx" },
        "user": {
            "id": B64URL.encode(session.user.as_bytes()),
            "name": session.user,
            "displayName": session.user,
        },
        "pubKeyCredParams": [ { "type": "public-key", "alg": -7 } ],
        "timeout": 120000,
        "attestation": "none",
        "authenticatorSelection": { "userVerification": "preferred" }
    }))
    .into_response()
}

/// Verify a `navigator.credentials.create` response and enroll the passkey.
async fn webauthn_register_finish(
    State(ctx): State<Arc<Ctx>>,
    jar: CookieJar,
    Json(body): Json<RegisterFinish>,
) -> Response {
    let session = match guard(&ctx, &jar) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let store = match ctx.passkeys.as_ref() {
        Ok(s) => s,
        Err(e) => return store_unavailable(e),
    };
    let Ok(client_data) = B64URL.decode(&body.client_data_json) else {
        return bad_request("client_data_json is not base64url");
    };
    let Ok(attestation) = B64URL.decode(&body.attestation_object) else {
        return bad_request("attestation_object is not base64url");
    };
    let Some(challenge) = challenge_of(&client_data) else {
        return bad_request("clientDataJSON carries no challenge");
    };
    match ctx.webauthn_pending.take(&session.user, &challenge) {
        Ok(webauthn::Purpose::Register) => {}
        Ok(_) => return ceremony_refused(&webauthn::WebAuthnError::Mismatch("ceremony purpose")),
        Err(e) => return ceremony_refused(&e),
    }
    let verified = match webauthn::verify_registration(
        &ctx.webauthn_rp,
        &challenge,
        &client_data,
        &attestation,
    ) {
        Ok(v) => v,
        Err(e) => return ceremony_refused(&e),
    };
    if verified.credential_id != body.credential_id {
        return ceremony_refused(&webauthn::WebAuthnError::Mismatch("credential id"));
    }
    let record = webauthn::PasskeyRecord {
        credential_id: verified.credential_id,
        public_key_x963: verified.public_key_x963,
        sign_count: verified.sign_count,
        created_at: chrono::Utc::now().to_rfc3339(),
        label: if body.label.trim().is_empty() {
            "passkey".into()
        } else {
            body.label.trim().to_string()
        },
    };
    let credential_id = record.credential_id.clone();
    if let Err(e) = store.add(&session.user, record) {
        return ceremony_refused(&e);
    }
    tracing::info!(
        user = %session.user, credential = %credential_id,
        user_verified = verified.user_verified,
        "webauthn passkey enrolled"
    );
    Json(json!({ "enrolled": true, "credential_id": credential_id })).into_response()
}

/// Mint a per-action challenge bound to the exact command + args the operator
/// is about to confirm, and say which credentials may answer it.
async fn webauthn_action_start(
    State(ctx): State<Arc<Ctx>>,
    jar: CookieJar,
    Json(body): Json<ActionStart>,
) -> Response {
    let session = match guard(&ctx, &jar) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let store = match ctx.passkeys.as_ref() {
        Ok(s) => s,
        Err(e) => return store_unavailable(e),
    };
    let keys = store.for_user(&session.user);
    if keys.is_empty() {
        return bad_request("no passkey enrolled; enroll one before starting an action ceremony");
    }
    if !SENSITIVE_COMMANDS.contains(&body.command.as_str()) {
        return bad_request("this command carries no webauthn ceremony");
    }
    let args_sha256 = genaryx_signing::body_sha256_hex(canonical_args(&body.args).as_bytes());
    let challenge = match ctx.webauthn_pending.mint(
        &session.user,
        webauthn::Purpose::Action {
            command: body.command.clone(),
            args_sha256,
        },
    ) {
        Ok(c) => c,
        Err(e) => return ceremony_refused(&e),
    };
    Json(json!({
        "challenge": challenge,
        "rp_id": ctx.webauthn_rp.rp_id,
        "timeout": 120000,
        "user_verification": "preferred",
        "allow_credentials": keys
            .iter()
            .map(|k| json!({ "type": "public-key", "id": k.credential_id }))
            .collect::<Vec<_>>(),
    }))
    .into_response()
}

/// The per-action gate. Runs AFTER the role gate and BEFORE dispatch, only
/// for [`SENSITIVE_COMMANDS`]. Fail-closed everywhere: a corrupt store, a
/// stale/foreign/replayed challenge, a binding mismatch, or a bad signature
/// all refuse. The only pass-throughs are a verified assertion, or a caller
/// with no passkey enrolled at all - the documented trial fallback, which the
/// next increment records as "software-signed" in the command journal.
// A ready-made `Response` IS the refusal here, same shape and same rationale
// as `guard` above - see its comment on why boxing it would be worse.
#[allow(clippy::result_large_err)]
fn webauthn_gate(
    ctx: &Arc<Ctx>,
    session: &auth::SessionInfo,
    name: &str,
    args: &Value,
    headers: &HeaderMap,
) -> Result<Option<genaryx_api::console_actor::ConsoleSignature>, Response> {
    let store = match ctx.passkeys.as_ref() {
        Ok(s) => s,
        Err(e) => return Err(store_unavailable(e)),
    };
    if !store.has_any(&session.user) {
        // No override: the plane's own transport-signing fields stay, which
        // on this shell honestly read "software-signed".
        tracing::info!(
            user = %session.user, command = %name,
            "webauthn: no passkey enrolled; software-signed fallback"
        );
        return Ok(None);
    }
    let Some(header) = headers.get("x-genaryx-webauthn") else {
        return Err((
            StatusCode::PRECONDITION_REQUIRED,
            Json(json!({
                "error": "a webauthn assertion is required for this command",
                "webauthn": "required",
            })),
        )
            .into_response());
    };
    let envelope: AssertionEnvelope = match header
        .to_str()
        .ok()
        .and_then(|s| B64URL.decode(s).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
    {
        Some(e) => e,
        None => return Err(bad_request("x-genaryx-webauthn is not base64url(JSON)")),
    };
    let Ok(client_data) = B64URL.decode(&envelope.client_data_json) else {
        return Err(bad_request("client_data_json is not base64url"));
    };
    let Ok(auth_data) = B64URL.decode(&envelope.authenticator_data) else {
        return Err(bad_request("authenticator_data is not base64url"));
    };
    let Ok(signature) = B64URL.decode(&envelope.signature) else {
        return Err(bad_request("signature is not base64url"));
    };
    let Some(challenge) = challenge_of(&client_data) else {
        return Err(bad_request("clientDataJSON carries no challenge"));
    };
    let purpose = ctx
        .webauthn_pending
        .take(&session.user, &challenge)
        .map_err(|e| ceremony_refused(&e))?;
    let (bound_command, bound_args) = match purpose {
        webauthn::Purpose::Action {
            command,
            args_sha256,
        } => (command, args_sha256),
        webauthn::Purpose::Register => {
            return Err(ceremony_refused(&webauthn::WebAuthnError::Mismatch(
                "ceremony purpose",
            )));
        }
    };
    if bound_command != name {
        return Err(ceremony_refused(&webauthn::WebAuthnError::Mismatch(
            "bound command",
        )));
    }
    if bound_args != genaryx_signing::body_sha256_hex(canonical_args(args).as_bytes()) {
        return Err(ceremony_refused(&webauthn::WebAuthnError::Mismatch(
            "bound arguments",
        )));
    }
    let record = match store
        .for_user(&session.user)
        .into_iter()
        .find(|k| k.credential_id == envelope.credential_id)
    {
        Some(r) => r,
        None => return Err(ceremony_refused(&webauthn::WebAuthnError::WrongUser)),
    };
    let verified = webauthn::verify_assertion(
        &ctx.webauthn_rp,
        &challenge,
        &record,
        &client_data,
        &auth_data,
        &signature,
    )
    .map_err(|e| ceremony_refused(&e))?;
    if let Err(e) =
        store.update_sign_count(&session.user, &record.credential_id, verified.sign_count)
    {
        // The signature already verified; a failed counter persist weakens
        // only the clone heuristic, so say so loudly rather than refusing an
        // action the operator just physically confirmed.
        tracing::warn!(error = %e, "webauthn: could not persist sign count");
    }
    tracing::info!(
        user = %session.user, command = %name, credential = %record.credential_id,
        user_verified = verified.user_verified,
        "webauthn assertion verified"
    );
    Ok(Some(genaryx_api::console_actor::ConsoleSignature {
        alg: "webauthn-es256".to_string(),
        fpr: record.credential_id.clone(),
    }))
}

/// The canonical serialization both `action/start` and the gate hash:
/// `serde_json::Value` maps are key-sorted, so the same parsed args always
/// serialize to the same bytes and the binding cannot drift on key order.
fn canonical_args(args: &Value) -> String {
    args.to_string()
}

/// Pull the echoed challenge string out of a raw clientDataJSON.
fn challenge_of(client_data_json: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(client_data_json)
        .ok()?
        .get("challenge")?
        .as_str()
        .map(str::to_string)
}

fn store_unavailable(why: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error": format!("passkey store unavailable: {why}")})),
    )
        .into_response()
}

fn bad_request(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": msg}))).into_response()
}

fn ceremony_refused(e: &webauthn::WebAuthnError) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({"error": format!("webauthn: {e}")})),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// commands and live events
// ---------------------------------------------------------------------------

async fn command(
    State(ctx): State<Arc<Ctx>>,
    jar: CookieJar,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Response {
    let session = match guard(&ctx, &jar) {
        Ok(s) => s,
        Err(r) => return r,
    };

    // Role gate (docs/CONSOLE-IDP.md): refuse before dispatch if the caller is
    // below the command's required role. One place, not per-plane.
    let need = roles::required_role(&name);
    if session.role < need {
        tracing::warn!(user = %session.user, role = %session.role.as_str(), command = %name, "role gate refused");
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": format!("role {} required", need.as_str())})),
        )
            .into_response();
    }

    let args = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));

    // Per-action WebAuthn ceremony (docs/CONSOLE-IDP.md, B3/2): the
    // privileged few need a fresh hardware assertion on top of the role,
    // AFTER the role gate and BEFORE dispatch. Signing in gets you the
    // console; it does not get you the kill. A verified ceremony rides into
    // the command journal as sig_alg/sig_fpr (the assertion's algorithm +
    // credential id); the no-passkey trial fallback keeps the plane's own
    // honest "software-signed" fields.
    let webauthn_signature = if SENSITIVE_COMMANDS.contains(&name.as_str()) {
        match webauthn_gate(&ctx, &session, &name, &args, &headers) {
            Ok(sig) => sig,
            Err(refusal) => return refusal,
        }
    } else {
        None
    };
    // Attribute any journaled mutation to the signed-in human, not the OS
    // account running this process, and carry the WebAuthn ceremony's
    // signature fields alongside (both genaryx_api::console_actor task-locals;
    // a None signature is simply no override).
    let result = genaryx_api::console_actor::with_actor(
        Some(session.user.clone()),
        genaryx_api::console_actor::with_signature(
            webauthn_signature,
            dispatch::dispatch(&ctx, &name, args),
        ),
    )
    .await;
    match result {
        Ok(r) => r,
        Err(r) => r,
    }
}

/// The live bus, as Server-Sent Events, plus (multiplexed onto the SAME
/// connection) any live remote-tail lines.
///
/// The browser receives the bus feed as an `EventSource` (the removed
/// desktop shell delivered the same feed as a Tauri event, with an identical
/// payload and cadence, so the panels that redraw on a new event needed no
/// change when the web shell became the only one). The bus rides the
/// `bus`-named SSE event, unchanged from before the Remote panel moved here;
/// a remote tail's lines/ended marker ride their own `remote:tail-line`/
/// `remote:tail-ended` named events instead of being folded into the `bus`
/// shape they do not fit (see `ctx::RemoteTailEvent`'s own doc comment) - one
/// `EventSource` in the browser, one `addEventListener` per name.
async fn events(
    State(ctx): State<Arc<Ctx>>,
    jar: CookieJar,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, Response> {
    guard(&ctx, &jar)?;

    let bus_rx = ctx.events.subscribe();
    let bus_stream = futures_util::stream::unfold(bus_rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let data = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                    return Some((Ok(Event::default().event("bus").data(data)), rx));
                }
                // Lagged: this reader fell behind the bounded channel. Keep
                // the stream open and carry on from the newest events rather
                // than tearing the page's connection down.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!(dropped = n, "SSE reader lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    let tail_rx = ctx.remote_tail.subscribe();
    let tail_stream = futures_util::stream::unfold(tail_rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let (name, data) = match &ev {
                        ctx::RemoteTailEvent::Line(line) => (
                            "remote:tail-line",
                            serde_json::to_string(line).unwrap_or_else(|_| "{}".into()),
                        ),
                        ctx::RemoteTailEvent::Ended(ended) => (
                            "remote:tail-ended",
                            serde_json::to_string(ended).unwrap_or_else(|_| "{}".into()),
                        ),
                    };
                    return Some((Ok(Event::default().event(name).data(data)), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!(dropped = n, "remote-tail SSE reader lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    let stream = futures_util::stream::select(bus_stream, tail_stream);
    Ok(Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}

// ---------------------------------------------------------------------------
// tests: the auth + role gate on /api/command (docs/CONSOLE-IDP.md, B3/1)
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Method;
    use crate::roles::Role;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// A hermetic Ctx: a temp state dir, no UI, an unavailable bus, and every
    /// plane left in its pending state (never `resolve()`d, so no network and
    /// no background tasks). Enough to exercise the auth + role gate, which
    /// runs entirely BEFORE any plane is touched.
    fn test_ctx() -> Arc<Ctx> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "gw-gate-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let cfg = Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            state_dir: dir,
            ui_dir: None,
            secure_cookies: false,
        };
        let (events_tx, _) = tokio::sync::broadcast::channel(512);
        let bus = genaryx_api::bus::AppState {
            events_dir: None,
            mode: genaryx_api::bus::BusMode::Unavailable {
                reason: "test".into(),
            },
        };
        Arc::new(Ctx::bootstrap(cfg, bus, events_tx))
    }

    async fn post_command(ctx: &Arc<Ctx>, name: &str, cookie: Option<&str>) -> StatusCode {
        let mut req = Request::builder()
            .method("POST")
            .uri(format!("/api/command/{name}"))
            .header("content-type", "application/json");
        if let Some(c) = cookie {
            req = req.header("cookie", format!("{}={}", auth::COOKIE, c));
        }
        let req = req.body(Body::from("{}")).unwrap();
        app(Arc::clone(ctx)).oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn a_command_without_a_session_is_401() {
        let ctx = test_ctx();
        assert_eq!(
            post_command(&ctx, "money_status", None).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn a_viewer_is_403_on_an_admin_command_with_the_role_message() {
        let ctx = test_ctx();
        let sid = ctx.sessions.create("carol", Role::Viewer, Method::Oidc);
        // The router, driven directly, so we can read the body too.
        let req = Request::builder()
            .method("POST")
            .uri("/api/command/money_kill_run")
            .header("content-type", "application/json")
            .header("cookie", format!("{}={}", auth::COOKIE, sid))
            .body(Body::from("{}"))
            .unwrap();
        let resp = app(Arc::clone(&ctx)).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "role admin required");
    }

    #[tokio::test]
    async fn a_viewer_passes_the_gate_on_a_read() {
        let ctx = test_ctx();
        let sid = ctx.sessions.create("carol", Role::Viewer, Method::Oidc);
        // money_status is a viewer command: the gate must let it through. The
        // pending plane answers with its own status (never 401/403).
        let status = post_command(&ctx, "money_status", Some(&sid)).await;
        assert_ne!(status, StatusCode::UNAUTHORIZED);
        assert_ne!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn an_admin_passes_the_gate_on_an_admin_command() {
        let ctx = test_ctx();
        let sid = ctx.sessions.create("alice", Role::Admin, Method::Local);
        // The gate lets an admin through to dispatch; dispatch then answers on
        // its own (no cloud in this test), but it is never the 403 gate.
        let status = post_command(&ctx, "money_kill_run", Some(&sid)).await;
        assert_ne!(status, StatusCode::FORBIDDEN);
        assert_ne!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn an_unknown_command_needs_admin_then_404s_not_403_for_admin() {
        let ctx = test_ctx();
        // A viewer is stopped by the fail-closed gate (unknown => admin).
        let sid_v = ctx.sessions.create("carol", Role::Viewer, Method::Oidc);
        assert_eq!(
            post_command(&ctx, "no_such_command", Some(&sid_v)).await,
            StatusCode::FORBIDDEN
        );
        // An admin passes the gate; dispatch then reports the unknown name.
        let sid_a = ctx.sessions.create("alice", Role::Admin, Method::Local);
        assert_ne!(
            post_command(&ctx, "no_such_command", Some(&sid_a)).await,
            StatusCode::FORBIDDEN
        );
    }

    // -- the per-action WebAuthn gate (docs/CONSOLE-IDP.md, B3/2) ------------

    use crate::webauthn::test_support;

    /// POST a command with an optional `x-genaryx-webauthn` header and body.
    async fn post_command_with(
        ctx: &Arc<Ctx>,
        name: &str,
        cookie: &str,
        assertion: Option<&str>,
        body: &str,
    ) -> StatusCode {
        let mut req = Request::builder()
            .method("POST")
            .uri(format!("/api/command/{name}"))
            .header("content-type", "application/json")
            .header("cookie", format!("{}={}", auth::COOKIE, cookie));
        if let Some(a) = assertion {
            req = req.header("x-genaryx-webauthn", a);
        }
        let req = req.body(Body::from(body.to_string())).unwrap();
        app(Arc::clone(ctx)).oneshot(req).await.unwrap().status()
    }

    /// Drive `POST /api/webauthn/action/start` through the real router and
    /// return the minted challenge.
    async fn start_action(ctx: &Arc<Ctx>, cookie: &str, command: &str, args: &str) -> String {
        let req = Request::builder()
            .method("POST")
            .uri("/api/webauthn/action/start")
            .header("content-type", "application/json")
            .header("cookie", format!("{}={}", auth::COOKIE, cookie))
            .body(Body::from(format!(
                "{{\"command\":\"{command}\",\"args\":{args}}}"
            )))
            .unwrap();
        let resp = app(Arc::clone(ctx)).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "action/start must succeed");
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        v["challenge"].as_str().unwrap().to_string()
    }

    /// Build the browser-side assertion header for `challenge`, signed by the
    /// test authenticator.
    fn assertion_header(ctx: &Arc<Ctx>, challenge: &str) -> String {
        let s = test_support::signer();
        let cd = test_support::client_data("webauthn.get", challenge, &ctx.webauthn_rp.origin);
        let ad = test_support::auth_data(&ctx.webauthn_rp.rp_id, 0x01, 1, None);
        let sig = test_support::assert_sign(&s, &ad, &cd);
        let envelope = json!({
            "credential_id": test_support::enrolled(&s, 0).credential_id,
            "client_data_json": B64URL.encode(&cd),
            "authenticator_data": B64URL.encode(&ad),
            "signature": B64URL.encode(&sig),
        });
        B64URL.encode(envelope.to_string())
    }

    fn enroll_test_passkey(ctx: &Arc<Ctx>, user: &str) {
        let s = test_support::signer();
        ctx.passkeys
            .as_ref()
            .expect("test store opens")
            .add(user, test_support::enrolled(&s, 0))
            .unwrap();
    }

    #[tokio::test]
    async fn a_sensitive_command_with_a_passkey_and_no_assertion_is_428() {
        let ctx = test_ctx();
        let sid = ctx.sessions.create("alice", Role::Admin, Method::Oidc);
        enroll_test_passkey(&ctx, "alice");
        assert_eq!(
            post_command_with(&ctx, "money_kill_run", &sid, None, "{}").await,
            StatusCode::PRECONDITION_REQUIRED
        );
    }

    #[tokio::test]
    async fn a_sensitive_command_with_no_passkey_falls_back_software_signed() {
        let ctx = test_ctx();
        let sid = ctx.sessions.create("alice", Role::Admin, Method::Local);
        // No enrollment: the gate lets dispatch answer (trial fallback).
        let status = post_command_with(&ctx, "money_kill_run", &sid, None, "{}").await;
        assert_ne!(status, StatusCode::PRECONDITION_REQUIRED);
        assert_ne!(status, StatusCode::FORBIDDEN);
        assert_ne!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_read_command_never_asks_for_an_assertion() {
        let ctx = test_ctx();
        let sid = ctx.sessions.create("alice", Role::Admin, Method::Oidc);
        enroll_test_passkey(&ctx, "alice");
        assert_ne!(
            post_command_with(&ctx, "money_status", &sid, None, "{}").await,
            StatusCode::PRECONDITION_REQUIRED
        );
    }

    #[tokio::test]
    async fn the_role_gate_still_runs_before_the_webauthn_gate() {
        let ctx = test_ctx();
        let sid = ctx.sessions.create("carol", Role::Viewer, Method::Oidc);
        enroll_test_passkey(&ctx, "carol");
        // A viewer gets the ROLE refusal, never a webauthn 428: privilege
        // first, ceremony second.
        assert_eq!(
            post_command_with(&ctx, "money_kill_run", &sid, None, "{}").await,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn the_full_ceremony_passes_and_its_challenge_is_one_shot() {
        let ctx = test_ctx();
        let sid = ctx.sessions.create("alice", Role::Admin, Method::Oidc);
        enroll_test_passkey(&ctx, "alice");

        let challenge = start_action(&ctx, &sid, "money_kill_run", "{}").await;
        let header = assertion_header(&ctx, &challenge);

        // A genuine assertion passes the gate; the pending plane answers
        // downstream (whatever it answers, it is not the gate's refusals).
        let status = post_command_with(&ctx, "money_kill_run", &sid, Some(&header), "{}").await;
        assert_ne!(status, StatusCode::PRECONDITION_REQUIRED);
        assert_ne!(status, StatusCode::FORBIDDEN);
        assert_ne!(status, StatusCode::UNAUTHORIZED);

        // Replaying the SAME assertion must refuse: the challenge was
        // consumed by the first dispatch.
        assert_eq!(
            post_command_with(&ctx, "money_kill_run", &sid, Some(&header), "{}").await,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn an_assertion_bound_to_another_command_or_args_is_refused() {
        let ctx = test_ctx();
        let sid = ctx.sessions.create("alice", Role::Admin, Method::Oidc);
        enroll_test_passkey(&ctx, "alice");

        // Bound to money_set_budget, replayed against money_kill_run.
        let challenge = start_action(&ctx, &sid, "money_set_budget", "{}").await;
        let header = assertion_header(&ctx, &challenge);
        assert_eq!(
            post_command_with(&ctx, "money_kill_run", &sid, Some(&header), "{}").await,
            StatusCode::FORBIDDEN
        );

        // Bound to one args shape, dispatched with another.
        let challenge = start_action(&ctx, &sid, "money_kill_run", "{\"run_id\":\"a\"}").await;
        let header = assertion_header(&ctx, &challenge);
        assert_eq!(
            post_command_with(
                &ctx,
                "money_kill_run",
                &sid,
                Some(&header),
                "{\"run_id\":\"b\"}"
            )
            .await,
            StatusCode::FORBIDDEN
        );
    }
}
