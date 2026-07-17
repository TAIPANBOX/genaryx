//! Identity-panel Tauri managed state: an [`IdryxClient`] (or an honest
//! record of why there isn't one yet), plus the two extra, best-effort
//! resolutions Rescan needs (docs/PHASE3.md W2): the `idryx` binary and the
//! taipan events dir's per-source ndjson files.
//!
//! Mirrors `crate::policy::state` structurally (same `Bootstrapping` ->
//! background-resolve -> `Ready`/failure-variant shape, same non-blocking
//! `setup`-calls-[`IdentityState::pending`]-then-spawns-[`bootstrap`]
//! contract), simpler in one way (idryx has no auth at all, not even a
//! bearer - [`connect`] below is a bare `GET /healthz`) and richer in
//! another: unlike Policy/Money, this module's [`bootstrap`] does NOT take
//! the console's own internal demo/live events dir at all (Identity
//! journals nothing - see `super::commands`'s module doc), but it DOES do
//! two extra best-effort resolutions Rescan needs, neither of which is part
//! of "discovering an environment" so neither lives in [`super::env`]:
//!
//! - the `idryx` binary for [`IdryxClient::rescan`] - tried at the fixed,
//!   well-known `~/.taipan/bin/idryx` (mirrors `TaipanHome::bin_dir()` in
//!   the taipan CLI, `~/Development/taipan/src/home.rs`); simply absent, not
//!   an error, when the file is not there.
//! - the `--load` specs for a Rescan, built from [`env::ResolvedEnv`]'s own
//!   `events_dir`/`event_files` (the SAME descriptor `env::discover` already
//!   parsed), intersected with the sources idryx's stack-bus `--load`
//!   actually accepts (`tokenfuse|wardryx|mockryx|verdryx`, 07 §4.4 /
//!   `crates/connectors/src/idryx.rs`'s module doc) and filtered to files
//!   that actually exist on disk right now.
//!
//! Both are best-effort and non-fatal: a `Ready` identity plane with an
//! unresolved binary or zero loads still renders every read command fine;
//! only [`super::commands::identity_rescan`] is affected, and it reports
//! that honestly (`IdentityError::RescanUnavailable`) rather than a fake
//! success.

use super::env::{self, EnvSource, ResolvedEnv};
use genaryx_connectors::IdryxClient;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// How long [`bootstrap`] waits for discovery+healthz before giving up and
/// falling back to [`IdentityInner::Unreachable`] - same value and
/// rationale as `policy::state::CONNECT_TIMEOUT`.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The stack-bus sources idryx's `--load` accepts for a Rescan (07 §4.4,
/// `crates/connectors/src/idryx.rs`'s module doc: "sources for the stack
/// bus: tokenfuse|wardryx|mockryx|verdryx"). Idryx's general `--source` flag
/// accepts a wider vocabulary (okta/entra/cloudtrail/...) that has no ndjson
/// file in a taipan events dir at all, so this list is deliberately
/// narrower than idryx's own vocabulary - only the sources a taipan
/// environment can ever actually produce a file for.
const ACCEPTED_LOAD_SOURCES: &[&str] = &["tokenfuse", "wardryx", "mockryx", "verdryx"];

/// A ready-to-use Idryx connection plus everything [`super::commands`]'s
/// reads and Rescan need. Cheap to clone (an `Arc`ed client plus a handful
/// of small strings/paths), mirroring `PolicyClient`'s identical rationale.
#[derive(Clone)]
pub struct IdentityClient {
    pub client: Arc<IdryxClient>,
    pub source: EnvSource,
    pub idryx_url: String,
    /// The resolved `idryx` binary for Rescan, or `None` when
    /// `~/.taipan/bin/idryx` is not a file - Rescan is then simply
    /// unavailable (`IdentityError::RescanUnavailable`), never a fake
    /// success.
    pub idryx_bin: Option<PathBuf>,
    /// `(source, path)` pairs for Rescan's `--load` flags - may be empty
    /// (e.g. an environment with only `tokenfuse` and no `--with wardryx`);
    /// an empty list is a normal, honestly-smaller Rescan, not an error.
    pub rescan_loads: Vec<(String, PathBuf)>,
}

/// The Identity panel's whole state machine - mirrors `PolicyInner` with the
/// same four shapes (no separate "pairing" concept here either: idryx has
/// no auth at all, so `Unreachable` covers a failed/timed-out `healthz`
/// exactly like it does for Policy's bearer-only Wardryx).
pub enum IdentityInner {
    /// The initial state from [`IdentityState::pending`], until the
    /// background [`bootstrap`] task resolves.
    Bootstrapping,
    /// [`env::discover`] found nothing usable: no `taipan up` descriptor
    /// with an `idryx` service. The common case for an environment brought
    /// up without `--with idryx` - a normal, renderable "no identity plane"
    /// state, never an error (PHASE3.md's Wave-2 parity checklist calls
    /// this out by name).
    NoEnvironment,
    /// An environment resolved (a URL we could build a client for), but
    /// `GET /healthz` failed, timed out, or answered a non-2xx status.
    Unreachable {
        source: EnvSource,
        idryx_url: String,
        reason: String,
    },
    Ready(IdentityClient),
}

/// Tauri-managed state wrapping [`IdentityInner`] in an async mutex,
/// mirroring `PolicyState`'s identical shape.
pub struct IdentityState {
    pub inner: Mutex<IdentityInner>,
}

impl IdentityState {
    /// The synchronous, immediately-manageable starting state - `setup`
    /// calls this directly, then spawns [`bootstrap`] in the background
    /// (see this module's doc comment).
    #[must_use]
    pub fn pending() -> Self {
        Self {
            inner: Mutex::new(IdentityInner::Bootstrapping),
        }
    }
}

/// Resolve an environment, confirm it is actually live, and (best-effort)
/// resolve everything Rescan needs - the [`IdentityInner`] the caller should
/// swap into managed state. Unlike `policy::state::bootstrap`/
/// `money::state::bootstrap`, this takes no `events_dir` parameter:
/// Identity journals nothing (see `super::commands`'s module doc), so it
/// has no use for the console's own internal demo/live events dir the other
/// two panels journal onto. Never panics and never returns anything other
/// than an [`IdentityInner`] the UI can render.
pub async fn bootstrap() -> IdentityInner {
    let Some(resolved) = env::discover() else {
        return IdentityInner::NoEnvironment;
    };

    match tokio::time::timeout(CONNECT_TIMEOUT, connect(&resolved)).await {
        Ok(Ok(client)) => {
            let rescan_loads =
                resolve_rescan_loads(resolved.events_dir.as_deref(), &resolved.event_files);
            IdentityInner::Ready(IdentityClient {
                client: Arc::new(client),
                source: resolved.source,
                idryx_url: resolved.idryx_url,
                idryx_bin: resolve_idryx_bin(),
                rescan_loads,
            })
        }
        Ok(Err(reason)) => IdentityInner::Unreachable {
            source: resolved.source,
            idryx_url: resolved.idryx_url,
            reason,
        },
        Err(_elapsed) => IdentityInner::Unreachable {
            source: resolved.source,
            idryx_url: resolved.idryx_url,
            reason: format!(
                "timed out after {:.0}s waiting for idryx to respond",
                CONNECT_TIMEOUT.as_secs_f64()
            ),
        },
    }
}

/// Build an [`IdryxClient`] and confirm it is live via `GET /healthz`.
/// Unlike Wardryx/Cloud, idryx has no auth at all to fail on - the only
/// failure modes are a transport error and a non-2xx status
/// (`IdryxClient::healthz` returns `Ok(false)` for the latter rather than an
/// `Err`, since it only checks the status code, never a body parse - see
/// its own doc comment), both folded into one `Err(String)` here exactly
/// like `policy::state::connect`'s equivalent step.
async fn connect(resolved: &ResolvedEnv) -> Result<IdryxClient, String> {
    let client = IdryxClient::new(resolved.idryx_url.clone()).map_err(|e| e.to_string())?;
    match client.healthz().await {
        Ok(true) => Ok(client),
        Ok(false) => Err("idryx /healthz answered a non-success status".to_string()),
        Err(e) => Err(format!("idryx health check failed: {e}")),
    }
}

/// `~/.taipan/bin/idryx`, best-effort - mirrors `TaipanHome::bin_dir()` in
/// the taipan CLI (`~/Development/taipan/src/home.rs`). `None` when `$HOME`
/// is unset or no file exists there; either way Rescan is simply
/// unavailable, never a panic and never a guessed alternate path.
fn resolve_idryx_bin() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let candidate = PathBuf::from(home)
        .join(".taipan")
        .join("bin")
        .join("idryx");
    candidate.is_file().then_some(candidate)
}

/// Build Rescan's `--load` specs: `events_dir.join(file)` for every
/// `event_files` entry whose source idryx's stack-bus `--load` actually
/// accepts ([`ACCEPTED_LOAD_SOURCES`]), filtered to files that exist on disk
/// right now. `events_dir: None` (no events section on the descriptor, or a
/// blank `events.dir`) yields an empty list, same as it would for any other
/// reason a candidate file turns out to be unusable - Rescan then just runs
/// with fewer sources, not an error.
fn resolve_rescan_loads(
    events_dir: Option<&Path>,
    event_files: &BTreeMap<String, String>,
) -> Vec<(String, PathBuf)> {
    let Some(dir) = events_dir else {
        return Vec::new();
    };
    event_files
        .iter()
        .filter(|(source, _)| ACCEPTED_LOAD_SOURCES.contains(&source.as_str()))
        .map(|(source, file)| (source.clone(), dir.join(file)))
        .filter(|(_, path)| path.is_file())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_starts_in_the_bootstrapping_state() {
        let state = IdentityState::pending();
        let guard = state
            .inner
            .try_lock()
            .expect("uncontended right after construction");
        assert!(matches!(&*guard, IdentityInner::Bootstrapping));
    }

    #[tokio::test]
    async fn bootstrap_never_panics_with_no_environment_available() {
        // Same rationale as policy::state's identical test: this only
        // proves `bootstrap` resolves to an `IdentityInner` rather than
        // panicking or hanging, regardless of whether this box happens to
        // have a real `taipan up` environment.
        let inner = bootstrap().await;
        match inner {
            IdentityInner::Bootstrapping => {
                panic!("bootstrap must resolve past its own pending state")
            }
            IdentityInner::NoEnvironment
            | IdentityInner::Unreachable { .. }
            | IdentityInner::Ready(_) => {}
        }
    }

    #[test]
    fn resolve_rescan_loads_filters_to_accepted_sources_and_existing_files() {
        let dir = std::env::temp_dir().join(format!(
            "genaryx-identity-state-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create dir");
        std::fs::write(dir.join("tokenfuse.ndjson"), "").expect("touch tokenfuse file");
        // "wardryx.ndjson" deliberately NOT written, to prove a
        // declared-but-missing file is skipped rather than fabricated into
        // a load spec.

        let mut files = BTreeMap::new();
        files.insert("tokenfuse".to_string(), "tokenfuse.ndjson".to_string());
        files.insert("wardryx".to_string(), "wardryx.ndjson".to_string());
        // Not an accepted stack-bus source (idryx's general --source
        // vocabulary, but never a taipan events-dir file).
        files.insert("okta".to_string(), "okta.ndjson".to_string());

        let loads = resolve_rescan_loads(Some(&dir), &files);
        assert_eq!(loads.len(), 1, "got {loads:?}");
        assert_eq!(loads[0].0, "tokenfuse");
        assert_eq!(loads[0].1, dir.join("tokenfuse.ndjson"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_rescan_loads_is_empty_with_no_events_dir() {
        let files = BTreeMap::new();
        assert!(resolve_rescan_loads(None, &files).is_empty());
    }

    #[test]
    fn resolve_idryx_bin_never_panics() {
        // Best-effort: this only proves the function resolves to a
        // consistent Option without panicking - whether this box actually
        // has ~/.taipan/bin/idryx depends on local dev state, not this
        // test.
        let _ = resolve_idryx_bin();
    }
}
