//! `genaryx-web`: the Genaryx console, served over HTTP from inside the
//! customer's own perimeter.
//!
//! This process runs beside the customer's stack, on their box. It reads the
//! same `~/.taipan/environments/` descriptors the desktop console reads and
//! answers every request by calling `genaryx-api`, the same functions the
//! Tauri shell wraps. No run, spend figure, identity or policy decision
//! leaves the customer's network to render this UI, and it-rat.com has no
//! route to it: the site sells and licenses the product, it never sees the
//! data.
//!
//! Reaching it is the operator's own tunnel (D11). The default bind is
//! loopback for exactly that reason, and binding wider says so out loud.

mod auth;
mod config;
mod ctx;
mod dispatch;
mod doctor;

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

    let app = app.with_state(Arc::clone(&ctx));
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

#[derive(Deserialize)]
struct Credentials {
    username: String,
    password: String,
}

/// Who the caller is, and whether this box has an operator at all.
///
/// Answering "not configured" to an anonymous caller is deliberate: it is the
/// difference between a first-run box that needs setting up and a box whose
/// password you do not know, and the operator standing in front of it needs
/// to be told which.
async fn session(State(ctx): State<Arc<Ctx>>, jar: CookieJar) -> Response {
    let configured = ctx.operator.read().expect("operator lock").is_some();
    let user = jar
        .get(auth::COOKIE)
        .and_then(|c| ctx.sessions.touch(c.value()));
    Json(json!({
        "configured": configured,
        "signed_in": user.is_some(),
        "user": user,
    }))
    .into_response()
}

async fn login(
    State(ctx): State<Arc<Ctx>>,
    jar: CookieJar,
    Json(body): Json<Credentials>,
) -> Response {
    let op = ctx.operator.read().expect("operator lock").clone();
    let Some(op) = op else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "no operator account on this box yet"})),
        )
            .into_response();
    };
    if !auth::verify(&op, &body.username, &body.password) {
        tracing::warn!(user = %body.username, "sign-in refused");
        // One message for both failures: the endpoint must not say which half
        // was wrong.
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "wrong username or password"})),
        )
            .into_response();
    }
    let id = ctx.sessions.create(&op.username);
    let mut cookie = Cookie::new(auth::COOKIE, id);
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Strict);
    cookie.set_path("/");
    cookie.set_secure(ctx.cfg.secure_cookies);
    tracing::info!(user = %op.username, "signed in");
    (
        jar.add(cookie),
        Json(json!({"signed_in": true, "user": op.username})),
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
fn guard(ctx: &Arc<Ctx>, jar: &CookieJar) -> Result<String, Response> {
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
    if let Err(r) = guard(&ctx, &jar) {
        return r;
    }
    let args = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    match dispatch::dispatch(&ctx, &name, args).await {
        Ok(r) => r,
        Err(r) => r,
    }
}

/// The live bus, as Server-Sent Events, plus (multiplexed onto the SAME
/// connection) any live remote-tail lines.
///
/// The desktop shell delivers the bus feed as a Tauri event; in a browser it
/// is an `EventSource`. Same payload, same cadence, so the panels that redraw
/// on a new event behave identically in both shells. The bus rides the
/// `bus`-named SSE event, unchanged from before the Remote panel moved here;
/// a remote tail's lines/ended marker ride their own `remote:tail-line`/
/// `remote:tail-ended` named events instead of being folded into the `bus`
/// shape they do not fit (see `ctx::RemoteTailEvent`'s own doc comment) - one
/// `EventSource` in the browser, one `addEventListener` per name, exactly the
/// desktop shell's two independent Tauri events over one process.
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
