//! `IdryxHandle`: the UniFFI Object wrapping `genaryx_connectors::IdryxClient`
//! for the SwiftUI Identity surface (docs/PHASE3.md wave 2, "Track B
//! `crates/ffi/src/idryx/`"), at parity with the Tauri shell's own
//! `src-tauri/src/identity/` (the sibling Track A). Structurally this mirrors
//! [`crate::wardryx::WardryxHandle`] (same owned-runtime async-to-sync
//! bridge - one `tokio::runtime::Runtime` built once in the constructor,
//! `block_on` per read), but MUCH SIMPLER, for one reason: Idryx `serve` has
//! **no authentication of any kind** and this whole panel is **read-only**
//! (docs/PHASE3.md's "Grounded Idryx contract": `SECURITY.md:121-123`,
//! `server.go:78,207,242` - every handler discards the `*http.Request`
//! entirely). So this handle carries:
//!
//! - no bearer, no signer, no pairing/device ceremony (`IdryxHandle::build`
//!   is synchronous end to end, exactly like `WardryxHandle::build` - neither
//!   `IdryxClient::new` nor `WardryxClient::new` ever touches the network);
//! - no `org_domain` / `operator` principal (there is no `console_command` to
//!   attribute - see the next point);
//! - **no journal at all**: unlike every other Object in this crate,
//!   `IdryxHandle` never calls `genaryx_core::command::record`, seeds no
//!   temp Store/events world, and needs no `Drop` impl to clean one up (there
//!   is no temp world to clean up). Every exported method here is a plain
//!   read (`list_identities`, `list_alerts`, `list_remediations`) or a
//!   read-shaped recompute (`rescan` - see below); none of them changes
//!   state on any system this console does not itself own, so there is
//!   nothing to audit. PHASE3.md's W2 scope is explicit about this: "drop
//!   all of that machinery `WardryxHandle` carries."
//!
//! ## `rescan`: a subprocess call, not an HTTP mutation
//!
//! Idryx `serve` is a **load-once immutable snapshot** (`server.go:16-17`,
//! quoted in PHASE3.md: "no file-watch, no SIGHUP, no reload endpoint, no
//! polling, no TTL"): polling any `/api` route returns byte-identical data
//! for the process lifetime. [`IdryxHandle::rescan`] is how the panel picks up new
//! findings without restarting idryx: it shells out to
//! `idryx detect --format json` over the CURRENT stack bus files
//! (`genaryx_connectors::IdryxClient::rescan`, a synchronous, non-`self`
//! associated function - no `block_on` needed here, unlike the REST reads
//! below). Both the idryx binary path and the `--load source:path` inputs
//! are resolved fresh on every call ([`env::resolve_rescan_inputs`]), not
//! cached at construction - see that function's own doc for why. When either
//! piece cannot be located, this returns an honest
//! [`IdryxError::RescanUnavailable`] - never a fake empty `Vec` dressed up as
//! "no alerts found" (PHASE3.md W2: "if the binary is not found, return an
//! honest IdryxError variant, never a fake empty success").
//!
//! ## Attestation is never a field
//!
//! [`dto::IdentityRecord`] carries no `attestation` field, on purpose: the
//! wire contract has none (PHASE3.md: "`model.Identity.Attestation` ... is
//! used only internally by the `attestation_missing` detector, which embeds
//! `attestation=<value>` as free text inside `apiAlert.Summary`"). The
//! SwiftUI panel derives attestation status from `attestation_missing` /
//! `bom_incomplete` rows in [`IdryxHandle::list_alerts`] instead - never
//! invented here.
//!
//! Fail-closed at the boundary (06 §0.5): nothing here panics across FFI.

pub mod dto;
pub mod env;

pub use dto::{AlertRecord, IdentityRecord, IdryxError, RemediationRecord};
pub use env::IdryxEnvSource;

use env::ResolvedEnv;
use genaryx_connectors::IdryxClient;

/// The Identity UniFFI Object: an unauthenticated, read-only [`IdryxClient`]
/// plus the environment it resolved from. See the module doc for why this
/// carries none of `WardryxHandle`'s journal/signer/operator machinery.
#[derive(uniffi::Object)]
pub struct IdryxHandle {
    runtime: tokio::runtime::Runtime,
    client: IdryxClient,
    source: IdryxEnvSource,
    idryx_url: String,
}

#[uniffi::export]
impl IdryxHandle {
    /// Discover which Idryx identity plane to talk to: a `taipan up`
    /// descriptor's `services.idryx.url` (07 §4.4), or `IDRYX_URL` for an
    /// idryx started by hand. Fails closed with [`IdryxError::NoEnvironment`]
    /// when neither resolves - a normal, renderable "no identity plane"
    /// outcome (PHASE3.md: "No-idryx environment renders a clean empty
    /// state, not an error"), not a bug.
    #[uniffi::constructor]
    pub fn discover() -> Result<Self, IdryxError> {
        let resolved = env::discover().ok_or(IdryxError::NoEnvironment)?;
        Self::build(resolved)
    }

    /// Connect directly to `idryx_url`, skipping discovery - for an Idryx the
    /// caller already knows how to reach (an operator-entered value, or a
    /// test harness). No bearer/key parameter at all: idryx has no auth (see
    /// the module doc).
    #[uniffi::constructor]
    pub fn connect(idryx_url: String) -> Result<Self, IdryxError> {
        Self::build(ResolvedEnv {
            source: IdryxEnvSource::EnvFallback,
            idryx_url,
        })
    }

    /// Where this handle resolved its environment from.
    pub fn source(&self) -> IdryxEnvSource {
        self.source.clone()
    }

    /// The Idryx base URL this handle talks to.
    pub fn idryx_url(&self) -> String {
        self.idryx_url.clone()
    }

    // ---- reads (the whole surface - Identity is read-only, 09 §Ф3) --------

    /// `GET /api/identities` - the load-once snapshot, flattened into
    /// [`IdentityRecord`]s. The SwiftUI panel labels this "as of load", never
    /// implied live (PHASE3.md: "serve is LOAD-ONCE... polling any `/api`
    /// route returns byte-identical data for the process lifetime").
    pub fn list_identities(&self) -> Result<Vec<IdentityRecord>, IdryxError> {
        let identities = self.runtime.block_on(self.client.list_identities())?;
        Ok(identities.iter().map(IdentityRecord::from).collect())
    }

    /// `GET /api/alerts` - every detector alert in the loaded snapshot,
    /// server-sorted severity-desc then time-asc. Feeds both the Alerts
    /// stream and the Attestation surface (`attestation_missing` /
    /// `bom_incomplete` rows - see the module doc).
    pub fn list_alerts(&self) -> Result<Vec<AlertRecord>, IdryxError> {
        let alerts = self.runtime.block_on(self.client.list_alerts())?;
        Ok(alerts.iter().map(AlertRecord::from).collect())
    }

    /// `GET /api/remediations` - every right-size/rotation suggestion idryx
    /// generated at load time.
    pub fn list_remediations(&self) -> Result<Vec<RemediationRecord>, IdryxError> {
        let recommendations = self.runtime.block_on(self.client.list_remediations())?;
        Ok(recommendations
            .iter()
            .map(RemediationRecord::from)
            .collect())
    }

    /// Recompute the 21 detectors over the CURRENT stack bus files (the
    /// **Rescan** button - see the module doc's "a subprocess call, not an
    /// HTTP mutation"). Always asks idryx for `--min-severity low` (the
    /// lowest threshold idryx accepts) regardless of what the panel's own
    /// severity filter currently shows, so a later filter change never needs
    /// a second Rescan just to reveal rows idryx already computed but this
    /// call would otherwise have discarded server-side.
    pub fn rescan(&self) -> Result<Vec<AlertRecord>, IdryxError> {
        let inputs = env::resolve_rescan_inputs()
            .map_err(|reason| IdryxError::RescanUnavailable { reason })?;
        let loads: Vec<(&str, &str)> = inputs
            .loads
            .iter()
            .map(|(source, path)| (source.as_str(), path.as_str()))
            .collect();
        let alerts = IdryxClient::rescan(&inputs.idryx_bin, &loads, "low")?;
        Ok(alerts.iter().map(AlertRecord::from).collect())
    }

    /// A cheap, synchronous pre-check for whether [`Self::rescan`] currently
    /// has what it needs - the exact same resolution
    /// [`env::resolve_rescan_inputs`] performs, just without spawning
    /// `idryx detect` afterward. Lets the SwiftUI panel disable the Rescan
    /// button with an honest, specific note before the operator ever clicks
    /// it (PHASE3.md W2: "a Rescan button... disabled with an honest note
    /// when the binary is unavailable"), rather than only discovering
    /// unavailability after a failed attempt. `None` means Rescan is
    /// currently expected to work; `Some(reason)` names exactly what is
    /// missing - the same text [`Self::rescan`] itself would fail with,
    /// wrapped in [`IdryxError::RescanUnavailable`].
    pub fn rescan_unavailable_reason(&self) -> Option<String> {
        env::resolve_rescan_inputs().err()
    }
}

// ---- private helpers (not exported over FFI) -------------------------------

impl IdryxHandle {
    /// Shared constructor body: build the unauthenticated client. Never
    /// touches the network (`IdryxClient::new` is a plain local constructor -
    /// builds a `reqwest::Client`, matching `WardryxClient::new`'s own doc
    /// comment); every fallible step folds into an [`IdryxError`], never a
    /// panic.
    fn build(resolved: ResolvedEnv) -> Result<Self, IdryxError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| IdryxError::ConnectFailed {
                reason: format!("could not start async runtime: {e}"),
            })?;

        let client = IdryxClient::new(resolved.idryx_url.clone()).map_err(|e| {
            IdryxError::ConnectFailed {
                reason: e.to_string(),
            }
        })?;

        Ok(Self {
            runtime,
            client,
            source: resolved.source,
            idryx_url: resolved.idryx_url,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    /// Rust-side stand-in proving `IdryxHandle` never panics when discovery
    /// finds nothing - the far more common case in CI than a live Idryx
    /// being available at all. Mirrors
    /// `wardryx::tests::discover_without_an_environment_is_a_clean_error_not_a_panic`.
    #[test]
    fn discover_without_an_environment_is_a_clean_error_not_a_panic() {
        // Does not touch `~/.taipan` or env vars; only proves the `Result`
        // shape, regardless of whether this box happens to have a real
        // `taipan up` environment or `IDRYX_URL` set (either a
        // `NoEnvironment`/`ConnectFailed` error or a genuine `Ok` are all
        // valid, non-panicking outcomes).
        match IdryxHandle::discover() {
            Ok(_) | Err(IdryxError::NoEnvironment | IdryxError::ConnectFailed { .. }) => {}
            Err(other) => panic!("unexpected error shape from discover(): {other:?}"),
        }
    }

    /// `IdryxHandle::connect` never touches the network at construction time
    /// (see the module doc): must succeed even against a port nothing is
    /// listening on. Mirrors
    /// `wardryx::tests::connect_never_touches_the_network_even_against_an_unreachable_url`.
    #[test]
    fn connect_never_touches_the_network_even_against_an_unreachable_url() {
        let handle = IdryxHandle::connect("http://127.0.0.1:1".to_string())
            .expect("connect() must succeed locally regardless of reachability");
        assert_eq!(handle.idryx_url(), "http://127.0.0.1:1");
        assert!(matches!(handle.source(), IdryxEnvSource::EnvFallback));
    }

    /// `rescan()` must never panic, regardless of what (if anything) this
    /// box's real `~/.taipan` happens to contain - this test deliberately
    /// does not control that state (see `env::resolve_rescan_inputs`'s own
    /// doc: it is independent of how the handle connected), so either a
    /// clean `RescanUnavailable`, a real recompute, or any other `IdryxError`
    /// (an unreachable dummy URL makes a *successful* recompute reach a
    /// network error) are all valid outcomes here - only a panic is not.
    #[test]
    fn rescan_never_panics_regardless_of_environment() {
        let handle = IdryxHandle::connect("http://127.0.0.1:1".to_string())
            .expect("connect() must succeed locally");
        match handle.rescan() {
            Ok(_) | Err(_) => {} // any Result is fine; a panic is not
        }
    }

    // ==========================================================================
    // live e2e: a real `idryx serve` (REST snapshot, through the handle) plus
    // a real `idryx detect --format json` (the exact call `rescan()` makes
    // internally), both against schema-conforming demo bus fixtures.
    // ==========================================================================
    // Same gated, hermetic shape as `wardryx::tests`' own live_e2e test:
    // skip-gracefully (an `eprintln!`, then an early return) whenever the
    // idryx binary cannot be obtained, never a hard failure over a missing
    // sibling checkout. Two sources are tried, per this crate's own task
    // brief: a fresh `go build` from `~/Development/idryx` (source of truth,
    // preferred so this test always exercises the CURRENT idryx source, not
    // a possibly-stale artifact) falling back to the `taipan up`-installed
    // `~/.taipan/bin/idryx` (a real binary, just not necessarily freshly
    // built from HEAD). `env::resolve_rescan_inputs` itself is intentionally
    // NOT exercised here (it reads this developer's real `~/.taipan`, which
    // this test must not mutate or depend on - see `wardryx::env`'s own
    // "never mutate real process environment" rationale, the same one that
    // applies to real `$HOME` state); this test instead calls
    // `IdryxClient::rescan` directly with fixture-controlled paths, the exact
    // function `IdryxHandle::rescan()` calls after its own env resolution
    // succeeds - so this still proves the full connector integration
    // end-to-end, just supplying the inputs `resolve_rescan_inputs` would
    // otherwise have located.

    /// Kills and reaps the `idryx serve` child on drop (including on a
    /// mid-test panic), and removes the scratch binary this test built, IF
    /// it built one (never removes a borrowed `~/.taipan/bin/idryx`).
    struct ChildGuard {
        child: Child,
        scratch_bin: Option<PathBuf>,
    }

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            if let Some(bin) = &self.scratch_bin {
                let _ = std::fs::remove_file(bin);
            }
        }
    }

    fn free_port() -> Option<u16> {
        std::net::TcpListener::bind("127.0.0.1:0")
            .ok()
            .and_then(|l| l.local_addr().ok())
            .map(|a| a.port())
    }

    fn idryx_repo() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok()?;
        let dir = PathBuf::from(home).join("Development/idryx");
        dir.join("go.mod").is_file().then_some(dir)
    }

    fn taipan_installed_binary() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok()?;
        let bin = PathBuf::from(home).join(".taipan/bin/idryx");
        bin.is_file().then_some(bin)
    }

    /// Prefer a fresh build from source (matches HEAD, mirrors
    /// `wardryx::tests::try_start_wardryx`'s own `go build` step); fall back
    /// to the `taipan up`-installed binary; skip-gracefully (an `eprintln!`)
    /// when neither is available.
    fn resolve_test_idryx_binary() -> Option<(PathBuf, Option<PathBuf>)> {
        if let Some(repo) = idryx_repo() {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let scratch = std::env::temp_dir().join(format!(
                "genaryx-ffi-idryx-test-bin-{}-{nanos}",
                std::process::id()
            ));
            match Command::new("go")
                .arg("build")
                .arg("-o")
                .arg(&scratch)
                .arg("./cmd/idryx")
                .current_dir(&repo)
                .status()
            {
                Ok(status) if status.success() && scratch.is_file() => {
                    let scratch_bin = scratch.clone();
                    return Some((scratch, Some(scratch_bin)));
                }
                Ok(status) => {
                    eprintln!(
                        "genaryx-ffi idryx live_e2e: `go build` failed ({status}); trying ~/.taipan/bin/idryx"
                    );
                }
                Err(e) => {
                    eprintln!(
                        "genaryx-ffi idryx live_e2e: could not run `go`: {e}; trying ~/.taipan/bin/idryx"
                    );
                }
            }
        }
        if let Some(bin) = taipan_installed_binary() {
            return Some((bin, None));
        }
        eprintln!(
            "genaryx-ffi idryx live_e2e: SKIPPING: neither ~/Development/idryx (go.mod) nor \
             ~/.taipan/bin/idryx was found"
        );
        None
    }

    fn spawn_idryx_serve(bin: &Path, addr: &str, loads: &[(&str, String)]) -> Option<Child> {
        let mut cmd = Command::new(bin);
        cmd.arg("serve").arg("--addr").arg(addr);
        for (source, path) in loads {
            cmd.arg("--load").arg(format!("{source}:{path}"));
        }
        cmd.stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()
    }

    /// Wait (bounded) for `idryx serve` to start accepting TCP connections,
    /// plus a short grace sleep so route setup has finished before real
    /// traffic starts - a plain TCP connect rather than an HTTP `/healthz`
    /// GET, same rationale `wardryx::tests`' own live test gives: this crate
    /// has no ad-hoc HTTP client of its own to spare just for a readiness
    /// poll.
    fn wait_for_port(child: &mut Child, addr: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(Some(_status)) = child.try_wait() {
                return false; // exited early
            }
            if std::net::TcpStream::connect(addr).is_ok() {
                std::thread::sleep(Duration::from_millis(300));
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    #[test]
    fn live_e2e_serve_snapshot_and_detect_rescan() {
        let Some((bin, scratch_bin)) = resolve_test_idryx_binary() else {
            return; // already explained why via eprintln! above
        };

        // Real, schema-conforming bus fixtures - the same generator
        // `crate::tests` (FleetHandle's own live test, `lib.rs`) already
        // trusts - so idryx's own `tokenfuse`/`wardryx`/`mockryx`/`verdryx`
        // loaders see exactly the shape the rest of this stack produces,
        // never hand-crafted JSON that might drift from the real schema.
        let events_dir = std::env::temp_dir().join(format!(
            "genaryx-ffi-idryx-test-events-{}",
            std::process::id()
        ));
        if let Err(e) = std::fs::create_dir_all(&events_dir) {
            eprintln!(
                "genaryx-ffi idryx live_e2e: SKIPPING: could not create a scratch events dir: {e}"
            );
            return;
        }
        if let Err(e) = genaryx_core::demo::generate(&events_dir) {
            eprintln!("genaryx-ffi idryx live_e2e: SKIPPING: demo::generate failed: {e}");
            let _ = std::fs::remove_dir_all(&events_dir);
            return;
        }

        // The same four stack-bus sources `env::RESCAN_SOURCES` names (kept
        // as its own private constant there - this is a deliberate small
        // duplication, not a shared import, matching this file's own
        // established convention for tiny test-only constants).
        const SOURCES: [&str; 4] = ["tokenfuse", "wardryx", "mockryx", "verdryx"];
        let loads: Vec<(&str, String)> = SOURCES
            .iter()
            .map(|source| {
                (
                    *source,
                    events_dir
                        .join(format!("{source}.ndjson"))
                        .to_string_lossy()
                        .into_owned(),
                )
            })
            .collect();

        let Some(port) = free_port() else {
            eprintln!("genaryx-ffi idryx live_e2e: SKIPPING: could not reserve a port");
            let _ = std::fs::remove_dir_all(&events_dir);
            return;
        };
        let addr = format!("127.0.0.1:{port}");
        let Some(mut child) = spawn_idryx_serve(&bin, &addr, &loads) else {
            eprintln!(
                "genaryx-ffi idryx live_e2e: SKIPPING: failed to spawn {}",
                bin.display()
            );
            let _ = std::fs::remove_dir_all(&events_dir);
            return;
        };
        if !wait_for_port(&mut child, &addr, Duration::from_secs(30)) {
            eprintln!("genaryx-ffi idryx live_e2e: SKIPPING: idryx serve never opened its port");
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&events_dir);
            return;
        }
        let _guard = ChildGuard { child, scratch_bin };

        // ---- REST snapshot, through the handle's own exported reads ----
        let handle =
            IdryxHandle::connect(format!("http://{addr}")).expect("connect() must succeed locally");
        assert_eq!(handle.idryx_url(), format!("http://{addr}"));

        let identities = handle
            .list_identities()
            .expect("list_identities against a live idryx");
        assert!(
            !identities.is_empty(),
            "demo fixtures must yield at least one identity"
        );
        assert!(
            identities
                .iter()
                .any(|i| !i.id.is_empty() && !i.identity_type.is_empty()),
            "identities must carry a real id and type"
        );

        let alerts = handle
            .list_alerts()
            .expect("list_alerts against a live idryx");
        // Remediations may legitimately be empty (idryx only emits one when
        // it has a right-size/rotation suggestion) - just prove the call
        // itself succeeds against a live server.
        let _remediations = handle
            .list_remediations()
            .expect("list_remediations against a live idryx");

        // ---- Rescan: the exact connector call `IdryxHandle::rescan()`
        // makes internally, supplied with this test's own fixture paths
        // rather than `env::resolve_rescan_inputs()` - see this block's own
        // module-level doc comment for why. ----
        let load_args: Vec<(&str, &str)> = loads.iter().map(|(s, p)| (*s, p.as_str())).collect();
        let rescanned = IdryxClient::rescan(&bin, &load_args, "low")
            .expect("idryx detect --format json over the fixtures");
        let rescanned_records: Vec<AlertRecord> = rescanned.iter().map(AlertRecord::from).collect();
        assert!(
            !rescanned_records.is_empty(),
            "the demo campaign is known to trip at least one of the 21 detectors"
        );
        assert!(
            rescanned_records
                .iter()
                .all(|a| !a.detector.is_empty() && !a.severity.is_empty()),
            "every rescanned alert must carry a detector id and a severity"
        );

        eprintln!(
            "genaryx-ffi idryx live_e2e: PASSED - {} identities, {} alerts (serve), {} alerts (rescan)",
            identities.len(),
            alerts.len(),
            rescanned_records.len()
        );

        let _ = std::fs::remove_dir_all(&events_dir);
        // `_guard` drops here: kills idryx serve, removes the scratch binary
        // if one was built.
    }
}
