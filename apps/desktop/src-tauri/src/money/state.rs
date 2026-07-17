//! Money-panel Tauri managed state: a paired `CloudClient` (or an honest
//! record of why there isn't one yet), plus everything a mutation needs to
//! journal a `console_command` onto the same bus the Bus Explorer tails.
//!
//! Bootstrap is non-blocking by design: `lib.rs`'s `setup` hook calls
//! [`MoneyState::pending`] synchronously (managed immediately, so every
//! command has *something* to read from the instant the app starts) and
//! then `tauri::async_runtime::spawn`s [`bootstrap`] in the background,
//! swapping the real result in once it resolves. This deliberately avoids
//! `tauri::async_runtime::block_on` inside `setup`: pairing is a network
//! round trip, and blocking app startup on it (or risking a
//! runtime-within-a-runtime panic if `setup` ever turns out to already run
//! inside Tauri's own async context) is worse than a brief
//! [`MoneyInner::Bootstrapping`] window the frontend can render as
//! "connecting...".
//!
//! Flow: [`env::discover`] -> build a [`CloudClient`] -> pair a fresh
//! [`SoftwareSigner`] -> `attach_device`. Every step is fallible and none of
//! them panics: any failure lands in [`MoneyInner::NoEnvironment`] or
//! [`MoneyInner::PairingFailed`], and the Money/Overview views render that
//! state instead of the app failing to launch (06 §0.5 fail-closed,
//! matching `live::bootstrap`'s own degrade-to-mock-data contract for the
//! Bus Explorer).

use super::env::{self, EnvSource, ResolvedEnv};
use genaryx_connectors::CloudClient;
use genaryx_signing::{Es256Signer, SoftwareSigner};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// How long [`bootstrap`] waits for discovery+pairing before giving up and
/// falling back to [`MoneyInner::PairingFailed`]. Generous enough for a cold
/// local `tokenfuse-cloud` process to answer, short enough that a genuinely
/// unreachable Cloud (e.g. a stale env-var URL) never hangs app startup.
const PAIRING_TIMEOUT: Duration = Duration::from_secs(10);

/// Where the console events file (`command::record`'s `console_events_path`)
/// and its companion `console.sqlite` live - i.e. the same live-wire events
/// directory `live::bootstrap` seeded, so a `console_command` line lands
/// exactly where the Bus Explorer's `IngestService` is already tailing (see
/// `live.rs`'s module docs). `None` means Phase-0 live-wire startup itself
/// failed (Bus Explorer is on mock data): mutations still reach the Cloud in
/// that case, they just cannot also be journaled locally, which
/// [`super::commands`] reports back to the caller rather than pretending it
/// happened.
#[derive(Debug, Clone)]
pub struct BusHandle {
    pub store_db_path: PathBuf,
    pub console_events_path: PathBuf,
}

/// One of the six demo-seeded NDJSON files `live::bootstrap` always
/// registers as a `FileTail` source (`genaryx_core::demo`'s `SOURCES`).
/// `tokenfuse.ndjson` is picked deliberately: it is guaranteed to exist
/// whenever `live::bootstrap` succeeds, it is thematically the money plane,
/// and - unlike `wardryx`/`verdryx`/`mockryx`/`engram` - the live feeder
/// (`live.rs::feeder_line`) never appends to it itself, so a console-command
/// append here can never race the feeder's own writes to the same file.
const CONSOLE_EVENTS_FILE: &str = "tokenfuse.ndjson";

impl BusHandle {
    pub fn from_events_dir(events_dir: &std::path::Path) -> Self {
        Self {
            store_db_path: events_dir.join("console.sqlite"),
            console_events_path: events_dir.join(CONSOLE_EVENTS_FILE),
        }
    }
}

/// A paired, ready-to-use Cloud connection plus everything
/// [`super::commands`]'s mutation handlers need to build a
/// `genaryx_core::CommandRecord`. Cheap to clone (an `Arc`ed client plus a
/// handful of small strings/paths), so callers can clone it out of the
/// [`MoneyState`] lock and make their (possibly slow) HTTP calls without
/// holding that lock for the duration.
#[derive(Clone)]
pub struct MoneyClient {
    pub client: Arc<CloudClient>,
    pub source: EnvSource,
    pub cloud_url: String,
    /// Sanitized, `agent_id`-safe org label (07 §1 `[a-z0-9.-]+`) the paired
    /// device's `org` resolved to; feeds `command::record`'s `org_domain`.
    pub org_domain: String,
    /// `user://<org_domain>/<local OS user>` - the `on_behalf_of` principal
    /// for every mutation this process issues (there is no separate login
    /// system in the desktop shell; the OS user is the closest honest
    /// identity available).
    pub operator: String,
    pub host: String,
    /// The attached signer's honest assurance label (`"software-signed"`
    /// today; `"secure-enclave"` once a hardware signer is wired in a later
    /// wave) - `CommandRecord::sig_fpr`. Captured at pairing time since
    /// `CloudClient` does not expose the attached device's signer back out.
    pub sig_fpr: &'static str,
    pub bus: Option<BusHandle>,
}

/// The Money panel's whole state machine. `Ready` is the only variant able
/// to serve reads/mutations; the others are honest, displayable reasons it
/// is not (yet, or ever), surfaced to the UI instead of a generic error
/// (spec: "never crash", "leave the money surface in a clean state").
pub enum MoneyInner {
    /// The initial state from [`MoneyState::pending`], until the
    /// background [`bootstrap`] task resolves. Never persists for long, but
    /// is a real, renderable state rather than a gap the frontend has to
    /// paper over with its own "loading" guess.
    Bootstrapping,
    /// [`env::discover`] found nothing usable: no `taipan up` descriptor and
    /// no `TOKENFUSE_CLOUD_ADMIN_KEY`.
    NoEnvironment,
    /// An environment resolved, but building the client, pairing, or
    /// attaching the device failed (Cloud unreachable, pairing rejected,
    /// timed out, ...).
    PairingFailed {
        source: EnvSource,
        cloud_url: String,
        reason: String,
    },
    Ready(MoneyClient),
}

/// Tauri-managed state wrapping [`MoneyInner`] in an async mutex so a later
/// reconnect (not wired to a command yet, but the shape supports one) can
/// atomically swap the whole state, while individual read/mutation commands
/// only hold the lock long enough to clone out a [`MoneyClient`] (see this
/// module's docs) before making any network call.
pub struct MoneyState {
    pub inner: Mutex<MoneyInner>,
    /// Session-local `run_id -> budget_micros` overrides, applied on top of
    /// whatever `/v1/alerts` reveals when building the runs table
    /// (`commands::money_runs`). `CloudClient` (Phase-1 wave 1) does not
    /// wrap a `GET /v1/budgets` read - only the three DTOs its own module
    /// docs list (`summary`/`runs`/`agents`/`savings`/`incidents`/`alerts`/
    /// `audit-verify`) - so a run's budget is otherwise only visible once it
    /// is already at/above its alert threshold. Recording our own successful
    /// `set_budget` calls here means the operator's own actions always show
    /// up immediately, even for a run `/v1/alerts` has not flagged.
    /// Deliberately in-memory only: it resets on restart along with the rest
    /// of this process's Money state, never presented as more durable than
    /// it is.
    pub budget_overrides: Mutex<HashMap<String, i64>>,
}

impl MoneyState {
    /// The synchronous, immediately-manageable starting state (see this
    /// module's docs: `setup` calls this directly, then spawns [`bootstrap`]
    /// to replace it once discovery+pairing resolves).
    #[must_use]
    pub fn pending() -> Self {
        Self {
            inner: Mutex::new(MoneyInner::Bootstrapping),
            budget_overrides: Mutex::new(HashMap::new()),
        }
    }
}

/// Resolve an environment, pair a portable software device, and produce the
/// [`MoneyInner`] the caller should swap into managed state. `events_dir` is
/// the live-wire directory from `live::bootstrap` (`None` if that step
/// itself failed); see [`BusHandle`]. Never panics and never returns an
/// `Err`: every failure mode is a [`MoneyInner`] variant the UI can render.
pub async fn bootstrap(events_dir: Option<PathBuf>) -> MoneyInner {
    let bus = events_dir.map(|dir| BusHandle::from_events_dir(&dir));

    let Some(resolved) = env::discover() else {
        return MoneyInner::NoEnvironment;
    };

    match tokio::time::timeout(PAIRING_TIMEOUT, connect(&resolved)).await {
        Ok(Ok((client, org_domain, sig_fpr))) => {
            let operator = operator_principal(&org_domain);
            MoneyInner::Ready(MoneyClient {
                client: Arc::new(client),
                source: resolved.source,
                cloud_url: resolved.cloud_url,
                org_domain,
                operator,
                host: local_hostname(),
                sig_fpr,
                bus,
            })
        }
        Ok(Err(reason)) => MoneyInner::PairingFailed {
            source: resolved.source,
            cloud_url: resolved.cloud_url,
            reason,
        },
        Err(_elapsed) => MoneyInner::PairingFailed {
            source: resolved.source,
            cloud_url: resolved.cloud_url,
            reason: format!(
                "timed out after {:.0}s waiting for the Cloud to respond",
                PAIRING_TIMEOUT.as_secs_f64()
            ),
        },
    }
}

/// Build a [`CloudClient`], pair a fresh [`SoftwareSigner`] against it, and
/// attach the paired device - the one-time ceremony `bootstrap` runs before
/// the Money panel can serve anything. Returns the client plus the
/// sanitized org domain and the signer's assurance label, both needed by
/// every mutation afterward.
async fn connect(resolved: &ResolvedEnv) -> Result<(CloudClient, String, &'static str), String> {
    let mut client = CloudClient::new(resolved.cloud_url.clone(), resolved.admin_bearer.clone())
        .map_err(|e| e.to_string())?;

    let signer =
        SoftwareSigner::generate().map_err(|e| format!("could not generate a device key: {e}"))?;
    let sig_fpr = signer.assurance().label();

    let paired = client
        .pair(&resolved.admin_bearer, &signer)
        .await
        .map_err(|e| format!("device pairing failed: {e}"))?;
    let org_domain = sanitize_domain(&paired.org);

    client.attach_device(
        paired.device_id.clone(),
        paired.device_token.clone(),
        Box::new(signer),
    );
    Ok((client, org_domain, sig_fpr))
}

/// Fold `org` into the `agent_id`-safe charset `command::console_command_line`
/// requires (07 §1, `^agent://[a-z0-9.-]+/...`): lowercase, everything
/// outside `[a-z0-9.-]` becomes `-`. Mirrors `genaryx_core::command`'s own
/// private `sanitize_host` charset exactly (that function is not `pub`, so
/// this is a two-line, deliberately identical mirror rather than a new
/// convention). Falls back to `"genaryx.local"` if nothing survives (e.g. an
/// empty or entirely non-ASCII org name).
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

/// `user://<org_domain>/<local-user>` - see [`MoneyClient::operator`]. Built
/// from `USER` (falling back to `"operator"`) since the desktop shell has no
/// login system of its own; the OS account running it is the honest
/// principal. `org_domain` is already [`sanitize_domain`]'d by the caller
/// (`connect`'s return value); the username itself is not run through it:
/// the schema only requires an `on_behalf_of` item to start with
/// `agent://`/`user://` (`agent-event.v0.2.schema.json`), so the local
/// username can pass through as-is.
fn operator_principal(org_domain: &str) -> String {
    let user = std::env::var("USER")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| "operator".to_string());
    format!("user://{org_domain}/{user}")
}

/// Best-effort local hostname for `command::record`'s `host` parameter
/// (folded into the emitted `agent_id`, see `console_command_line`'s own
/// sanitizing). Dependency-free by design (no `libc`/`hostname` crate; this
/// task's spec only sanctions adding `genaryx-connectors`/`genaryx-signing`):
/// tries the `HOSTNAME` env var first, then shells out to the `hostname`
/// binary, then falls back to `"localhost"` - never a panic, mirroring
/// `taipan`'s own `util::hostname` fallback contract exactly.
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
        assert_eq!(sanitize_domain("default"), "default");
    }

    #[test]
    fn bus_handle_targets_the_tokenfuse_ndjson_file() {
        let dir = std::path::PathBuf::from("/tmp/some-events-dir");
        let bus = BusHandle::from_events_dir(&dir);
        assert_eq!(bus.store_db_path, dir.join("console.sqlite"));
        assert_eq!(bus.console_events_path, dir.join("tokenfuse.ndjson"));
    }

    #[test]
    fn operator_principal_is_a_conforming_user_principal() {
        let op = operator_principal("acme.example");
        assert!(op.starts_with("user://acme.example/"), "got {op:?}");
    }

    #[test]
    fn pending_starts_in_the_bootstrapping_state() {
        let state = MoneyState::pending();
        let guard = state
            .inner
            .try_lock()
            .expect("uncontended right after construction");
        assert!(matches!(&*guard, MoneyInner::Bootstrapping));
    }

    #[tokio::test]
    async fn bootstrap_never_panics_with_no_environment_available() {
        // This test does not touch `~/.taipan` or env vars; it only proves
        // `bootstrap` resolves to a `MoneyInner` rather than panicking or
        // hanging when `events_dir` is `None` (the Phase-0 live-wire-failed
        // case) - regardless of whether this box happens to have a real
        // `taipan up` environment or `TOKENFUSE_CLOUD_ADMIN_KEY` set, every
        // one of the resulting variants is a valid, renderable outcome.
        let inner = bootstrap(None).await;
        match inner {
            MoneyInner::Bootstrapping => {
                panic!("bootstrap must resolve past its own pending state")
            }
            MoneyInner::NoEnvironment | MoneyInner::PairingFailed { .. } | MoneyInner::Ready(_) => {
            }
        }
    }

    // ==========================================================================
    // live e2e: real tokenfuse-cloud, real pairing, a real signed mutation,
    // a real console_command appended and re-read back off disk.
    // ==========================================================================
    // Same gated, hermetic, single-test-function shape as
    // `crates/connectors/tests/cloud_rest_test.rs` (builds `tokenfuse-cloud`
    // from `~/Development/tokenfuse` with `TOKENFUSE_CLOUD_ALLOW_DEVKEY=1` on
    // a fresh ephemeral port, torn down after), reused here rather than
    // reimplemented from scratch. `env::discover` itself is already fully
    // covered by `env.rs`'s own fixture-based tests (no live server needed
    // for filesystem/JSON logic) so this exercises exactly the two things
    // that DO need one: `connect` (this module's pairing ceremony) and the
    // `CommandRecord` -> `command::record` journal path this task's spec
    // calls out by name.

    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::time::Instant;

    struct ChildGuard(Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn free_port() -> Option<u16> {
        TcpListener::bind("127.0.0.1:0")
            .ok()
            .and_then(|l| l.local_addr().ok())
            .map(|a| a.port())
    }

    fn tokenfuse_repo() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok()?;
        let dir = PathBuf::from(home).join("Development/tokenfuse");
        dir.join("Cargo.toml").is_file().then_some(dir)
    }

    fn build_and_spawn(repo: &std::path::Path, port: u16) -> Option<Child> {
        let build = Command::new("cargo")
            .args(["build", "--quiet", "-p", "tokenfuse-cloud"])
            .current_dir(repo)
            .status();
        match build {
            Ok(status) if status.success() => {}
            _ => {
                eprintln!("money::state live_e2e: SKIPPING: could not build tokenfuse-cloud");
                return None;
            }
        }
        let binary = repo.join("target/debug/tokenfuse-cloud");
        if !binary.is_file() {
            eprintln!(
                "money::state live_e2e: SKIPPING: {} is missing",
                binary.display()
            );
            return None;
        }
        Command::new(&binary)
            .env("TOKENFUSE_CLOUD_ALLOW_DEVKEY", "1")
            .env("PORT", port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()
    }

    async fn try_start_cloud() -> Option<(ChildGuard, String)> {
        let Some(repo) = tokenfuse_repo() else {
            eprintln!("money::state live_e2e: SKIPPING: ~/Development/tokenfuse not found");
            return None;
        };
        let Some(port) = free_port() else {
            eprintln!("money::state live_e2e: SKIPPING: could not reserve a port");
            return None;
        };
        let mut child = build_and_spawn(&repo, port)?;

        let base = format!("http://127.0.0.1:{port}");
        let http = reqwest::Client::new();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                eprintln!(
                    "money::state live_e2e: SKIPPING: tokenfuse-cloud exited early ({status})"
                );
                return None;
            }
            if let Ok(resp) = http.get(format!("{base}/healthz")).send().await
                && resp.status().is_success()
            {
                return Some((ChildGuard(child), base));
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                eprintln!(
                    "money::state live_e2e: SKIPPING: tokenfuse-cloud never answered /healthz"
                );
                return None;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    #[tokio::test]
    async fn live_e2e_connect_summary_signed_kill_and_console_command_journal() {
        let Some((_guard, base)) = try_start_cloud().await else {
            return; // already explained why via eprintln! above
        };

        // ---- connect(): this module's own pairing ceremony, against a real
        // Cloud, using the devkey fallback (org "default") exactly the way
        // `env::discover_env_fallback` would resolve a locally-started Cloud. ----
        let resolved = ResolvedEnv {
            source: EnvSource::EnvFallback,
            cloud_url: base.clone(),
            admin_bearer: "devkey".to_string(),
        };
        let (client, org_domain, sig_fpr) = connect(&resolved)
            .await
            .expect("connect() must pair against a live Cloud");
        assert_eq!(
            org_domain, "default",
            "devkey fallback resolves org=default (unsanitized already-safe)"
        );
        assert_eq!(sig_fpr, "software-signed");
        assert!(client.has_device());

        // ---- a real read ----
        let summary = client.summary().await.expect("GET /v1/summary");
        assert_eq!(
            summary.runs, 0,
            "a freshly spawned process has an empty org view"
        );

        // ---- a real signed mutation ----
        let run_id = format!("money-state-live-e2e-{}", std::process::id());
        let killed = client
            .kill_run(&run_id)
            .await
            .expect("signed kill_run must be accepted (200)");
        assert_eq!(killed.killed, run_id);

        // ---- the exact CommandRecord shape commands.rs::finish_mutation
        // builds for a successful kill, journaled through the same public
        // genaryx_core::command::record entry point the Money panel's
        // mutation commands call. A kill is break-glass (Phase-2 wave 3B), so
        // `params` must carry the same non-empty "reason" money_kill_run
        // itself now requires before it ever calls the Cloud - the core's own
        // `require_break_glass_reason` refuses to journal a break_glass
        // record without one. ----
        let rec = genaryx_core::CommandRecord {
            operator: operator_principal(&org_domain),
            env: "local".to_string(),
            action: "console.kill_run".to_string(),
            target: run_id.clone(),
            params: serde_json::json!({ "reason": "live-e2e test kill" }),
            decision: "break_glass".to_string(),
            sig_alg: "es256".to_string(),
            sig_fpr: sig_fpr.to_string(),
            http_status: 200,
            verify_result: format!("killed:{}", killed.killed == run_id),
        };

        let scratch_dir = std::env::temp_dir().join(format!(
            "genaryx-money-state-live-e2e-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&scratch_dir).expect("create scratch dir");
        let db_path = scratch_dir.join("console.sqlite");
        let events_path = scratch_dir.join("tokenfuse.ndjson");
        let store = genaryx_core::store::Store::open(&db_path).expect("open scratch store");

        genaryx_core::command::record(&store, &events_path, &org_domain, "live-e2e-host", &rec)
            .expect("command::record must journal + append the console_command line");

        let body =
            std::fs::read_to_string(&events_path).expect("read the console events file back");
        let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 1, "exactly one console_command line appended");

        let conformer = genaryx_core::Conformer::new().expect("embedded schemas must compile");
        let report = conformer.check_line(lines[0]);
        assert!(
            report.valid,
            "appended console_command must conform: {:?}\n  line: {}",
            report.errors, lines[0]
        );

        let value: serde_json::Value =
            serde_json::from_str(lines[0]).expect("parse the appended line");
        assert_eq!(
            value.get("source").and_then(|v| v.as_str()),
            Some("console")
        );
        assert_eq!(
            value.get("type").and_then(|v| v.as_str()),
            Some("console_command")
        );
        assert_eq!(
            value
                .get("data")
                .and_then(|d| d.get("verify_result"))
                .and_then(|v| v.as_str()),
            Some("killed:true")
        );

        eprintln!(
            "money::state live_e2e: PASSED - paired against {base}, summary read, signed kill of {run_id} \
             accepted, console_command appended to {} and conforms",
            events_path.display()
        );

        let _ = std::fs::remove_dir_all(&scratch_dir);
    }
}
