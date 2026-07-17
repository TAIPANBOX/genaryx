//! `DrillsHandle`: the UniFFI Object wrapping `genaryx_connectors::MockryxClient`
//! for the SwiftUI Drills surface (docs/PHASE4.md W2, "Track B
//! `crates/ffi/src/drills/`"), at parity with the Tauri shell's own Drills
//! panel (the sibling Track A). Like [`crate::crypto::CryptoHandle`], this
//! needs no owned `tokio::runtime::Runtime`: [`MockryxClient`] is a
//! synchronous subprocess runner (`std::process::Command::output`, blocking),
//! the same shape as `QryxClient`.
//!
//! ## On-demand, never auto-run
//!
//! Every read here shells out to `mockryx run`, which sends real adversarial
//! HTTP traffic at a live TokenFuse gateway - genuinely consequential, never
//! something this handle should run without the operator asking for it
//! (docs/PHASE4.md W2: "'Run drills' action... on demand... never
//! auto-run"). So, exactly like [`crate::crypto::CryptoHandle`], every method
//! here is meant to be called ONLY in direct response to an operator action;
//! the Swift `DrillsModel` never wraps [`DrillsHandle::run`] in a periodic
//! `.task` refresh loop.
//!
//! ## `MockryxClient` is trivially `Send + Sync`: held directly
//!
//! [`MockryxClient`] wraps only an owned `PathBuf` (no connection, no
//! socket), so - exactly like [`crate::crypto::CryptoHandle`] holds one
//! `QryxClient` - this handle holds ONE [`MockryxClient`] for its whole
//! lifetime. There is no cache to go stale: every exported method still runs
//! a fresh `mockryx` subprocess invocation on every call.
//!
//! ## `load_report`: the "last run" view survives an app restart
//!
//! [`DrillsHandle::run`] is never auto-run, so a freshly-launched console
//! would otherwise show an empty Drills panel until the operator presses Run
//! at least once THIS session - a worse "as of last run" story than every
//! sibling on-demand panel gets for free. [`env::default_save_path`] names a
//! well-known `~/.taipan/mockryx-last-report.json`; the Swift model passes it
//! as `run`'s own `save_path` by default (so every real run leaves a durable
//! trail) and calls [`DrillsHandle::load_report`] against that same path once
//! at connect time to pre-populate the "last run" view - loading a past
//! artifact is reading, not running a new drill, so this does not violate
//! "never auto-run".
//!
//! ## Fail-closed, and what "gap" means here
//!
//! No panics, no `unwrap`/`expect`. [`DrillsHandle::discover`] fails closed
//! with [`DrillsError::NoEnvironment`] when [`env::discover_bin`] finds no
//! `mockryx` binary at all (docs/PHASE4.md W2: "honest empty state when no
//! mockryx binary"). A gap (mockryx exit `1`, real findings) is NEVER an
//! error here - see [`dto::DrillsError`]'s own doc for why that distinction
//! is the one bug class this whole wave's review discipline calls out by
//! name.

pub mod dto;
pub mod env;

pub use dto::{
    DrillFindingRecord, DrillMetricsRecord, DrillReportRecord, DrillResultRecord, DrillsError,
    HeaderEntry,
};
pub use env::DrillsEnvSource;

use env::ResolvedBin;
use genaryx_connectors::MockryxClient;
use std::path::{Path, PathBuf};

/// The Drills UniFFI Object: a resolved `mockryx` binary plus pre-filled
/// defaults (scenario dir, gateway, api key, save path) the operator can see
/// and override before running. See the module doc for why [`MockryxClient`]
/// is held directly rather than reopened per call.
#[derive(uniffi::Object)]
pub struct DrillsHandle {
    source: DrillsEnvSource,
    mockryx_bin: PathBuf,
    default_scenario_dir: PathBuf,
    default_gateway: Option<String>,
    default_api_key: Option<String>,
    default_save_path: PathBuf,
    client: MockryxClient,
}

#[uniffi::export]
impl DrillsHandle {
    /// Discover the `mockryx` binary ([`env::discover_bin`]: `MOCKRYX_BIN`,
    /// then a sibling checkout's `bin/mockryx`). Fails closed with
    /// [`DrillsError::NoEnvironment`] when neither resolves - a normal,
    /// renderable "no drills plane" outcome (see the module doc), not a bug.
    /// Never runs mockryx itself: exactly like `CryptoHandle::discover`, this
    /// never spawns a subprocess until [`Self::run`] is actually called.
    #[uniffi::constructor]
    pub fn discover() -> Result<Self, DrillsError> {
        let resolved = env::discover_bin().ok_or(DrillsError::NoEnvironment)?;
        Ok(Self::build(resolved))
    }

    /// Point directly at `mockryx_bin`, skipping discovery - for a mockryx
    /// binary the operator names explicitly. Always reports
    /// [`DrillsEnvSource::Explicit`], mirroring `CryptoHandle::connect`'s own
    /// escape-hatch role. The other defaults (scenario dir, gateway, api key,
    /// save path) are still resolved normally - independent of how the
    /// binary itself was located.
    #[uniffi::constructor]
    pub fn connect(mockryx_bin: String) -> Self {
        Self::build(ResolvedBin {
            source: DrillsEnvSource::Explicit,
            bin: PathBuf::from(mockryx_bin),
        })
    }

    /// Where this handle resolved its `mockryx` binary from.
    pub fn source(&self) -> DrillsEnvSource {
        self.source.clone()
    }

    /// The resolved `mockryx` binary path this handle runs.
    pub fn mockryx_bin(&self) -> String {
        self.mockryx_bin.display().to_string()
    }

    /// A pre-filled scenario directory the operator can see/override before
    /// running (docs/PHASE4.md W2 phrasing, mirroring
    /// `CryptoHandle::default_scan_target`'s own "never enforced" contract).
    pub fn default_scenario_dir(&self) -> String {
        self.default_scenario_dir.display().to_string()
    }

    /// A pre-filled TokenFuse gateway URL, or `None` when nothing resolved
    /// (a common, honest outcome - see `env`'s own module doc) - the Swift
    /// panel then shows a blank, operator-fillable field rather than a
    /// whole-panel empty state for this alone.
    pub fn default_gateway(&self) -> Option<String> {
        self.default_gateway.clone()
    }

    /// A pre-filled gateway api key, or `None`.
    pub fn default_api_key(&self) -> Option<String> {
        self.default_api_key.clone()
    }

    /// The well-known path [`Self::run`]'s `save_path` and
    /// [`Self::load_report`] both default to - see the module doc's
    /// "'last run' view survives an app restart".
    pub fn default_save_path(&self) -> String {
        self.default_save_path.display().to_string()
    }

    // ---- on-demand run (never auto-refreshed - see the module doc) --------

    /// `mockryx run --gateway <gateway> --format json [--api-key K]
    /// [--fail-on-skip] [--save save_path] <scenario_dir>` - the drill's
    /// verdict. Exit `0`/`1` (guardrails held / a real gap found) both yield
    /// a normal [`DrillReportRecord`] here (see [`dto::DrillsError`]'s own
    /// doc); only a spawn failure or exit `2` (bad usage - e.g. an empty
    /// `gateway`) surfaces as [`DrillsError`].
    pub fn run(
        &self,
        scenario_dir: String,
        gateway: String,
        api_key: Option<String>,
        fail_on_skip: bool,
        save_path: Option<String>,
    ) -> Result<DrillReportRecord, DrillsError> {
        let report = self.client.run(
            Path::new(&scenario_dir),
            &gateway,
            api_key.as_deref(),
            fail_on_skip,
            save_path.as_deref().map(Path::new),
        )?;
        Ok(DrillReportRecord::from(&report))
    }

    /// Re-load a previously `--save`d report from disk, without re-running
    /// the drill - the Drills panel's "last run" view across app restarts
    /// (see the module doc), and a plain manual re-load if the operator ever
    /// wants to revisit a specific saved file. A missing/unreadable file is
    /// [`DrillsError::Read`] - a normal "nothing saved yet" outcome the Swift
    /// model treats as an empty state, never a banner error, on the
    /// connect-time best-effort call.
    pub fn load_report(&self, path: String) -> Result<DrillReportRecord, DrillsError> {
        let report = MockryxClient::load_report(Path::new(&path))?;
        Ok(DrillReportRecord::from(&report))
    }
}

// ---- private helpers (not exported over FFI) -------------------------------

impl DrillsHandle {
    fn build(resolved: ResolvedBin) -> Self {
        Self {
            source: resolved.source,
            mockryx_bin: resolved.bin.clone(),
            default_scenario_dir: env::default_scenario_dir(),
            default_gateway: env::default_gateway(),
            default_api_key: env::default_api_key(),
            default_save_path: env::default_save_path(),
            client: MockryxClient::new(resolved.bin),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rust-side stand-in proving `DrillsHandle` never panics when discovery
    /// finds nothing - the common case in CI (no `mockryx` binary on the
    /// box). Mirrors
    /// `crypto::tests::discover_without_an_environment_is_a_clean_error_not_a_panic`.
    #[test]
    fn discover_without_an_environment_is_a_clean_error_not_a_panic() {
        match DrillsHandle::discover() {
            Ok(_) | Err(DrillsError::NoEnvironment) => {}
            Err(other) => panic!("unexpected error shape from discover(): {other:?}"),
        }
    }

    /// `connect()` never touches the filesystem/subprocess at construction
    /// time - must succeed even against a path nothing has ever written.
    /// Mirrors `crypto::tests::connect_never_touches_the_filesystem_at_construction_time`.
    #[test]
    fn connect_never_touches_the_filesystem_at_construction_time() {
        let handle = DrillsHandle::connect("/definitely/not/a/real/mockryx".to_string());
        assert_eq!(handle.mockryx_bin(), "/definitely/not/a/real/mockryx");
        assert!(matches!(handle.source(), DrillsEnvSource::Explicit));
        assert!(!handle.default_scenario_dir().is_empty());
        assert!(!handle.default_save_path().is_empty());
    }

    /// A run against a binary that cannot spawn must surface an honest
    /// [`DrillsError::Spawn`], never a panic and never a fake-empty result.
    #[test]
    fn run_against_a_missing_binary_is_an_honest_spawn_error_not_a_panic() {
        let handle = DrillsHandle::connect("/definitely/not/a/real/mockryx".to_string());
        match handle.run(
            "/tmp/scenarios".to_string(),
            "http://127.0.0.1:4100".to_string(),
            None,
            false,
            None,
        ) {
            Err(DrillsError::Spawn { bin, .. }) => {
                assert_eq!(bin, "/definitely/not/a/real/mockryx");
            }
            other => panic!("expected DrillsError::Spawn, got {other:?}"),
        }
    }

    /// `load_report` against a file that was never written must be an honest
    /// [`DrillsError::Read`], never a panic.
    #[test]
    fn load_report_missing_file_is_fail_closed() {
        let handle = DrillsHandle::connect("/definitely/not/a/real/mockryx".to_string());
        match handle.load_report("/definitely/not/a/real/report.json".to_string()) {
            Err(DrillsError::Read { .. }) => {}
            other => panic!("expected DrillsError::Read, got {other:?}"),
        }
    }

    // ==========================================================================
    // live e2e: a real `mockryx` (freshly `go build`'d from a sibling
    // `~/Development/mockryx` checkout) run for real against an UNREACHABLE
    // gateway (127.0.0.1:1, nothing listening) plus the checkout's own
    // bundled scenarios. No TokenFuse/Wardryx stack needed: every scenario's
    // HTTP calls simply fail to connect, which mockryx reports as real
    // Findings (exit 1, a normal gap) rather than a spawn/usage error -
    // proven against the real CLI before writing this test (`mockryx run
    // --gateway http://127.0.0.1:1 ...` => exit 1, five `failed` results,
    // each finding's `detail` a real "connection refused" transport error).
    // This exercises the WHOLE real path: spawn, args, exit-code handling,
    // JSON parsing, `has_gaps` computation, and every DTO conversion -
    // skip-gracefully (an `eprintln!`, then an early return) when `go` or the
    // sibling checkout is unavailable, mirroring `idryx::tests`' own
    // live_e2e shape.
    #[test]
    fn live_e2e_run_and_load_report_over_a_real_mockryx_against_an_unreachable_gateway() {
        let Some(repo) = mockryx_repo() else {
            eprintln!(
                "genaryx-ffi drills live_e2e: SKIPPING: ~/Development/mockryx (go.mod) not found"
            );
            return;
        };
        let Some(bin) = build_mockryx(&repo) else {
            return; // already explained why via eprintln! above
        };
        let scenarios = repo.join("scenarios");
        if !scenarios.is_dir() {
            eprintln!(
                "genaryx-ffi drills live_e2e: SKIPPING: no scenarios/ dir in the mockryx checkout"
            );
            return;
        }

        let save_path = std::env::temp_dir().join(format!(
            "genaryx-ffi-drills-live-e2e-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&save_path);

        let handle = DrillsHandle::connect(bin.to_string_lossy().into_owned());

        let report = handle
            .run(
                scenarios.to_string_lossy().into_owned(),
                "http://127.0.0.1:1".to_string(),
                None,
                false,
                Some(save_path.to_string_lossy().into_owned()),
            )
            .expect("run against a real mockryx binary");

        assert_eq!(
            report.results.len(),
            5,
            "the 5 bundled scenarios: {report:?}"
        );
        assert!(
            report.has_gaps,
            "every scenario must fail against an unreachable gateway: {report:?}"
        );
        assert!(
            report
                .results
                .iter()
                .all(|r| r.status == "failed" && !r.findings.is_empty()),
            "every result must be a real failed finding, not silently held: {report:?}"
        );
        assert!(
            report.results.iter().flat_map(|r| &r.findings).all(|f| f
                .detail
                .to_lowercase()
                .contains("refused")
                || f.detail.to_lowercase().contains("connect")),
            "each finding's detail must be the real transport error: {report:?}"
        );

        // A real usage error (exit 2: no gateway at all) must be a real
        // DrillsError::Cli, never folded into a fake report.
        match handle.run(
            scenarios.to_string_lossy().into_owned(),
            String::new(),
            None,
            false,
            None,
        ) {
            Err(DrillsError::Cli { code, .. }) => assert_eq!(code, 2),
            other => panic!(
                "expected DrillsError::Cli{{code: 2, ..}} for an empty gateway, got {other:?}"
            ),
        }

        // load_report: the file `run` itself just saved, re-read without
        // re-running - must match what `run` returned.
        let reloaded = handle
            .load_report(save_path.to_string_lossy().into_owned())
            .expect("load_report against the file run() just saved");
        assert_eq!(reloaded.run_id, report.run_id);
        assert_eq!(reloaded.results.len(), report.results.len());

        let _ = std::fs::remove_file(&save_path);
        eprintln!(
            "genaryx-ffi drills live_e2e: PASSED - {} results, has_gaps={}",
            report.results.len(),
            report.has_gaps
        );
    }

    fn mockryx_repo() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok()?;
        let dir = PathBuf::from(home).join("Development/mockryx");
        dir.join("go.mod").is_file().then_some(dir)
    }

    /// `go build -o bin/mockryx ./cmd/mockryx`, exactly mockryx's own
    /// Makefile target and exactly where `env::checkout_bin_under` looks -
    /// mirrors `crates/connectors/tests/exit_gate_test.rs`'s own
    /// `build_mockryx` (that file is in the FROZEN `connectors` crate, so
    /// this is a deliberate small duplication rather than a cross-crate
    /// import, matching this crate's own established
    /// "independent-evolution-over-shared-abstraction" precedent).
    fn build_mockryx(repo: &Path) -> Option<PathBuf> {
        let bin = repo.join("bin").join("mockryx");
        let status = std::process::Command::new("go")
            .args(["build", "-o", "bin/mockryx", "./cmd/mockryx"])
            .current_dir(repo)
            .status();
        match status {
            Ok(status) if status.success() && bin.is_file() => Some(bin),
            Ok(status) => {
                eprintln!("genaryx-ffi drills live_e2e: SKIPPING: `go build` failed ({status})");
                None
            }
            Err(e) => {
                eprintln!("genaryx-ffi drills live_e2e: SKIPPING: could not run `go`: {e}");
                None
            }
        }
    }
}
