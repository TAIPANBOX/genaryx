//! Policy-panel console-managed state: a [`WardryxClient`] (or an honest
//! record of why there isn't one yet), plus everything a decision needs to
//! journal a `console_command` onto the same bus the Bus Explorer tails.
//!
//! Mirrors `crate::money::state` structurally (same `Bootstrapping` ->
//! background-resolve -> `Ready`/failure-variant shape, same non-blocking
//! `setup`-calls-[`PolicyState::pending`]-then-spawns-[`bootstrap`]
//! contract), but simpler throughout because Wardryx is bearer-only (07
//! §4.3, `crates/connectors/src/wardryx.rs`'s module docs): no signer to
//! generate, no device to pair/attach, and therefore no `org` a pairing
//! response could hand back. [`bootstrap`] below explains where
//! [`PolicyClient::org_domain`] comes from instead.
//!
//! Flow: [`env::discover`] -> build a [`WardryxClient`] -> a `GET /healthz`
//! liveness check (the closest Wardryx equivalent to money's pairing
//! handshake: proof the resolved URL is actually a live wardryx, not just a
//! resolved string). Every step is fallible and none of them panics: any
//! failure lands in [`PolicyInner::NoEnvironment`] or
//! [`PolicyInner::Unreachable`], and the Policy view renders that state
//! instead of the app failing to launch (06 §0.5 fail-closed).

use super::env::{self, EnvSource, ResolvedEnv};
use genaryx_connectors::WardryxClient;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// How long [`bootstrap`] waits for discovery+healthz before giving up and
/// falling back to [`PolicyInner::Unreachable`] - same value and rationale
/// as `money::state::PAIRING_TIMEOUT`.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// One of the six demo-seeded NDJSON files `live::bootstrap` always
/// registers as a `FileTail` source (`genaryx_core::demo`'s `SOURCES`:
/// tokenfuse, wardryx, engram, verdryx, mockryx, qryx). The live feeder
/// (`live.rs::feeder_line`'s `VARIANTS`) cycles through exactly four of
/// those - wardryx, verdryx, mockryx, engram - and `money::state` already
/// claimed `tokenfuse.ndjson` for its own `console_command` journal
/// (deliberately, per that module's own doc comment, because the feeder
/// never writes there). `qryx.ndjson` is the one remaining file NEITHER the
/// feeder NOR the money panel ever appends to, so it is the only choice
/// here that cannot race a concurrent writer. Appending here does not
/// affect the Decision Stream (`DecisionStream.tsx` filters on
/// `source == "wardryx"`; every `console_command` line this module writes
/// carries `source:"console"` regardless of which file it lands in, per
/// `genaryx_core::command::console_command_line`) - the file choice is
/// purely about avoiding a write race, not about which planes render it.
const CONSOLE_EVENTS_FILE: &str = "qryx.ndjson";

/// Where the console events file (`command::record`'s `console_events_path`)
/// and its companion `console.sqlite` live. Deliberately duplicated from
/// `money::state::BusHandle` (a two-field struct) rather than shared - see
/// `super::env`'s module docs for why this module keeps its own small
/// mirrors instead of coupling to `money`.
#[derive(Debug, Clone)]
pub struct BusHandle {
    pub store_db_path: PathBuf,
    pub console_events_path: PathBuf,
}

impl BusHandle {
    /// `store_dir` holds the console's own SQLite; `source_dir` is where the
    /// products write their NDJSON. Two different directories in a live
    /// deployment: a `console_command` written into the store directory lands
    /// where nothing tails and nothing keeps it.
    pub fn from_dirs(store_dir: &std::path::Path, source_dir: &std::path::Path) -> Self {
        Self {
            store_db_path: store_dir.join("console.sqlite"),
            console_events_path: source_dir.join(CONSOLE_EVENTS_FILE),
        }
    }
}

/// A ready-to-use Wardryx connection plus everything [`super::commands`]'s
/// `policy_decide_approval` needs to build a `genaryx_core::CommandRecord`.
/// Cheap to clone (an `Arc`ed client plus a handful of small strings/paths),
/// mirroring `money::state::MoneyClient`'s identical rationale.
#[derive(Clone)]
pub struct PolicyClient {
    pub client: Arc<WardryxClient>,
    pub source: EnvSource,
    pub wardryx_url: String,
    /// An `agent_id`-safe org label (07 §1 `[a-z0-9.-]+`) this policy
    /// plane's journal entries are filed under. Unlike
    /// `MoneyClient::org_domain` (learned from a live pairing response),
    /// Wardryx has no handshake that hands an org back to the caller - its
    /// bearer auth is purely static (`Authorization: Bearer <token>`,
    /// looked up server-side against `WARDRYX_KEYS`), and per
    /// docs/PHASE1.md's issue-#20 fix the keyfile secret this panel reads
    /// is now the BARE token alone, with no embedded org either. So this is
    /// derived locally, with no network round trip, from whatever
    /// [`EnvSource`] resolved: the sanitized `taipan up` environment name
    /// for [`EnvSource::Taipan`], or a fixed `"wardryx.local"` for
    /// [`EnvSource::EnvFallback`] - see [`org_domain_for`]. Either way it is
    /// only ever used to build the acting `agent_id` on this panel's own
    /// `console_command` lines (`agent://<org_domain>/console/<host>`), not
    /// sent to Wardryx or asserted as Wardryx's own notion of org.
    pub org_domain: String,
    /// `user://<org_domain>/<local OS user>` - the `decided_by` principal
    /// `WardryxClient::decide_approval` records, and the `operator` on every
    /// `CommandRecord` this panel journals. Same derivation as
    /// `MoneyClient::operator` (no separate login system in the desktop
    /// shell; the OS user is the closest honest identity available).
    pub operator: String,
    pub host: String,
    /// Honest signing-assurance labels for this plane's `CommandRecord`s.
    /// Wardryx's admin API has no signing story at all (bearer-only, see
    /// this module's doc comment), so unlike `MoneyClient::sig_fpr`
    /// (`"software-signed"`/`"secure-enclave"`), these are fixed, plain
    /// labels that say exactly what actually authenticated the call: a
    /// static admin bearer, nothing cryptographically signed. Never
    /// upgraded to look more assured than the transport actually is.
    pub sig_alg: &'static str,
    pub sig_fpr: &'static str,
    pub bus: Option<BusHandle>,
}

/// The Policy panel's whole state machine - mirrors `money::state::MoneyInner`
/// with `Unreachable` standing in for `PairingFailed` (renamed: there is no
/// pairing here, just a failed/timed-out `healthz` liveness check).
pub enum PolicyInner {
    /// The initial state from [`PolicyState::pending`], until the
    /// background [`bootstrap`] task resolves.
    Bootstrapping,
    /// [`env::discover`] found nothing usable: no `taipan up` descriptor
    /// with a `wardryx` service and no `WARDRYX_ADMIN_KEY`. This is the
    /// common case for a `taipan up` stack brought up without
    /// `--with wardryx` - a normal, renderable "no policy plane" state, not
    /// an error (PHASE2.md's Wave-2 parity checklist calls this out by
    /// name).
    NoEnvironment,
    /// An environment resolved (a URL + bearer we could construct a client
    /// for), but the `GET /healthz` liveness check failed or timed out -
    /// e.g. a stale descriptor pointing at a wardryx that is no longer
    /// running.
    Unreachable {
        source: EnvSource,
        wardryx_url: String,
        reason: String,
    },
    Ready(PolicyClient),
}

/// Console-managed state wrapping [`PolicyInner`] in an async mutex, mirroring
/// `money::state::MoneyState`'s identical shape (minus the budget-override
/// map, which has no Policy-panel equivalent).
pub struct PolicyState {
    pub inner: Mutex<PolicyInner>,
}

impl PolicyState {
    /// The synchronous, immediately-manageable starting state - `setup`
    /// calls this directly, then spawns [`bootstrap`] in the background
    /// (see this module's doc comment).
    #[must_use]
    pub fn pending() -> Self {
        Self {
            inner: Mutex::new(PolicyInner::Bootstrapping),
        }
    }
}

/// Resolve an environment and confirm it is actually live, producing the
/// [`PolicyInner`] the caller should swap into managed state. `events_dir`
/// is the live-wire directory from `live::bootstrap` (`None` if that step
/// itself failed); see [`BusHandle`]. Never panics and never returns an
/// `Err`: every failure mode is a [`PolicyInner`] variant the UI can render.
pub async fn bootstrap(dirs: Option<(PathBuf, PathBuf)>) -> PolicyInner {
    let bus = dirs.map(|(store, source)| BusHandle::from_dirs(&store, &source));

    let Some(resolved) = env::discover() else {
        return PolicyInner::NoEnvironment;
    };

    match tokio::time::timeout(CONNECT_TIMEOUT, connect(&resolved)).await {
        Ok(Ok(client)) => {
            let org_domain = org_domain_for(&resolved.source);
            let operator = operator_principal(&org_domain);
            PolicyInner::Ready(PolicyClient {
                client: Arc::new(client),
                source: resolved.source,
                wardryx_url: resolved.wardryx_url,
                org_domain,
                operator,
                host: local_hostname(),
                sig_alg: "none",
                sig_fpr: "bearer-admin",
                bus,
            })
        }
        Ok(Err(reason)) => PolicyInner::Unreachable {
            source: resolved.source,
            wardryx_url: resolved.wardryx_url,
            reason,
        },
        Err(_elapsed) => PolicyInner::Unreachable {
            source: resolved.source,
            wardryx_url: resolved.wardryx_url,
            reason: format!(
                "timed out after {:.0}s waiting for wardryx to respond",
                CONNECT_TIMEOUT.as_secs_f64()
            ),
        },
    }
}

/// Build a [`WardryxClient`] and confirm it is live via `GET /healthz` - the
/// whole "connect" ceremony here, versus `money::state::connect`'s
/// pair+attach (Wardryx has neither, see this module's doc comment).
async fn connect(resolved: &ResolvedEnv) -> Result<WardryxClient, String> {
    let client = WardryxClient::new(resolved.wardryx_url.clone(), resolved.admin_bearer.clone())
        .map_err(|e| e.to_string())?;
    client
        .healthz()
        .await
        .map_err(|e| format!("wardryx health check failed: {e}"))?;
    Ok(client)
}

/// See [`PolicyClient::org_domain`]'s doc comment for why this is derived
/// locally instead of learned from the server. Reuses [`sanitize_domain`]
/// so the result is always a conforming `agent_id` domain segment
/// regardless of what the taipan environment happened to be named.
fn org_domain_for(source: &EnvSource) -> String {
    match source {
        EnvSource::Taipan { name } => sanitize_domain(name),
        EnvSource::EnvFallback => "wardryx.local".to_string(),
    }
}

/// Fold an arbitrary string into the `agent_id`-safe charset
/// `command::console_command_line` requires (07 §1,
/// `^agent://[a-z0-9.-]+/...`. Byte-for-byte duplicate of
/// `money::state::sanitize_domain` (itself a two-line mirror of
/// `genaryx_core::command`'s private `sanitize_host`) - see `super::env`'s
/// module docs for why this module keeps its own copies rather than
/// depending on `money`.
fn sanitize_domain(org: &str) -> String {
    let sanitized: String = org
        .trim()
        .chars()
        .map(|c| {
            let lower = c.to_ascii_lowercase();
            if lower.is_ascii_alphanumeric() || lower == '.' || lower == '-' {
                lower
            } else {
                '-'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "genaryx.local".to_string()
    } else {
        sanitized
    }
}

/// `user://<org_domain>/<local-user>` - see [`PolicyClient::operator`].
/// Byte-for-byte duplicate of `money::state::operator_principal`.
fn operator_principal(org_domain: &str) -> String {
    let user = std::env::var("USER")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| "operator".to_string());
    format!("user://{org_domain}/{user}")
}

/// Best-effort local hostname for `command::record`'s `host` parameter.
/// Byte-for-byte duplicate of `money::state::local_hostname`.
fn local_hostname() -> String {
    if let Ok(h) = std::env::var("HOSTNAME")
        && !h.trim().is_empty()
    {
        return h;
    }
    std::process::Command::new("hostname")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_domain_lowercases_and_folds_bad_chars() {
        assert_eq!(sanitize_domain("Taipan-P1Full"), "taipan-p1full");
        assert_eq!(sanitize_domain("acme_corp inc"), "acme-corp-inc");
        assert_eq!(sanitize_domain(""), "genaryx.local");
        assert_eq!(sanitize_domain("   "), "genaryx.local");
    }

    #[test]
    fn org_domain_for_taipan_sanitizes_the_environment_name() {
        let source = EnvSource::Taipan {
            name: "P1 Full!".to_string(),
        };
        assert_eq!(org_domain_for(&source), "p1-full-");
    }

    #[test]
    fn org_domain_for_env_fallback_is_fixed() {
        assert_eq!(org_domain_for(&EnvSource::EnvFallback), "wardryx.local");
    }

    #[test]
    fn bus_handle_targets_the_qryx_ndjson_file() {
        let dir = std::path::PathBuf::from("/tmp/some-events-dir");
        let bus = BusHandle::from_dirs(&dir, &dir);
        assert_eq!(bus.store_db_path, dir.join("console.sqlite"));
        assert_eq!(bus.console_events_path, dir.join("qryx.ndjson"));
    }

    #[test]
    fn operator_principal_is_a_conforming_user_principal() {
        let op = operator_principal("acme.example");
        assert!(op.starts_with("user://acme.example/"), "got {op:?}");
    }

    #[test]
    fn pending_starts_in_the_bootstrapping_state() {
        let state = PolicyState::pending();
        let guard = state
            .inner
            .try_lock()
            .expect("uncontended right after construction");
        assert!(matches!(&*guard, PolicyInner::Bootstrapping));
    }

    #[tokio::test]
    async fn bootstrap_never_panics_with_no_environment_available() {
        // Same rationale as money::state's identical test: this only proves
        // `bootstrap` resolves to a `PolicyInner` rather than panicking or
        // hanging, regardless of whether this box happens to have a real
        // `taipan up` environment or `WARDRYX_ADMIN_KEY` set.
        let inner = bootstrap(None).await;
        match inner {
            PolicyInner::Bootstrapping => {
                panic!("bootstrap must resolve past its own pending state")
            }
            PolicyInner::NoEnvironment
            | PolicyInner::Unreachable { .. }
            | PolicyInner::Ready(_) => {}
        }
    }
}
