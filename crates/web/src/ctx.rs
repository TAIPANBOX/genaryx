//! Everything a request might need, resolved once at startup.
//!
//! The shape deliberately mirrors what the desktop shell hands to Tauri's
//! managed state, because it IS the same state: one client per plane, plus
//! the console's own bus. Each plane is created in its `pending()` form
//! immediately and resolved in the background, exactly as `lib.rs`'s `setup`
//! does, so a slow or absent Cloud delays nothing and every command has
//! something to read from the first request onward. A plane that never
//! resolves renders as a clean "no environment" state, never as an error.

use crate::auth::{Operator, Sessions};
use crate::config::Config;
use genaryx_api::bus::AppState;
use genaryx_api::copilot::CopilotState;
use genaryx_api::credentials::CredentialsState;
use genaryx_api::crypto::CryptoState;
use genaryx_api::drills::DrillsState;
use genaryx_api::events::UiEvent;
use genaryx_api::evidence::EvidenceState;
use genaryx_api::identity::IdentityState;
use genaryx_api::memory::MemoryState;
use genaryx_api::money::MoneyState;
use genaryx_api::policy::PolicyState;
use genaryx_api::quality::QualityState;
use genaryx_api::remote::RemoteState;
use genaryx_api::remote::commands::{RemoteTailEnded, RemoteTailLine, TailSink};
use std::sync::RwLock;
use tokio::sync::broadcast;

/// One event from a remote SSH tail, fanned out over its own named SSE event
/// (`remote:tail-line`/`remote:tail-ended` - see [`crate::main`]'s `events`
/// handler) rather than folded into the `UiEvent` bus stream, which has no
/// shape for a raw remote log line - see
/// `genaryx_api::remote::commands::TailSink`'s own module doc for why this is
/// a separate sink in the first place.
#[derive(Debug, Clone)]
pub enum RemoteTailEvent {
    Line(RemoteTailLine),
    Ended(RemoteTailEnded),
}

/// This shell's [`TailSink`]: forwards each remote-tail line/ended event onto
/// its own SSE broadcast channel - mirrors `main.rs`'s `SseSink`'s identical
/// role for the live bus feed. Built fresh per `remote_ssh_tail_start` call
/// (see `dispatch.rs`) from `Ctx.remote_tail`, cheap since a broadcast
/// `Sender` is just a cloneable handle onto the shared channel.
#[derive(Clone)]
pub struct SseTailSink(pub broadcast::Sender<RemoteTailEvent>);

impl TailSink for SseTailSink {
    fn line(&self, line: RemoteTailLine) {
        // A send with no subscribers is not an error: nobody has the console
        // open, and there is nothing to replay a live tail line from later
        // (unlike the bus, a remote tail is not durably stored anywhere).
        let _ = self.0.send(RemoteTailEvent::Line(line));
    }

    fn ended(&self, ended: RemoteTailEnded) {
        let _ = self.0.send(RemoteTailEvent::Ended(ended));
    }
}

/// Shared application context, held behind an `Arc` by every handler.
pub struct Ctx {
    pub cfg: Config,
    pub bus: AppState,
    pub money: MoneyState,
    pub policy: PolicyState,
    pub identity: IdentityState,
    /// I15 "key lifecycle health": the gateway's key-lifecycle report, an
    /// entirely independent plane from `identity` above (different
    /// descriptor service - `services.gateway`, not `services.idryx` - see
    /// `genaryx_api::credentials`'s module doc) that just happens to render
    /// in the same Identity tab.
    pub credentials: CredentialsState,
    pub quality: QualityState,
    pub crypto: CryptoState,
    pub memory: MemoryState,
    pub drills: DrillsState,
    pub evidence: EvidenceState,
    pub remote: RemoteState,
    pub copilot: CopilotState,
    pub sessions: Sessions,
    /// The operator record, re-readable at runtime so setting a password does
    /// not need a restart.
    pub operator: RwLock<Option<Operator>>,
    /// Offline OIDC config for the IdP login path (docs/CONSOLE-IDP.md).
    /// `None` unless `GENARYX_WEB_OIDC_*` is configured at startup, in which
    /// case the login route also accepts a verified ID-token and the local
    /// account stays as break-glass. Resolved once here, never per request.
    pub oidc: Option<crate::oidc::OidcConfig>,
    /// Live bus events, fanned out to every open SSE stream. Bounded on
    /// purpose: a browser tab that stops reading drops events rather than
    /// growing this process's memory without limit. The UI's own reads are
    /// the source of truth for state; this stream is for liveness.
    pub events: broadcast::Sender<UiEvent>,
    /// Live remote-tail lines, fanned out the same way `events` is - see
    /// [`RemoteTailEvent`]/[`SseTailSink`]. Created here rather than threaded
    /// in like `events`: nothing needs to subscribe before a tail actually
    /// starts (unlike the bus feeder, which is already running by the time
    /// `Ctx` exists), so there is no risk of a missed first batch to guard
    /// against.
    pub remote_tail: broadcast::Sender<RemoteTailEvent>,
}

impl Ctx {
    /// Build the context with every plane in its pending state, then resolve
    /// each in the background.
    ///
    /// `events` is passed in rather than created here because the bus feeder
    /// has to be handed the matching sender BEFORE this exists: the feeder
    /// starts producing as soon as it is bootstrapped, and a channel created
    /// here would be a second, unconnected one that no event ever reaches.
    pub fn bootstrap(cfg: Config, bus: AppState, events: broadcast::Sender<UiEvent>) -> Self {
        Self {
            operator: RwLock::new(crate::auth::load(&cfg.operator_file())),
            oidc: crate::oidc::OidcConfig::from_env(),
            bus,
            money: MoneyState::pending(),
            policy: PolicyState::pending(),
            identity: IdentityState::pending(),
            credentials: CredentialsState::pending(),
            quality: QualityState::pending(),
            crypto: CryptoState::pending(),
            memory: MemoryState::pending(),
            drills: DrillsState::pending(),
            evidence: EvidenceState::pending(),
            remote: RemoteState::pending(),
            copilot: CopilotState::pending(),
            sessions: Sessions::default(),
            events,
            // No prior sender to reuse (see this field's own doc comment) -
            // a fresh bounded channel, same capacity as `events`.
            remote_tail: broadcast::channel(512).0,
            cfg,
        }
    }

    /// Kick off the per-plane resolution. Split from [`Self::bootstrap`] so
    /// the caller can put the context behind its `Arc` first: each task needs
    /// a handle on the same state the request path will read.
    pub fn resolve(self: &std::sync::Arc<Self>) {
        let events_dir = self.bus.events_dir.clone();

        macro_rules! resolve {
            ($field:ident, $call:expr) => {{
                let ctx = std::sync::Arc::clone(self);
                tokio::spawn(async move {
                    let resolved = $call.await;
                    *ctx.$field.inner.lock().await = resolved;
                });
            }};
        }

        let money_dir = events_dir.clone();
        resolve!(money, genaryx_api::money::bootstrap(money_dir));
        let policy_dir = events_dir.clone();
        resolve!(policy, genaryx_api::policy::bootstrap(policy_dir));
        resolve!(identity, genaryx_api::identity::bootstrap());
        resolve!(credentials, genaryx_api::credentials::bootstrap());
        resolve!(quality, genaryx_api::quality::bootstrap());
        resolve!(crypto, genaryx_api::crypto::bootstrap());
        resolve!(memory, genaryx_api::memory::bootstrap());
        resolve!(drills, genaryx_api::drills::bootstrap());
        resolve!(evidence, genaryx_api::evidence::bootstrap());
        resolve!(remote, genaryx_api::remote::bootstrap());
        resolve!(copilot, genaryx_api::copilot::bootstrap());
    }
}
