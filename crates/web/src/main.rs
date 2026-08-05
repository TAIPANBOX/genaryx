//! `genaryx-web`: the Genaryx console, served over HTTP from inside the
//! customer's own perimeter.
//!
//! This process runs beside the customer's stack, on their box. It reads the
//! same `~/.taipan/environments/` descriptors the former desktop console once
//! read, and answers every request by calling `genaryx-api` directly - the
//! same functions the Tauri shell used to wrap. No run, spend figure,
//! identity or policy decision leaves the customer's network to render this
//! UI, and it-rat.com has no route to it at all: the site documents the
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
use std::io::IsTerminal;
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
    /// Issue the operator's own WireGuard device and print it here.
    ///
    /// This exists because of an ordering nobody can get out of: the console
    /// is reachable only through the tunnel, and the tunnel needs a config the
    /// console issues. Told to "issue yourself a device from the console", an
    /// operator with no tunnel yet has nowhere to click. Somebody has to hand
    /// out the FIRST config from outside the browser, and the only channel
    /// that exists before a tunnel does is the one the operator used to
    /// install this in the first place.
    ///
    /// The `.conf` goes to STDOUT and everything else to STDERR, so
    /// `... issue-device > box.conf` saves the file and still shows the QR.
    IssueDevice {
        /// Force ANSI colour on the QR.
        ///
        /// Needed because this process usually cannot tell. Run through
        /// `kubectl exec` without a TTY, its stderr is a pipe whichever way
        /// the operator's own terminal is set up, so the automatic check says
        /// "not a terminal" exactly when a human is watching. Colour is what
        /// makes the QR scannable on a dark background (render_qr_terminal),
        /// so the caller that knows has to say.
        #[arg(long, conflicts_with = "no_color")]
        color: bool,
        /// Force it off, for output that is being captured.
        #[arg(long)]
        no_color: bool,
        /// Skip the QR entirely. For a laptop, where the `.conf` is imported
        /// as a file and a QR is decoration.
        #[arg(long)]
        no_qr: bool,
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
        Cmd::IssueDevice {
            color,
            no_color,
            no_qr,
        } => {
            // No bus: this runs as a one-shot process with no console actor
            // around it, so there is no signature or operator identity to
            // attribute the action to. Passing None makes the journal record
            // it honestly as unattributed rather than inventing an author.
            let issued = match genaryx_api::remote::wg_operator::operator_wg_config(None).await {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("genaryx-web: could not issue a device: {e}");
                    std::process::exit(1);
                }
            };
            if !no_qr {
                // Explicit beats the guess in both directions; the guess is
                // only right when a human runs this directly on the box.
                let ansi = color || (!no_color && std::io::stderr().is_terminal());
                match genaryx_api::remote::wg_operator::render_qr_terminal(&issued.conf, ansi) {
                    Ok(qr) => eprintln!("\n{qr}"),
                    // A QR is a convenience; the config above is the thing.
                    // Failing the whole command over the decoration would mean
                    // minting a peer and then refusing to hand over its config.
                    Err(e) => eprintln!("genaryx-web: no QR ({e}); the config below still works"),
                }
            }
            eprintln!(
                "\n  device      {}\n  address     {}\n  endpoint    {}\n  console     {}\n",
                issued.peer_public_key,
                issued.client_ip,
                issued.endpoint,
                issued.console_tunnel_url,
            );
            eprintln!(
                "  The config on stdout carries this device's PRIVATE key. It is not\n  \
                 stored here and cannot be shown again: issue another device if it is\n  \
                 lost, and revoke {} from the console.\n",
                issued.peer_public_key,
            );
            print!("{}", issued.conf);
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
                require_passkey: Config::require_passkey_from_env(),
            };
            serve(cfg).await;
        }
    }
}

async fn serve(cfg: Config) {
    cfg.warn_if_exposed();
    cfg.announce_passkey_policy();
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
            source_events_dir: Some(b.source_events_dir),
            mode: b.mode,
        },
        Err(e) => {
            // Fail-closed, exactly as the desktop shell does: a bus that will
            // not open degrades the Bus Explorer, it never stops the console
            // serving the planes that are fine.
            tracing::error!(error = %e, "bus startup failed; the Bus Explorer will be empty");
            genaryx_api::bus::AppState {
                events_dir: None,
                source_events_dir: None,
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
        .route("/webauthn/passkeys/remove", post(webauthn_remove))
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

/// The two ceremony names that are NOT dispatchable commands: each names an
/// endpoint of the passkey lifecycle rather than something
/// `POST /api/command/<name>` could ever run, and `action/start` mints a
/// challenge bound to one exactly as it does for a sensitive command.
///
/// Deliberately kept out of [`SENSITIVE_COMMANDS`], which is the DISPATCH
/// list the command chokepoint reads: a name that cannot be dispatched has no
/// business in a list of commands.
const REMOVE_PASSKEY_CEREMONY: &str = "webauthn_remove_passkey";
const ENROLL_PASSKEY_CEREMONY: &str = "webauthn_enroll_passkey";

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

/// `POST /api/webauthn/register/start`'s body. Optional in the request (an
/// enrollment that proves itself with an assertion sends no body at all), so
/// the handler takes it as `Option<Json<_>>`.
#[derive(Deserialize, Default)]
struct RegisterStart {
    /// The local operator account's password, the factor for a FIRST
    /// enrollment (see [`webauthn_register_start`]).
    #[serde(default)]
    operator_password: Option<String>,
}

/// `POST /api/webauthn/passkeys/remove`'s body.
#[derive(Deserialize)]
struct RemovePasskey {
    /// The enrolled credential to drop, base64url, exactly as
    /// `GET /api/webauthn/passkeys` reports it.
    credential_id: String,
    /// The local operator account's password (`genaryx-web set-password`).
    /// Required to remove the LAST enrolled passkey, and accepted in place of
    /// an assertion for any other - see [`webauthn_remove`] for why.
    #[serde(default)]
    operator_password: Option<String>,
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
    Json(json!({
        "passkeys": keys,
        // "will this caller's next sensitive command need an assertion", which
        // is simply "have they enrolled anything".
        "webauthn_required": !keys.is_empty(),
        // "does this box refuse a sensitive command from a caller with none",
        // so the panel can say that BEFORE the operator finds out by being
        // refused. Independent of the line above: the policy is the box's,
        // enrollment is the caller's.
        "policy_requires_passkey": ctx.cfg.require_passkey,
    }))
    .into_response()
}

/// Mint a registration challenge and return the exact
/// `PublicKeyCredentialCreationOptions` the frontend spreads into
/// `navigator.credentials.create` (decoding challenge/user.id browser-side).
///
/// Enrolling used to need nothing but a live session cookie, which is the one
/// thing this whole ceremony exists to distrust: an attacker holding a stolen
/// admin session on a box where nobody had enrolled yet could add their own
/// authenticator and then satisfy every per-action ceremony from it. "A
/// stolen session cannot pull the switch" was not true on that path, and it
/// was the COMMON path, because the software-signed fallback meant most boxes
/// had nobody enrolled.
///
/// So a challenge is minted only against a factor the session does not carry:
///
/// - **Nothing enrolled yet**: the operator password, the box's break-glass
///   credential (`genaryx-web set-password`). It is also what the recovery
///   path after a lost key lands on, so the two ends meet.
/// - **Something enrolled already**: a fresh assertion from one of those
///   keys. While a working authenticator exists, the phishing-resistant proof
///   is the one to demand, and it means the password alone cannot QUIETLY add
///   a second authenticator beside the operator's own: it would first have to
///   remove theirs ([`webauthn_remove`]), which is visible in this very list.
///
/// The factor gates the START, so no registration ceremony can even begin
/// without it; `register/finish` then rides on the one-shot, user-bound
/// challenge this mints, exactly as before.
async fn webauthn_register_start(
    State(ctx): State<Arc<Ctx>>,
    jar: CookieJar,
    headers: HeaderMap,
    body: Option<Json<RegisterStart>>,
) -> Response {
    let session = match guard(&ctx, &jar) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let store = match ctx.passkeys.as_ref() {
        Ok(s) => s,
        Err(e) => return store_unavailable(e),
    };
    let Json(body) = body.unwrap_or_default();

    if store.has_any(&session.user) {
        let Some(header) = headers.get("x-genaryx-webauthn") else {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": "you already have an enrolled passkey: confirm with it before \
                              enrolling another, so a session on its own cannot add one",
                    "webauthn": "assertion_required",
                })),
            )
                .into_response();
        };
        if let Err(refusal) = verify_assertion_header(
            &ctx,
            &session,
            store,
            ENROLL_PASSKEY_CEREMONY,
            &json!({}),
            header,
        ) {
            return refusal;
        }
    } else {
        let Some(password) = body.operator_password.as_deref() else {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": "enrolling the first passkey needs the operator password (the one \
                              `genaryx-web set-password` set): a session alone is what the \
                              ceremony exists to defend against, so it cannot mint the key that \
                              would satisfy it",
                    "webauthn": "password_required",
                })),
            )
                .into_response();
        };
        if let Err(refusal) = operator_password_ok(&ctx, password) {
            return refusal;
        }
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

/// Remove one enrolled passkey.
///
/// An enrolled passkey used to be permanent: `PasskeyStore` could add and
/// count, and nothing could take one away, so an operator whose only
/// authenticator was lost or wiped was 428'd out of every sensitive command
/// with no way back except hand-editing `passkeys.json` on the box - shell
/// access to the box you reach THROUGH this console, as the emergency path
/// for the emergency console.
///
/// The authority to remove is deliberately never the session, since the
/// session is the exact thing the ceremony defends against:
///
/// - **While another passkey remains**: a fresh, one-shot WebAuthn assertion
///   bound to this exact credential id, from any of the caller's own enrolled
///   keys. This is the ordinary "I lost one of my two" flow, and it demands
///   the strongest proof that is still available.
/// - **The last one**: the operator password (`genaryx-web set-password`),
///   and only that. An assertion from the key being removed is a fine proof
///   of possession, and still not enough: this is the removal that takes the
///   whole box back to session-only (or, with `GENARYX_WEB_REQUIRE_PASSKEY`,
///   to refusing outright), so it is the box owner's decision. It is also the
///   only rule that works in the case that matters, a lost key, where no
///   assertion can be produced at all.
/// - The password is accepted for a non-last removal too. That adds no
///   authority it did not already have (holding it, one can remove the others
///   one at a time and then the last), and it removes the one lockout the
///   assertion-only rule would create: every enrolled key lost at once.
///
/// So the recovery story is: operator password removes what is left, and the
/// first enrollment after that needs the same password again
/// ([`webauthn_register_start`]). A stolen session alone gets neither.
async fn webauthn_remove(
    State(ctx): State<Arc<Ctx>>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(body): Json<RemovePasskey>,
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
    if !keys.iter().any(|k| k.credential_id == body.credential_id) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("webauthn: {}", webauthn::WebAuthnError::UnknownCredential)})),
        )
            .into_response();
    }
    let is_last = keys.len() == 1;

    // A supplied password is answered on its own terms: wrong is a refusal,
    // never a quiet fall-through to the assertion path.
    let password_ok = match body.operator_password.as_deref() {
        Some(pw) => match operator_password_ok(&ctx, pw) {
            Ok(()) => true,
            Err(refusal) => return refusal,
        },
        None => false,
    };

    if is_last && !password_ok {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "this is the last enrolled passkey: removing it needs the operator \
                          password (the one `genaryx-web set-password` set), because it is what \
                          takes this console back to session-only",
                "webauthn": "password_required",
            })),
        )
            .into_response();
    }
    if !password_ok {
        let Some(header) = headers.get("x-genaryx-webauthn") else {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": "removing a passkey needs a fresh confirmation from an enrolled \
                              passkey, or the operator password: a session alone cannot take the \
                              ceremony away",
                    "webauthn": "assertion_required",
                })),
            )
                .into_response();
        };
        let bound = json!({ "credential_id": body.credential_id });
        if let Err(refusal) = verify_assertion_header(
            &ctx,
            &session,
            store,
            REMOVE_PASSKEY_CEREMONY,
            &bound,
            header,
        ) {
            return refusal;
        }
    }

    let remaining = match store.remove(&session.user, &body.credential_id, password_ok) {
        Ok(n) => n,
        Err(e) => return ceremony_refused(&e),
    };
    tracing::warn!(
        user = %session.user, credential = %body.credential_id, remaining,
        by = if password_ok { "operator password" } else { "passkey assertion" },
        "webauthn passkey removed"
    );
    Json(json!({
        "removed": true,
        "credential_id": body.credential_id,
        "remaining": remaining,
    }))
    .into_response()
}

/// Check the local operator account's password, the box's break-glass factor
/// for the passkey lifecycle (first enrollment, last removal).
///
/// Reuses `auth::verify` untouched, including its deliberate refusal to
/// short-circuit on the username, so a wrong password always costs the same
/// Argon2 verification. The username handed to it is the operator record's
/// OWN: who the caller is was settled by the session, and what is being
/// proven here is knowledge of the box credential, not that the signed-in
/// name (an OIDC `sub`, say) happens to match the local account's.
#[allow(clippy::result_large_err)]
fn operator_password_ok(ctx: &Arc<Ctx>, password: &str) -> Result<(), Response> {
    let op = ctx.operator.read().expect("operator lock").clone();
    let Some(op) = op else {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "this box has no operator account, so there is no password to prove: \
                          run `genaryx-web set-password --username <name>` on the box first",
                "webauthn": "no_operator_account",
            })),
        )
            .into_response());
    };
    if !auth::verify(&op, &op.username, password) {
        tracing::warn!("operator password refused for a passkey lifecycle change");
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "the operator password was not accepted"})),
        )
            .into_response());
    }
    Ok(())
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
    let ceremonial = SENSITIVE_COMMANDS.contains(&body.command.as_str())
        || body.command == REMOVE_PASSKEY_CEREMONY
        || body.command == ENROLL_PASSKEY_CEREMONY;
    if !ceremonial {
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
        // The strict console (`GENARYX_WEB_REQUIRE_PASSKEY`): no enrolled
        // passkey means no ceremony is possible, and an operator who asked for
        // the ceremony to be mandatory asked for the command to be REFUSED
        // rather than quietly run on the session. Say what to do about it: a
        // bare 403 here would read as a role problem, which it is not.
        if ctx.cfg.require_passkey {
            tracing::warn!(
                user = %session.user, command = %name,
                "webauthn: required by configuration and nothing enrolled; refused"
            );
            return Err((
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": format!(
                        "this console requires a passkey for {name} \
                         (GENARYX_WEB_REQUIRE_PASSKEY is on) and you have none enrolled: \
                         enrol one under Session > Passkeys, then run this again"
                    ),
                    "webauthn": "enrollment_required",
                })),
            )
                .into_response());
        }
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
    let record = verify_assertion_header(ctx, session, store, name, args, header)?;
    Ok(Some(genaryx_api::console_actor::ConsoleSignature {
        alg: "webauthn-es256".to_string(),
        fpr: record.credential_id,
    }))
}

/// Verify one `x-genaryx-webauthn` header against a ceremony bound to
/// `bound_to` + `args`, and answer with the enrolled passkey that signed it.
///
/// The cryptographic middle of the gate, shared by everything that demands a
/// fresh confirmation: the sensitive-command gate above, the passkey removal
/// ([`webauthn_remove`]) and the additional enrollment
/// ([`webauthn_register_start`]). Every step is fail-closed, and the challenge
/// is consumed one-shot by the `take` below whatever happens afterwards.
#[allow(clippy::result_large_err)]
fn verify_assertion_header(
    ctx: &Arc<Ctx>,
    session: &auth::SessionInfo,
    store: &webauthn::PasskeyStore,
    bound_to: &str,
    args: &Value,
    header: &axum::http::HeaderValue,
) -> Result<webauthn::PasskeyRecord, Response> {
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
    if bound_command != bound_to {
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
        user = %session.user, bound_to = %bound_to, credential = %record.credential_id,
        user_verified = verified.user_verified,
        "webauthn assertion verified"
    );
    Ok(record)
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

    /// A state directory of this test's own. Named per call because cargo runs
    /// these on parallel threads in one process, and a directory keyed only by
    /// pid would be shared (the passkey store is a FILE, so two tests sharing
    /// one would see each other's enrollments).
    fn test_dir() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "gw-gate-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    /// A hermetic Ctx over `dir`: no UI, an unavailable bus, and every plane
    /// left in its pending state (never `resolve()`d, so no network and no
    /// background tasks). Enough to exercise the auth, role and passkey gates,
    /// which run entirely BEFORE any plane is touched.
    fn ctx_at(dir: PathBuf, require_passkey: bool) -> Arc<Ctx> {
        let cfg = Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            state_dir: dir,
            ui_dir: None,
            secure_cookies: false,
            require_passkey,
        };
        let (events_tx, _) = tokio::sync::broadcast::channel(512);
        let bus = genaryx_api::bus::AppState {
            events_dir: None,
            source_events_dir: None,
            mode: genaryx_api::bus::BusMode::Unavailable {
                reason: "test".into(),
            },
        };
        Arc::new(Ctx::bootstrap(cfg, bus, events_tx))
    }

    fn test_ctx() -> Arc<Ctx> {
        ctx_at(test_dir(), false)
    }

    /// A console configured the strict way (`GENARYX_WEB_REQUIRE_PASSKEY`):
    /// the ceremony is mandatory, and a sensitive command with nobody enrolled
    /// is refused rather than falling back. Set on the resolved `Config` here
    /// rather than through the environment on purpose: the env is
    /// process-global and cargo runs these tests on parallel threads, so an
    /// env-var switch would leak between them.
    fn strict_ctx() -> Arc<Ctx> {
        ctx_at(test_dir(), true)
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

    /// Enroll a SECOND authenticator for `user`, under its own credential id.
    fn enroll_test_passkey_id(ctx: &Arc<Ctx>, user: &str, cred_id: &[u8]) {
        let s = test_support::signer();
        ctx.passkeys
            .as_ref()
            .expect("test store opens")
            .add(user, test_support::enrolled_with_id(&s, cred_id, 0))
            .unwrap();
    }

    /// Like [`assertion_header`], for an authenticator with a chosen
    /// credential id (the removal ceremony has to say WHICH enrolled key
    /// answered, and the removal tests enroll two).
    fn assertion_header_for(ctx: &Arc<Ctx>, challenge: &str, cred_id: &[u8]) -> String {
        let s = test_support::signer();
        let cd = test_support::client_data("webauthn.get", challenge, &ctx.webauthn_rp.origin);
        let ad = test_support::auth_data(&ctx.webauthn_rp.rp_id, 0x01, 1, None);
        let sig = test_support::assert_sign(&s, &ad, &cd);
        let envelope = json!({
            "credential_id": test_support::enrolled_with_id(&s, cred_id, 0).credential_id,
            "client_data_json": B64URL.encode(&cd),
            "authenticator_data": B64URL.encode(&ad),
            "signature": B64URL.encode(&sig),
        });
        B64URL.encode(envelope.to_string())
    }

    /// POST any `/api` route with a session cookie, an optional
    /// `x-genaryx-webauthn` header and a JSON body; return the status AND the
    /// parsed body, because these gates are as much about the message the
    /// operator reads as about the number.
    async fn post_api(
        ctx: &Arc<Ctx>,
        uri: &str,
        cookie: &str,
        assertion: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .header("cookie", format!("{}={}", auth::COOKIE, cookie));
        if let Some(a) = assertion {
            req = req.header("x-genaryx-webauthn", a);
        }
        let resp = app(Arc::clone(ctx))
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, parsed)
    }

    /// Give this box the local operator account whose password is the
    /// break-glass factor for the passkey lifecycle (first enrollment, last
    /// removal). Written through the real `auth::set_operator`, then loaded
    /// into the live `Ctx` exactly as `serve` would at startup.
    fn set_test_operator(ctx: &Arc<Ctx>, password: &str) {
        auth::set_operator(&ctx.cfg.operator_file(), "ops", password).unwrap();
        *ctx.operator.write().expect("operator lock") = auth::load(&ctx.cfg.operator_file());
    }

    fn enrolled_ids(ctx: &Arc<Ctx>, user: &str) -> Vec<String> {
        ctx.passkeys
            .as_ref()
            .expect("test store opens")
            .for_user(user)
            .into_iter()
            .map(|k| k.credential_id)
            .collect()
    }

    // -- removing a passkey (defect 1: enrolled, then unremovable) -----------

    #[tokio::test]
    async fn a_passkey_is_removed_by_a_caller_who_confirms_with_an_enrolled_one() {
        let ctx = test_ctx();
        let sid = ctx.sessions.create("alice", Role::Admin, Method::Oidc);
        enroll_test_passkey(&ctx, "alice"); // cred-1
        enroll_test_passkey_id(&ctx, "alice", b"cred-2");
        let victim = B64URL.encode(b"cred-2");

        let challenge = start_action(
            &ctx,
            &sid,
            "webauthn_remove_passkey",
            &json!({ "credential_id": victim }).to_string(),
        )
        .await;
        let header = assertion_header_for(&ctx, &challenge, b"cred-1");

        let (status, body) = post_api(
            &ctx,
            "/api/webauthn/passkeys/remove",
            &sid,
            Some(&header),
            json!({ "credential_id": victim }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(body["removed"], true);
        assert_eq!(body["remaining"], 1);
        assert_eq!(enrolled_ids(&ctx, "alice"), vec![B64URL.encode(b"cred-1")]);
    }

    #[tokio::test]
    async fn removing_a_passkey_on_a_session_alone_is_refused() {
        let ctx = test_ctx();
        let sid = ctx.sessions.create("alice", Role::Admin, Method::Oidc);
        enroll_test_passkey(&ctx, "alice");
        enroll_test_passkey_id(&ctx, "alice", b"cred-2");
        let victim = B64URL.encode(b"cred-2");

        // Nothing but the cookie: the exact thing the ceremony exists to
        // defend against must not be able to take the ceremony away.
        let (status, body) = post_api(
            &ctx,
            "/api/webauthn/passkeys/remove",
            &sid,
            None,
            json!({ "credential_id": victim }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
        assert_eq!(body["webauthn"], "assertion_required");
        assert_eq!(enrolled_ids(&ctx, "alice").len(), 2);

        // A wrong operator password is no better.
        set_test_operator(&ctx, "correct horse battery");
        let (status, _) = post_api(
            &ctx,
            "/api/webauthn/passkeys/remove",
            &sid,
            None,
            json!({ "credential_id": victim, "operator_password": "not the password" }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(enrolled_ids(&ctx, "alice").len(), 2);

        // Nor is an assertion bound to a DIFFERENT credential than the one
        // being removed: the challenge names what it authorizes.
        let challenge = start_action(
            &ctx,
            &sid,
            "webauthn_remove_passkey",
            &json!({ "credential_id": B64URL.encode(b"cred-1") }).to_string(),
        )
        .await;
        let header = assertion_header_for(&ctx, &challenge, b"cred-1");
        let (status, _) = post_api(
            &ctx,
            "/api/webauthn/passkeys/remove",
            &sid,
            Some(&header),
            json!({ "credential_id": victim }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(enrolled_ids(&ctx, "alice").len(), 2);
    }

    #[tokio::test]
    async fn the_last_passkey_goes_only_to_the_operator_password() {
        let ctx = test_ctx();
        let sid = ctx.sessions.create("alice", Role::Admin, Method::Oidc);
        enroll_test_passkey(&ctx, "alice");
        let only = B64URL.encode(b"cred-1");
        set_test_operator(&ctx, "correct horse battery");

        // An assertion from the key itself is a fine proof of possession and
        // still not enough: this removal is what downgrades the whole box.
        let challenge = start_action(
            &ctx,
            &sid,
            "webauthn_remove_passkey",
            &json!({ "credential_id": only }).to_string(),
        )
        .await;
        let header = assertion_header_for(&ctx, &challenge, b"cred-1");
        let (status, body) = post_api(
            &ctx,
            "/api/webauthn/passkeys/remove",
            &sid,
            Some(&header),
            json!({ "credential_id": only }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
        assert_eq!(body["webauthn"], "password_required");
        assert_eq!(enrolled_ids(&ctx, "alice").len(), 1);

        // The operator password is the recovery path, and it works with no
        // authenticator at all - the case a lost key leaves behind.
        let (status, body) = post_api(
            &ctx,
            "/api/webauthn/passkeys/remove",
            &sid,
            None,
            json!({ "credential_id": only, "operator_password": "correct horse battery" }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(body["remaining"], 0);
        assert!(enrolled_ids(&ctx, "alice").is_empty());
    }

    // -- enrolling a passkey (defect 3: gated by the session it defends) ----

    /// `POST /api/webauthn/register/start` with an optional operator password
    /// and an optional assertion header.
    async fn register_start(
        ctx: &Arc<Ctx>,
        cookie: &str,
        password: Option<&str>,
        assertion: Option<&str>,
    ) -> (StatusCode, Value) {
        let body = match password {
            Some(pw) => json!({ "operator_password": pw }),
            None => json!({}),
        };
        post_api(ctx, "/api/webauthn/register/start", cookie, assertion, body).await
    }

    /// Finish a registration the way the browser would, for `cred_id`.
    async fn register_finish(
        ctx: &Arc<Ctx>,
        cookie: &str,
        challenge: &str,
        cred_id: &[u8],
    ) -> (StatusCode, Value) {
        let (client_data, attestation) = test_support::registration_response(
            &ctx.webauthn_rp.rp_id,
            &ctx.webauthn_rp.origin,
            challenge,
            cred_id,
        );
        post_api(
            ctx,
            "/api/webauthn/register/finish",
            cookie,
            None,
            json!({
                "label": "a test authenticator",
                "credential_id": B64URL.encode(cred_id),
                "client_data_json": B64URL.encode(&client_data),
                "attestation_object": B64URL.encode(&attestation),
            }),
        )
        .await
    }

    #[tokio::test]
    async fn enrolling_a_first_passkey_on_a_session_alone_is_refused() {
        let ctx = test_ctx();
        let sid = ctx.sessions.create("mallory", Role::Admin, Method::Oidc);
        set_test_operator(&ctx, "correct horse battery");

        // A stolen admin session, on a box where nobody has enrolled yet, is
        // exactly the case the ceremony exists for: it must not be able to
        // mint itself an authenticator and then satisfy the ceremony from it.
        let (status, body) = register_start(&ctx, &sid, None, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
        assert_eq!(body["webauthn"], "password_required");
        assert!(body["challenge"].is_null(), "no challenge is minted");

        let (status, _) = register_start(&ctx, &sid, Some("not the password"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(enrolled_ids(&ctx, "mallory").is_empty());
    }

    #[tokio::test]
    async fn a_first_enrollment_takes_the_operator_password() {
        let ctx = test_ctx();
        let sid = ctx.sessions.create("alice", Role::Admin, Method::Local);
        set_test_operator(&ctx, "correct horse battery");

        let (status, body) = register_start(&ctx, &sid, Some("correct horse battery"), None).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        let challenge = body["challenge"].as_str().expect("a challenge").to_string();

        let (status, body) = register_finish(&ctx, &sid, &challenge, b"cred-1").await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(body["enrolled"], true);
        assert_eq!(enrolled_ids(&ctx, "alice"), vec![B64URL.encode(b"cred-1")]);
    }

    #[tokio::test]
    async fn an_additional_enrollment_takes_an_assertion_from_an_enrolled_passkey() {
        let ctx = test_ctx();
        let sid = ctx.sessions.create("alice", Role::Admin, Method::Local);
        set_test_operator(&ctx, "correct horse battery");
        enroll_test_passkey(&ctx, "alice"); // cred-1

        // With a passkey already enrolled, the password is no longer the
        // factor: the strongest proof available is a touch of the key that is
        // already protecting this account, so that is what is demanded.
        let (status, body) = register_start(&ctx, &sid, Some("correct horse battery"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
        assert_eq!(body["webauthn"], "assertion_required");

        let challenge = start_action(&ctx, &sid, "webauthn_enroll_passkey", "{}").await;
        let header = assertion_header_for(&ctx, &challenge, b"cred-1");
        let (status, body) = register_start(&ctx, &sid, None, Some(&header)).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        let challenge = body["challenge"].as_str().expect("a challenge").to_string();

        let (status, body) = register_finish(&ctx, &sid, &challenge, b"cred-2").await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(enrolled_ids(&ctx, "alice").len(), 2);
    }

    // -- a console that can make the ceremony mandatory (defect 2) ----------

    /// GET any `/api` route with a session cookie; status plus parsed body.
    async fn get_api(ctx: &Arc<Ctx>, uri: &str, cookie: &str) -> (StatusCode, Value) {
        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .header("cookie", format!("{}={}", auth::COOKIE, cookie))
            .body(Body::empty())
            .unwrap();
        let resp = app(Arc::clone(ctx)).oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn with_the_ceremony_required_and_nothing_enrolled_every_sensitive_command_is_refused() {
        let ctx = strict_ctx();
        let sid = ctx.sessions.create("alice", Role::Admin, Method::Local);

        for command in SENSITIVE_COMMANDS {
            let (status, body) = post_api(
                &ctx,
                &format!("/api/command/{command}"),
                &sid,
                None,
                json!({}),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{command} body: {body}");
            assert_eq!(body["webauthn"], "enrollment_required", "{command}");
            // Actionable, not a bare 403: it has to say what to do about it.
            let error = body["error"].as_str().unwrap_or_default();
            assert!(error.contains("passkey"), "{command}: {error}");
            assert!(error.contains("enrol"), "{command}: {error}");
        }

        // And the probe says so too, so the panel can explain it before the
        // operator finds out by being refused.
        let (status, body) = get_api(&ctx, "/api/webauthn/passkeys", &sid).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["policy_requires_passkey"], true);
    }

    #[tokio::test]
    async fn with_the_ceremony_optional_the_fallback_still_runs_and_is_still_software_signed() {
        let ctx = test_ctx();
        let sid = ctx.sessions.create("alice", Role::Admin, Method::Local);

        // Today's behaviour, unchanged by the new setting's default.
        let status = post_command_with(&ctx, "money_kill_run", &sid, None, "{}").await;
        assert_ne!(status, StatusCode::FORBIDDEN);
        assert_ne!(status, StatusCode::PRECONDITION_REQUIRED);
        assert_ne!(status, StatusCode::UNAUTHORIZED);

        let (_, body) = get_api(&ctx, "/api/webauthn/passkeys", &sid).await;
        assert_eq!(body["policy_requires_passkey"], false);

        // "Journaled software-signed" is exactly this: the gate hands the
        // command layer NO ceremony override, so `CommandRecord`'s own
        // transport-signing fields are what the journal carries.
        let session = auth::SessionInfo {
            user: "alice".into(),
            role: Role::Admin,
            method: Method::Local,
        };
        let signature = webauthn_gate(
            &ctx,
            &session,
            "money_kill_run",
            &json!({}),
            &HeaderMap::new(),
        )
        .expect("the fallback passes the gate");
        assert!(signature.is_none(), "the fallback overrides nothing");
        let journaled = genaryx_api::console_actor::with_signature(signature, async {
            genaryx_api::console_actor::signature_or("es256", "software-signed")
        })
        .await;
        assert_eq!(
            journaled,
            ("es256".to_string(), "software-signed".to_string())
        );
    }

    #[tokio::test]
    async fn with_the_ceremony_required_an_enrolled_caller_takes_the_ordinary_path() {
        let ctx = strict_ctx();
        let sid = ctx.sessions.create("alice", Role::Admin, Method::Oidc);
        enroll_test_passkey(&ctx, "alice");

        // Enrolled and no assertion: the ordinary 428 retry signal, NOT the
        // enrollment refusal (the frontend retries on exactly this shape).
        let (status, body) =
            post_api(&ctx, "/api/command/money_kill_run", &sid, None, json!({})).await;
        assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "body: {body}");
        assert_eq!(body["webauthn"], "required");

        // And the full ceremony passes exactly as it does without the setting.
        let challenge = start_action(&ctx, &sid, "money_kill_run", "{}").await;
        let header = assertion_header(&ctx, &challenge);
        let status = post_command_with(&ctx, "money_kill_run", &sid, Some(&header), "{}").await;
        assert_ne!(status, StatusCode::PRECONDITION_REQUIRED);
        assert_ne!(status, StatusCode::FORBIDDEN);
        assert_ne!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_corrupt_passkey_store_refuses_removal_instead_of_reading_empty() {
        let dir = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("passkeys.json"), "{ not json").unwrap();
        let ctx = ctx_at(dir, false);
        let sid = ctx.sessions.create("alice", Role::Admin, Method::Oidc);

        // An unreadable store must not read as "nobody enrolled": both the
        // removal and the sensitive command refuse.
        let (status, body) = post_api(
            &ctx,
            "/api/webauthn/passkeys/remove",
            &sid,
            None,
            json!({ "credential_id": B64URL.encode(b"cred-1") }),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body: {body}");
        assert_eq!(
            post_command_with(&ctx, "money_kill_run", &sid, None, "{}").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
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
