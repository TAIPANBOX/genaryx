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
mod oidc;
mod roles;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
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
// commands and live events
// ---------------------------------------------------------------------------

async fn command(
    State(ctx): State<Arc<Ctx>>,
    jar: CookieJar,
    Path(name): Path<String>,
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
    // Attribute any journaled mutation to the signed-in human, not the OS
    // account running this process (genaryx_api::console_actor). The desktop
    // shell never sets this, so its behavior is unchanged.
    let result = genaryx_api::console_actor::with_actor(
        Some(session.user.clone()),
        dispatch::dispatch(&ctx, &name, args),
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
}
