//! `CryptoHandle`: the UniFFI Object wrapping `genaryx_connectors::QryxClient`
//! for the SwiftUI Crypto surface (docs/PHASE4.md W1, "Track B
//! `crates/ffi/src/crypto/`"), at parity with the Tauri shell's own Crypto
//! panel (the sibling Track A). Like [`crate::quality::QualityHandle`], this
//! needs no owned `tokio::runtime::Runtime`: [`QryxClient`] is a synchronous
//! subprocess runner (`std::process::Command::output`, blocking), the same
//! shape as `IdryxClient::rescan`.
//!
//! ## On-demand, not a live feed
//!
//! Unlike Quality (a cheap SQLite read the panel can poll on a timer) or
//! Identity (a REST snapshot), every read here shells out to `qryx`, which
//! walks a filesystem tree - genuinely expensive, and never something this
//! handle should run without the operator asking for it. So, unlike
//! [`crate::idryx::IdryxHandle`]'s `list_identities`/`list_alerts` (safe to
//! auto-refresh on a timer), every method here is meant to be called ONLY in
//! direct response to an operator action (the Scan button, the Verify
//! button); the Swift `CryptoModel` never wraps these in a periodic
//! `.task` refresh loop the way `IdentityView`/`PolicyView` do. The Swift
//! panel labels all of it "as of last scan" (docs/PHASE4.md W1), never
//! implying a live feed - mirrors how the Identity panel labels Idryx's
//! load-once snapshot "as of load" for the same honesty reason.
//!
//! ## `QryxClient` is trivially `Send + Sync`: held directly, unlike Quality
//!
//! [`QryxClient`] wraps only an owned `PathBuf` (no connection, no socket),
//! so - unlike [`crate::quality::QualityHandle`]'s deliberate
//! open-fresh-per-call design (forced by `VerdryxClient`'s non-`Sync`
//! `rusqlite::Connection`) - this handle holds ONE [`QryxClient`] for its
//! whole lifetime, exactly like [`crate::idryx::IdryxHandle`] holds one
//! `IdryxClient`. There is no cache to go stale: every exported method still
//! runs a fresh `qryx` subprocess invocation on every call, this is purely
//! about not reconstructing a zero-cost wrapper struct repeatedly.
//!
//! ## Fail-closed, and what "absent" means here
//!
//! No panics, no `unwrap`/`expect`. [`CryptoHandle::discover`] fails closed
//! with [`CryptoError::NoEnvironment`] when [`env::discover`] finds no `qryx`
//! binary at all (docs/PHASE4.md W1: "An absent source (no... `qryx` binary)
//! must render as an HONEST first-class empty state, never a
//! fake-empty-success and never a panic"). A binary that WAS named
//! (an operator-supplied path via [`CryptoHandle::connect`]) but cannot be
//! run surfaces distinctly, as [`CryptoError::Spawn`], on the first scan
//! attempt - never silently collapsed into the same "no crypto plane" empty
//! state (mirrors [`crate::quality::QualityHandle`]'s own
//! `NoEnvironment`-vs-`Open` distinction).
//!
//! ## `migrated_count` is always 0 - never shown as progress
//!
//! [`dto::NcscPriorityRecord::migrated_count`] is carried through verbatim
//! from qryx (always `0` - see that field's own doc). This handle does not
//! attempt to derive, hide, or "fix" it; the honesty obligation is the Swift
//! panel's (docs/PHASE4.md W1 guard: "`migrated_count` is ALWAYS 0 - label it
//! honestly, never as real progress").

pub mod dto;
pub mod env;

pub use dto::{
    CountEntry, CryptoError, EvidenceReportRecord, EvidenceSignatureRecord, EvidenceSummaryRecord,
    NcscDiscoveryRecord, NcscFindingRecord, NcscFullMigrationRecord, NcscPriorityRecord,
    NcscReportRecord, VerifyOutcomeRecord,
};
pub use env::CryptoEnvSource;

use dto::EvidenceReportRecord as EvidenceRecord;
use env::ResolvedEnv;
use genaryx_connectors::QryxClient;
use std::path::{Path, PathBuf};

/// The Crypto UniFFI Object: a resolved `qryx` binary plus a default scan
/// target. See the module doc for why [`QryxClient`] is held directly rather
/// than reopened per call.
#[derive(uniffi::Object)]
pub struct CryptoHandle {
    source: CryptoEnvSource,
    qryx_bin: PathBuf,
    default_scan_target: PathBuf,
    client: QryxClient,
}

#[uniffi::export]
impl CryptoHandle {
    /// Discover the `qryx` binary: the well-known `~/.taipan/bin/qryx`. Fails
    /// closed with [`CryptoError::NoEnvironment`] when it is not there - a
    /// normal, renderable "no crypto plane" outcome, not a bug (see the
    /// module doc). Never runs qryx itself: exactly like
    /// `IdryxHandle::discover`/`connect` never touch the network at
    /// construction, this never spawns a subprocess until a scan/verify
    /// method is actually called.
    #[uniffi::constructor]
    pub fn discover() -> Result<Self, CryptoError> {
        let resolved = env::discover().ok_or(CryptoError::NoEnvironment)?;
        Ok(Self::build(resolved))
    }

    /// Point directly at `qryx_bin`, skipping discovery - for a qryx binary
    /// the operator names explicitly. Always reports
    /// [`CryptoEnvSource::Explicit`], mirroring `IdryxHandle::connect`'s own
    /// dual use of `EnvFallback`. The default scan target is still resolved
    /// normally ([`env::default_scan_target`]) - it is independent of how the
    /// binary itself was located.
    #[uniffi::constructor]
    pub fn connect(qryx_bin: String) -> Self {
        Self::build(ResolvedEnv {
            source: CryptoEnvSource::Explicit,
            qryx_bin: PathBuf::from(qryx_bin),
            default_scan_target: env::default_scan_target(),
        })
    }

    /// Where this handle resolved its `qryx` binary from.
    pub fn source(&self) -> CryptoEnvSource {
        self.source.clone()
    }

    /// The resolved `qryx` binary path this handle runs.
    pub fn qryx_bin(&self) -> String {
        self.qryx_bin.display().to_string()
    }

    /// A pre-filled scan target the operator can see and override before
    /// running a scan (docs/PHASE4.md W1: "operator can see/set it") - never
    /// enforced, just a starting point.
    pub fn default_scan_target(&self) -> String {
        self.default_scan_target.display().to_string()
    }

    // ---- on-demand scans (never auto-refreshed - see the module doc) ------

    /// `qryx scan --format ncsc <target>` - the PQC migration-timeline hero
    /// (the 2028/2031/2035 NCSC milestones).
    pub fn scan_ncsc(&self, target: String) -> Result<NcscReportRecord, CryptoError> {
        let report = self.client.scan_ncsc(Path::new(&target))?;
        Ok(NcscReportRecord::from(&report))
    }

    /// `qryx scan --format cbom <target>` - the CycloneDX 1.6 CBOM, as a JSON
    /// string (see `dto`'s module doc for why this crosses FFI untyped).
    pub fn scan_cbom(&self, target: String) -> Result<String, CryptoError> {
        let value = self.client.scan_cbom(Path::new(&target))?;
        serde_json::to_string(&value).map_err(|e| CryptoError::Json {
            reason: e.to_string(),
        })
    }

    /// `qryx scan --format evidence [--sign-key <pem>] <target>` - the build
    /// bundle the Evidence section shows (W1: unsigned is fine, so
    /// `sign_key` is normally `None`; carried through for W3's Evidence
    /// Center to reuse this same call signed).
    pub fn scan_evidence(
        &self,
        target: String,
        sign_key: Option<String>,
    ) -> Result<EvidenceReportRecord, CryptoError> {
        let sign_key_ref = sign_key.as_deref().map(Path::new);
        let report = self
            .client
            .scan_evidence(Path::new(&target), sign_key_ref)?;
        EvidenceRecord::from_conn(&report)
    }

    /// `qryx agents --format evidence <target>` - the same [`EvidenceReportRecord`]
    /// shape, scoped to the agent-governance stack's own trust surface
    /// (Agent Passport attestation crypto + agent-event hash-chain
    /// integrity - `crates/connectors/src/qryx.rs`'s own doc). The Evidence
    /// section's scope toggle (repository vs. agent stack) calls this
    /// instead of [`Self::scan_evidence`], never both at once.
    pub fn agents_evidence(&self, target: String) -> Result<EvidenceReportRecord, CryptoError> {
        let report = self.client.agents_evidence(Path::new(&target))?;
        EvidenceRecord::from_conn(&report)
    }

    /// `qryx verify-evidence <file>` - recompute the digest AND check the
    /// signature. `verified: false` is a real "not verified" answer, not an
    /// error (see `genaryx_connectors::VerifyOutcome`'s own doc); only a
    /// spawn failure surfaces as [`CryptoError`].
    pub fn verify_evidence(&self, file: String) -> Result<VerifyOutcomeRecord, CryptoError> {
        let outcome = self.client.verify_evidence(Path::new(&file))?;
        Ok(VerifyOutcomeRecord::from(&outcome))
    }
}

// ---- private helpers (not exported over FFI) -------------------------------

impl CryptoHandle {
    fn build(resolved: ResolvedEnv) -> Self {
        Self {
            source: resolved.source,
            qryx_bin: resolved.qryx_bin.clone(),
            default_scan_target: resolved.default_scan_target,
            client: QryxClient::new(resolved.qryx_bin),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rust-side stand-in proving `CryptoHandle` never panics when discovery
    /// finds nothing - the common case in CI (no `qryx` binary on the box).
    /// Mirrors `quality::tests::discover_without_an_environment_is_a_clean_error_not_a_panic`.
    #[test]
    fn discover_without_an_environment_is_a_clean_error_not_a_panic() {
        match CryptoHandle::discover() {
            Ok(_) | Err(CryptoError::NoEnvironment) => {}
            Err(other) => panic!("unexpected error shape from discover(): {other:?}"),
        }
    }

    /// `connect()` never touches the filesystem/subprocess at construction
    /// time - must succeed even against a path nothing has ever written.
    /// Mirrors
    /// `idryx::tests::connect_never_touches_the_network_even_against_an_unreachable_url`.
    #[test]
    fn connect_never_touches_the_filesystem_at_construction_time() {
        let handle = CryptoHandle::connect("/definitely/not/a/real/qryx".to_string());
        assert_eq!(handle.qryx_bin(), "/definitely/not/a/real/qryx");
        assert!(matches!(handle.source(), CryptoEnvSource::Explicit));
        assert!(!handle.default_scan_target().is_empty());
    }

    /// A scan against a binary that cannot spawn must surface an honest
    /// [`CryptoError::Spawn`], never a panic and never a fake-empty result.
    #[test]
    fn scan_against_a_missing_binary_is_an_honest_spawn_error_not_a_panic() {
        let handle = CryptoHandle::connect("/definitely/not/a/real/qryx".to_string());
        match handle.scan_ncsc("/tmp".to_string()) {
            Err(CryptoError::Spawn { bin, .. }) => {
                assert_eq!(bin, "/definitely/not/a/real/qryx");
            }
            other => panic!("expected CryptoError::Spawn, got {other:?}"),
        }
    }

    /// Same fail-closed contract, exercised over every remaining read method
    /// - none of them should ever panic against an unusable binary.
    #[test]
    fn every_read_method_fails_closed_against_a_missing_binary() {
        let handle = CryptoHandle::connect("/definitely/not/a/real/qryx".to_string());
        assert!(matches!(
            handle.scan_cbom("/tmp".to_string()),
            Err(CryptoError::Spawn { .. })
        ));
        assert!(matches!(
            handle.scan_evidence("/tmp".to_string(), None),
            Err(CryptoError::Spawn { .. })
        ));
        assert!(matches!(
            handle.agents_evidence("/tmp".to_string()),
            Err(CryptoError::Spawn { .. })
        ));
        assert!(matches!(
            handle.verify_evidence("/tmp/report.json".to_string()),
            Err(CryptoError::Spawn { .. })
        ));
    }
}
