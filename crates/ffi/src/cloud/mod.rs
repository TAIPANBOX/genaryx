//! `CloudHandle`: the UniFFI Object wrapping `genaryx_connectors::CloudClient`
//! for the SwiftUI Money + Overview surface (docs/PHASE1.md wave 3), at
//! parity with the Tauri shell's `money` module (commit b2a1eff): same
//! environment discovery ([`env`]), same connect-then-pair ceremony, same
//! fail-closed mutation contract (every mutation ALWAYS attempts
//! `genaryx_core::command::record`, even when the Cloud call itself failed -
//! see [`CloudHandle::finish_mutation`]).
//!
//! ## Async-to-sync: one `tokio::runtime::Runtime` owned by the Object
//!
//! `CloudClient`'s methods are `async fn`; every UniFFI-exported method here
//! is synchronous (F-04, docs/PHASE0.md). Unlike `FleetHandle` (a background
//! ingest thread pushing events through a callback), a Money read or
//! mutation is a plain request/response, so `CloudHandle` builds one
//! multi-thread `tokio::runtime::Runtime` in its constructor and keeps it for
//! the handle's whole lifetime, calling `self.runtime.block_on(...)` per
//! exported method. Multi-thread (not current-thread, unlike
//! `cloud_sse.rs`'s single dedicated background loop) specifically so
//! concurrent calls from more than one Swift caller thread (e.g. an Overview
//! refresh racing a Money-panel kill, both dispatched via `Task.detached`)
//! never contend for one exclusive `block_on` slot - a `current_thread`
//! runtime only supports one `block_on` in flight at a time.
//!
//! ## The console_command journal
//!
//! The constructor also seeds a small, throwaway Store + events file (the
//! same temp-dir shape `FleetHandle::new` seeds its demo world from, minus
//! the demo NDJSON itself - `command::record` only needs a writable
//! `commands_journal` table and an appendable events file, not pre-existing
//! content) so every mutation can journal a `console_command`, exactly like
//! `apps/desktop/src-tauri/src/money/commands.rs::journal` does. This is a
//! *separate* temp world from any `FleetHandle` the same process also holds
//! (disambiguated with a `-cloud-` infix, see [`fresh_world_dir`]): the two
//! Objects are independent UniFFI handles with independent lifetimes, so a
//! `console_command` journaled here lands on its own bus rather than the
//! Bus Explorer's, which is an accepted trade-off for this wave (see the
//! task report's "anything the lead should double-check").
//!
//! ## Break-glass (Phase-2 wave 3B)
//!
//! [`CloudHandle::kill_run`] and [`CloudHandle::set_budget`] are the two
//! genuinely-privileged mutations here: with no Wardryx precheck wired in
//! yet, each is honestly journaled as `decision: "break_glass"` (an
//! operator override of governance, not an automated `"allow"`) and each
//! REQUIRES a non-empty, operator-typed `reason` - rejected client-side,
//! before the Cloud is ever called (see [`require_break_glass_reason`]),
//! and rejected again, independently, at journal time by
//! `genaryx_core::command::require_break_glass_reason` if it somehow got
//! this far anyway. [`CloudHandle::ack_incident`] is a low-stakes
//! acknowledgment rather than an override, so it journals
//! `decision: "allow"` and takes no `reason` at all. See
//! [`CloudHandle::finish_mutation`] for where `decision` is threaded
//! through.
//!
//! Fail-closed at the boundary (06 §0.5): nothing here panics across FFI.

pub mod dto;
pub mod env;

pub use dto::{CloudError, Incident, MutationOutcome, Overview, Run, Savings};
pub use env::EnvSource;

use dto::{build_run, status_of};
use env::ResolvedEnv;
use genaryx_connectors::{CloudClient, ConnectorError};
use genaryx_core::CommandRecord;
use genaryx_core::command;
use genaryx_core::store::Store;
use genaryx_signing::{Es256Signer, SoftwareSigner};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

/// Lock a poisoned-or-not mutex without panicking (mirrors `lib.rs::relock`;
/// kept as its own copy since the two live in sibling modules and this one
/// only ever guards [`CloudHandle`]'s `budget_overrides` map).
fn relock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Fail-closed guard for the two break-glass mutations, `kill_run` and
/// `set_budget` (Phase-2 wave 3B): reject an empty/whitespace `reason`
/// BEFORE `kill_run`/`set_budget` ever call the Cloud, so an unjustified
/// override never mutates anything Cloud-side, let alone reaches
/// `genaryx_core::command::require_break_glass_reason`'s own, later,
/// journal-time refusal (`crates/core/src/command.rs`). That core guard
/// stays the authoritative one (it is what actually decides whether a
/// `commands_journal` row gets written); this is defense in depth at the
/// ffi boundary, not a replacement for it.
fn require_break_glass_reason(reason: &str) -> Result<(), CloudError> {
    if reason.trim().is_empty() {
        return Err(CloudError::BreakGlassReasonRequired);
    }
    Ok(())
}

/// The Money + Overview UniFFI Object: a paired [`CloudClient`] plus
/// everything a mutation needs to journal a `console_command`. See the
/// module docs for the async-to-sync bridge and the journal shape.
#[derive(uniffi::Object)]
pub struct CloudHandle {
    runtime: tokio::runtime::Runtime,
    client: CloudClient,
    source: EnvSource,
    cloud_url: String,
    /// Sanitized, `agent_id`-safe org label (07 §1 `[a-z0-9.-]+`) the paired
    /// device's `org` resolved to; feeds `command::record`'s `org_domain`.
    org_domain: String,
    /// `user://<org_domain>/<local OS user>` - the `on_behalf_of` principal
    /// for every mutation this process issues (there is no separate login
    /// system in this shell either; the OS user is the closest honest
    /// identity available).
    operator: String,
    host: String,
    /// The attached signer's honest assurance label (`"software-signed"`
    /// today) - `CommandRecord::sig_fpr`. Captured at pairing time since
    /// `CloudClient` does not expose the attached device's signer back out.
    sig_fpr: &'static str,
    store_db_path: PathBuf,
    console_events_path: PathBuf,
    /// Session-local `run_id -> budget_micros` overrides, applied on top of
    /// whatever `/v1/alerts` reveals when building the runs table - mirrors
    /// `MoneyState::budget_overrides` exactly: `CloudClient` does not wrap a
    /// `GET /v1/budgets` read, so a run's budget is otherwise only visible
    /// once it is already at/above its alert threshold. Deliberately
    /// in-memory only: it resets when this handle is dropped, never
    /// presented as more durable than it is.
    budget_overrides: Mutex<HashMap<String, i64>>,
    /// Temp world root (the Store + events file above), removed on drop
    /// (best effort).
    dir: PathBuf,
}

#[uniffi::export]
impl CloudHandle {
    /// Discover which TokenFuse Cloud to talk to (a `taipan up` descriptor
    /// under `~/.taipan/environments/`, or `TOKENFUSE_CLOUD_ADMIN_KEY` for a
    /// Cloud started by hand) and pair a fresh software device against it.
    /// Fails closed with [`CloudError::NoEnvironment`] when neither source
    /// resolves - a normal, renderable "no environment" outcome, not a bug.
    #[uniffi::constructor]
    pub fn discover() -> Result<Self, CloudError> {
        let resolved = env::discover().ok_or(CloudError::NoEnvironment)?;
        Self::build(resolved)
    }

    /// Connect directly to `cloud_url` with `admin_key`, skipping
    /// discovery - for a Cloud the caller already knows how to reach (an
    /// operator-entered value, or a test harness).
    #[uniffi::constructor]
    pub fn connect(cloud_url: String, admin_key: String) -> Result<Self, CloudError> {
        Self::build(ResolvedEnv {
            source: EnvSource::EnvFallback,
            cloud_url,
            admin_bearer: admin_key,
        })
    }

    /// Where this handle resolved its environment from.
    pub fn source(&self) -> EnvSource {
        self.source.clone()
    }

    /// The Cloud base URL this handle is paired against.
    pub fn cloud_url(&self) -> String {
        self.cloud_url.clone()
    }

    /// The paired device's sanitized org domain.
    pub fn org_domain(&self) -> String {
        self.org_domain.clone()
    }

    // ---- reads --------------------------------------------------------

    /// Summary + a few derived tiles (active runs, open incidents, total
    /// saved) - one call from the Swift side, four concurrent Cloud reads
    /// underneath.
    pub fn overview(&self) -> Result<Overview, CloudError> {
        let (summary, runs, incidents, savings) = self.runtime.block_on(async {
            tokio::try_join!(
                self.client.summary(),
                self.client.runs(),
                self.client.incidents(),
                self.client.savings(),
            )
        })?;
        Ok(Overview::build(&summary, &runs, &incidents, &savings))
    }

    /// The runs table. Budget is enriched from `GET /v1/alerts` (the only
    /// connector read that carries `budget_micros`) overlaid with any
    /// budget this session itself has set - see
    /// [`CloudHandle::budget_overrides`].
    pub fn runs(&self) -> Result<Vec<Run>, CloudError> {
        let (runs, alerts) = self
            .runtime
            .block_on(async { tokio::try_join!(self.client.runs(), self.client.alerts()) })?;

        let alert_budgets: HashMap<&str, i64> = alerts
            .iter()
            .map(|a| (a.run_id.as_str(), a.budget_micros))
            .collect();
        let overrides = relock(&self.budget_overrides);

        Ok(runs
            .iter()
            .map(|r| {
                let budget_micros = overrides
                    .get(&r.run_id)
                    .copied()
                    .or_else(|| alert_budgets.get(r.run_id.as_str()).copied());
                build_run(r, budget_micros)
            })
            .collect())
    }

    pub fn incidents(&self) -> Result<Vec<Incident>, CloudError> {
        let incidents = self.runtime.block_on(self.client.incidents())?;
        Ok(incidents.iter().map(Incident::from).collect())
    }

    pub fn savings(&self) -> Result<Savings, CloudError> {
        let savings = self.runtime.block_on(self.client.savings())?;
        Ok(Savings::from(&savings))
    }

    // ---- signed mutations ----------------------------------------------
    // Every mutation below ALWAYS attempts to journal a `console_command`,
    // even when the Cloud call itself failed or was rejected - see
    // `finish_mutation`'s doc.
    //
    // `kill_run`/`set_budget` are the two genuinely-privileged mutations:
    // there is no Wardryx precheck yet (no automated `allow`/`deny`
    // decision precedes them), so both are honestly journaled as
    // `decision: "break_glass"` - an operator override of governance - and
    // Phase-2 wave 3B requires each to carry a non-empty, operator-typed
    // `reason` in `params` before `genaryx_core::command::record` will
    // journal anything at all
    // (`crates/core/src/command.rs::require_break_glass_reason`). Both fail
    // closed on an empty/whitespace `reason` BEFORE ever calling the Cloud
    // (see [`require_break_glass_reason`] below) - defense in depth on top
    // of that core guard, not a replacement for it. `ack_incident` is a
    // low-stakes acknowledgment, not an operator override, so it journals
    // `decision: "allow"` and takes no `reason`.

    pub fn kill_run(&self, run_id: String, reason: String) -> Result<MutationOutcome, CloudError> {
        require_break_glass_reason(&reason)?;
        let result = self.runtime.block_on(self.client.kill_run(&run_id));
        self.finish_mutation(
            "console.kill_run",
            &run_id,
            "break_glass",
            json!({ "reason": reason }),
            result,
            |resp| {
                (
                    format!("run {run_id} killed"),
                    format!("killed:{}", resp.killed == run_id),
                )
            },
        )
    }

    pub fn set_budget(
        &self,
        run_id: String,
        budget_usd: f64,
        reason: String,
    ) -> Result<MutationOutcome, CloudError> {
        require_break_glass_reason(&reason)?;
        let result = self
            .runtime
            .block_on(self.client.set_budget(&run_id, budget_usd));

        if let Ok(resp) = &result {
            relock(&self.budget_overrides).insert(run_id.clone(), resp.budget_micros);
        }

        self.finish_mutation(
            "console.set_budget",
            &run_id,
            "break_glass",
            json!({ "reason": reason, "budget_usd": budget_usd }),
            result,
            |resp| {
                (
                    format!("run {run_id} budget set to ${budget_usd:.4}"),
                    format!("budget_micros:{}", resp.budget_micros),
                )
            },
        )
    }

    pub fn ack_incident(&self, id: String) -> Result<MutationOutcome, CloudError> {
        let result = self.runtime.block_on(self.client.ack_incident(&id));
        self.finish_mutation(
            "console.ack_incident",
            &id,
            "allow",
            json!({}),
            result,
            |resp| {
                (
                    format!("incident {id} acknowledged"),
                    format!("acknowledged:{}", resp.acknowledged == id),
                )
            },
        )
    }
}

// ---- private helpers (not exported over FFI) -------------------------------

impl CloudHandle {
    /// Shared constructor body: connect + pair (see [`connect_and_pair`]),
    /// then seed a small local Store + events file so mutations can journal
    /// a `console_command`. Never panics; every fallible step folds into a
    /// [`CloudError`].
    fn build(resolved: ResolvedEnv) -> Result<Self, CloudError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| CloudError::PairingFailed {
                reason: format!("could not start async runtime: {e}"),
            })?;

        let (client, org_domain, sig_fpr) = runtime
            .block_on(connect_and_pair(&resolved))
            .map_err(|reason| CloudError::PairingFailed { reason })?;

        let operator = operator_principal(&org_domain);
        let host = local_hostname();

        let dir = fresh_world_dir().map_err(fs_error)?;
        let events_dir = dir.join("events");
        std::fs::create_dir_all(&events_dir).map_err(fs_error)?;
        let store_db_path = dir.join("console.sqlite");
        // Opened once here to run migrations / prove the store is writable;
        // `journal` reopens per call, matching commands.rs's own per-call
        // `Store::open` pattern in the Tauri shell.
        Store::open(&store_db_path).map_err(|e| CloudError::Cloud {
            status: None,
            message: e.to_string(),
        })?;
        let console_events_path = events_dir.join("tokenfuse.ndjson");

        Ok(Self {
            runtime,
            client,
            source: resolved.source,
            cloud_url: resolved.cloud_url,
            org_domain,
            operator,
            host,
            sig_fpr,
            store_db_path,
            console_events_path,
            budget_overrides: Mutex::new(HashMap::new()),
            dir,
        })
    }

    /// Journal one `CommandRecord` (best-effort: a journal failure is
    /// reported, never panics and never blocks the caller from learning the
    /// Cloud's own verdict).
    fn journal(&self, rec: &CommandRecord) -> (bool, Option<String>) {
        match Store::open(&self.store_db_path) {
            Ok(store) => {
                match command::record(
                    &store,
                    &self.console_events_path,
                    &self.org_domain,
                    &self.host,
                    rec,
                ) {
                    Ok(()) => (true, None),
                    Err(e) => (false, Some(e.to_string())),
                }
            }
            Err(e) => (false, Some(e.to_string())),
        }
    }

    /// Shared tail end of every mutation: build the `CommandRecord` from the
    /// already-resolved Cloud outcome, ALWAYS attempt to journal it
    /// (regardless of that outcome - a rejected privileged attempt is
    /// itself part of the audit trail), then fold everything into either a
    /// [`MutationOutcome`] or a [`CloudError`] for the caller. Mirrors
    /// `money::commands::finish_mutation`, plus `decision` (Phase-2 wave
    /// 3B): `kill_run`/`set_budget` pass `"break_glass"` (no Wardryx
    /// precheck exists yet, so both are honestly an operator override,
    /// never an automated "allow"); `ack_incident` passes `"allow"` (a
    /// low-stakes acknowledgment, not a governance override).
    fn finish_mutation<T>(
        &self,
        action: &'static str,
        target: &str,
        decision: &'static str,
        params: Value,
        cloud_result: Result<T, ConnectorError>,
        on_ok: impl FnOnce(&T) -> (String, String),
    ) -> Result<MutationOutcome, CloudError> {
        let (http_status, verify_result, summary) = match &cloud_result {
            Ok(value) => {
                let (summary, verify_result) = on_ok(value);
                (200u16, verify_result, summary)
            }
            Err(e) => (status_of(e), format!("error: {e}"), String::new()),
        };

        let rec = CommandRecord {
            operator: self.operator.clone(),
            env: "local".to_string(),
            action: action.to_string(),
            target: target.to_string(),
            params,
            decision: decision.to_string(),
            sig_alg: "es256".to_string(),
            sig_fpr: self.sig_fpr.to_string(),
            http_status,
            verify_result: verify_result.clone(),
        };
        let (bus_recorded, bus_error) = self.journal(&rec);

        match cloud_result {
            Ok(_) => Ok(MutationOutcome {
                summary,
                http_status,
                verify_result,
                sig_alg: "es256".to_string(),
                sig_fpr: self.sig_fpr.to_string(),
                bus_recorded,
                bus_error,
            }),
            Err(e) => Err(CloudError::from(e)),
        }
    }
}

impl Drop for CloudHandle {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Build a [`CloudClient`], pair a fresh [`SoftwareSigner`] against it, and
/// attach the paired device - the one-time ceremony [`CloudHandle::build`]
/// runs before the handle can serve anything. Mirrors
/// `apps/desktop/src-tauri/src/money/state.rs::connect` exactly (same order
/// of operations, same error wording), so a `PairingFailed` reason looks
/// identical to an operator regardless of which shell they are using.
async fn connect_and_pair(
    resolved: &ResolvedEnv,
) -> Result<(CloudClient, String, &'static str), String> {
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
/// requires (07 §1, `^agent://[a-z0-9.-]+/...`). Mirrors
/// `money::state::sanitize_domain` exactly.
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

/// `user://<org_domain>/<local-user>`. Mirrors `money::state::operator_principal`.
fn operator_principal(org_domain: &str) -> String {
    let user = std::env::var("USER")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| "operator".to_string());
    format!("user://{org_domain}/{user}")
}

/// Best-effort local hostname, dependency-free by design. Mirrors
/// `money::state::local_hostname` exactly.
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

/// A unique, collision-proof temp directory for one handle's small events
/// world: pid + per-process counter + nanos. Same shape as `lib.rs`'s
/// `fresh_world_dir`, disambiguated with a `-cloud-` infix so a `FleetHandle`
/// and a `CloudHandle` constructed in the same process never collide.
fn fresh_world_dir() -> std::io::Result<PathBuf> {
    static INSTANCE: AtomicU64 = AtomicU64::new(0);
    let n = INSTANCE.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-ffi-cloud-{}-{n}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn fs_error(e: std::io::Error) -> CloudError {
    CloudError::Cloud {
        status: None,
        message: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    /// Rust-side stand-in proving `CloudHandle` never panics when
    /// discovery finds nothing - the far more common case in CI than a live
    /// Cloud being available at all.
    #[test]
    fn discover_without_an_environment_is_a_clean_error_not_a_panic() {
        // Does not touch `~/.taipan` or env vars; only proves the
        // `Result` shape, regardless of whether this box happens to have a
        // real `taipan up` environment or `TOKENFUSE_CLOUD_ADMIN_KEY` set
        // (either a `NoEnvironment`/`PairingFailed` error or a genuine
        // `Ok` are all valid, non-panicking outcomes).
        match CloudHandle::discover() {
            Ok(_) | Err(CloudError::NoEnvironment | CloudError::PairingFailed { .. }) => {}
            Err(other) => panic!("unexpected error shape from discover(): {other:?}"),
        }
    }

    #[test]
    fn connect_to_an_unreachable_url_fails_closed() {
        // `CloudHandle` deliberately has no `Debug` impl (it holds a live
        // `CloudClient`/runtime, not inert data), so this is a plain `match`
        // rather than `.expect_err(...)` (which would require `T: Debug` on
        // the `Ok` side too).
        match CloudHandle::connect("http://127.0.0.1:1".to_string(), "devkey".to_string()) {
            Err(err @ CloudError::PairingFailed { .. }) => drop(err),
            Err(other) => panic!("expected PairingFailed, got {other:?}"),
            Ok(_) => panic!("port 1 must not have a Cloud listening"),
        }
    }

    // ==========================================================================
    // live e2e: real tokenfuse-cloud, real pairing, a real signed mutation, a
    // real console_command appended and re-read back off disk.
    // ==========================================================================
    // Same gated, hermetic, single-test-function shape as
    // `crates/connectors/tests/cloud_rest_test.rs` and `money::state`'s own
    // live_e2e test (builds `tokenfuse-cloud` from `~/Development/tokenfuse`
    // with `TOKENFUSE_CLOUD_ALLOW_DEVKEY=1` on a fresh ephemeral port, torn
    // down after), reused here rather than reimplemented from scratch. The
    // readiness probe is a plain TCP connect rather than an HTTP `/healthz`
    // GET (unlike the other two): `genaryx-ffi` has no HTTP client
    // dependency of its own, and this test should not add one just to poll
    // readiness when a connect-then-grace-sleep is good enough for a local
    // spawned process.

    struct ChildGuard(Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn free_port() -> Option<u16> {
        std::net::TcpListener::bind("127.0.0.1:0")
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
                eprintln!("genaryx-ffi cloud live_e2e: SKIPPING: could not build tokenfuse-cloud");
                return None;
            }
        }
        let binary = repo.join("target/debug/tokenfuse-cloud");
        if !binary.is_file() {
            eprintln!(
                "genaryx-ffi cloud live_e2e: SKIPPING: {} is missing",
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

    /// Stand up a real `tokenfuse-cloud` on an ephemeral port and wait for
    /// it to start accepting TCP connections, plus a short grace sleep so
    /// the server has finished route setup before the real test traffic
    /// starts.
    fn try_start_cloud() -> Option<(ChildGuard, String)> {
        let Some(repo) = tokenfuse_repo() else {
            eprintln!("genaryx-ffi cloud live_e2e: SKIPPING: ~/Development/tokenfuse not found");
            return None;
        };
        let Some(port) = free_port() else {
            eprintln!("genaryx-ffi cloud live_e2e: SKIPPING: could not reserve a port");
            return None;
        };
        let mut child = build_and_spawn(&repo, port)?;

        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                eprintln!(
                    "genaryx-ffi cloud live_e2e: SKIPPING: tokenfuse-cloud exited early ({status})"
                );
                return None;
            }
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                std::thread::sleep(Duration::from_millis(300));
                return Some((ChildGuard(child), format!("http://127.0.0.1:{port}")));
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                eprintln!(
                    "genaryx-ffi cloud live_e2e: SKIPPING: tokenfuse-cloud never opened its port"
                );
                return None;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    #[test]
    fn live_e2e_connect_overview_signed_kill_and_console_command_journal() {
        let Some((_guard, base)) = try_start_cloud() else {
            return; // already explained why via eprintln! above
        };

        let handle = CloudHandle::connect(base.clone(), "devkey".to_string())
            .expect("CloudHandle::connect must pair against a live Cloud");
        assert_eq!(
            handle.org_domain(),
            "default",
            "devkey fallback resolves org=default"
        );
        assert_eq!(handle.cloud_url(), base);
        assert!(matches!(handle.source(), EnvSource::EnvFallback));

        // ---- a real read ----
        let overview = handle
            .overview()
            .expect("overview() must read a real summary/runs/incidents/savings");
        assert_eq!(
            overview.total_runs, 0,
            "a freshly spawned process has an empty org view"
        );

        // ---- a real signed mutation ----
        // `kill_run` is a break-glass override (Phase-2 wave 3B): a
        // non-empty `reason` is required, both by `CloudHandle` itself
        // (`require_break_glass_reason`, checked before this call ever
        // reaches the Cloud) and, again, by
        // `genaryx_core::command::require_break_glass_reason` at journal
        // time - an empty reason here would make this call fail before any
        // network traffic at all.
        let run_id = format!("genaryx-ffi-cloud-live-e2e-{}", std::process::id());
        let reason = "genaryx-ffi live_e2e: proving the break-glass kill path end to end";
        let outcome = handle
            .kill_run(run_id.clone(), reason.to_string())
            .expect("signed kill_run must be accepted (200)");
        assert_eq!(outcome.http_status, 200);
        assert_eq!(outcome.verify_result, "killed:true");
        assert!(
            outcome.bus_recorded,
            "console_command must be journaled: {:?}",
            outcome.bus_error
        );

        // ---- confirm the console_command line actually landed and conforms ----
        let body = std::fs::read_to_string(&handle.console_events_path)
            .expect("read the console events file back");
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
            "genaryx-ffi cloud live_e2e: PASSED - paired against {base}, overview read, signed kill of \
             {run_id} accepted, console_command appended to {} and conforms",
            handle.console_events_path.display()
        );
    }
}
