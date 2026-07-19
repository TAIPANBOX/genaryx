//! `WardryxHandle`: the UniFFI Object wrapping
//! `genaryx_connectors::WardryxClient` for the SwiftUI Policy surface
//! (docs/PHASE2.md wave 2, "Track B (SwiftUI) `crates/ffi/src/wardryx/`"),
//! at parity with the Tauri shell's own `src-tauri/src/policy/` (the
//! sibling Track A). Structurally this mirrors [`crate::cloud::CloudHandle`]
//! (same owned-runtime async-to-sync bridge, same fail-closed
//! `command::record` journal - see that module's own doc for the full
//! rationale of both), but SIMPLER in one respect: Wardryx's entire API
//! (bar `/healthz`) is gated purely by a static `Authorization: Bearer`
//! header (`genaryx_connectors::WardryxClient`'s own doc comment), so there
//! is no pairing/device/signer ceremony at all here. `WardryxHandle::build`
//! is therefore synchronous end to end - `WardryxClient::new` never touches
//! the network - unlike `CloudHandle::build`, which has to `block_on` a real
//! pair-and-attach handshake.
//!
//! ## No signature, but still a privileged mutation
//!
//! [`WardryxHandle::decide_approval`] journals a `console_command` exactly
//! like `CloudHandle`'s mutations do, but this handle never signs anything
//! (no `genaryx-signing` dependency at all): the call is authenticated by
//! the bearer token, and PHASE2.md's own contract puts the human-in-the-loop
//! gate client-side instead - the SwiftUI shell challenges
//! `LocalAuthentication` (Touch ID) BEFORE ever calling into this handle.
//! [`CommandRecord`]'s `sig_alg`/`sig_fpr` fields are still required, so
//! [`SIG_ALG`]/[`SIG_FPR`] stand in with an honest description of what
//! actually authorized the call (the bearer scheme; a local hardware
//! confirmation) rather than a fabricated signature - see their own doc
//! comments.
//!
//! ## No pairing means no server-confirmed org
//!
//! `CloudHandle` learns `org_domain` from the Cloud's own pairing response
//! (`paired.org`). Wardryx has no equivalent handshake, so this handle
//! derives an org-domain-shaped LABEL instead (never presented as
//! server-confirmed): the sanitized `taipan up` environment name when
//! [`WardryxEnvSource::Taipan`] resolved it, or the same `"genaryx.local"`
//! fallback `CloudHandle`'s own `sanitize_domain("")` would produce
//! otherwise. It exists only to build a conforming `agent_id`/`operator`
//! principal for the journal (07 §1's `[a-z0-9.-]+` charset), never to claim
//! authority about which org the operator belongs to.
//!
//! Fail-closed at the boundary (06 §0.5): nothing here panics across FFI.

pub mod dto;
pub mod env;

pub use dto::{ApprovalDecideOutcome, ApprovalRecord, ApprovalVerdict, PolicyRecord, WardryxError};
pub use env::WardryxEnvSource;

use dto::{describe_decision, status_of};
use env::ResolvedEnv;
use genaryx_connectors::{WardryxClient, WardryxError as ConnWardryxError};
use genaryx_core::CommandRecord;
use genaryx_core::command;
use genaryx_core::store::Store;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

/// What authenticated this call, for [`CommandRecord::sig_alg`] - see the
/// module doc's "No signature, but still a privileged mutation". Every
/// Wardryx route is gated by a static bearer token (07 §4.3), never an
/// ES256-signed payload, so this is honestly `"bearer"`, not a signing
/// algorithm name.
const SIG_ALG: &str = "bearer";

/// What authenticated this call, for [`CommandRecord::sig_fpr`] - see the
/// module doc's "No signature, but still a privileged mutation". There is no
/// key fingerprint to report (no signer exists here at all); this instead
/// honestly names the actual gate the SwiftUI shell places before every
/// `decide_approval` call: a local hardware confirmation
/// (`LocalAuthentication`/Touch ID), evaluated entirely client-side before
/// this handle is ever invoked.
const SIG_FPR: &str = "local-auth";

/// The Policy UniFFI Object: a bearer-authenticated [`WardryxClient`] plus
/// everything a decision needs to journal a `console_command`. See the
/// module docs for the async-to-sync bridge and the journal shape.
#[derive(uniffi::Object)]
pub struct WardryxHandle {
    runtime: tokio::runtime::Runtime,
    client: WardryxClient,
    source: WardryxEnvSource,
    wardryx_url: String,
    /// Sanitized, `agent_id`-safe org-domain-shaped label - see the module
    /// doc's "No pairing means no server-confirmed org".
    org_domain: String,
    /// `user://<org_domain>/<local OS user>` - the `decided_by` /
    /// `on_behalf_of` principal for every `decide_approval` call this
    /// process issues.
    operator: String,
    host: String,
    store_db_path: PathBuf,
    console_events_path: PathBuf,
    /// Temp world root (a throwaway Store + events file, the same shape
    /// `CloudHandle` seeds - see its module doc), removed on drop (best
    /// effort).
    dir: PathBuf,
}

#[uniffi::export]
impl WardryxHandle {
    /// Discover which Wardryx policy plane to talk to (a `taipan up`
    /// descriptor under `~/.taipan/environments/`, or `WARDRYX_ADMIN_KEY`
    /// for a Wardryx started by hand) and build a bearer client against it.
    /// Fails closed with [`WardryxError::NoEnvironment`] when neither source
    /// resolves - a normal, renderable "no policy plane" outcome (PHASE2.md),
    /// not a bug.
    #[uniffi::constructor]
    pub fn discover() -> Result<Self, WardryxError> {
        let resolved = env::discover().ok_or(WardryxError::NoEnvironment)?;
        Self::build(resolved)
    }

    /// Connect directly to `wardryx_url` with `admin_key`, skipping
    /// discovery - for a Wardryx the caller already knows how to reach (an
    /// operator-entered value, or a test harness).
    #[uniffi::constructor]
    pub fn connect(wardryx_url: String, admin_key: String) -> Result<Self, WardryxError> {
        Self::build(ResolvedEnv {
            source: WardryxEnvSource::EnvFallback,
            wardryx_url,
            admin_bearer: admin_key,
        })
    }

    /// Where this handle resolved its environment from.
    pub fn source(&self) -> WardryxEnvSource {
        self.source.clone()
    }

    /// The Wardryx base URL this handle talks to.
    pub fn wardryx_url(&self) -> String {
        self.wardryx_url.clone()
    }

    /// The locally-derived org-domain-shaped label - see the module doc's
    /// "No pairing means no server-confirmed org" for exactly what this is
    /// (and is not).
    pub fn org_domain(&self) -> String {
        self.org_domain.clone()
    }

    // ---- reads --------------------------------------------------------

    /// `GET /v1/approvals` - every approval visible to this bearer's org,
    /// flattened into [`ApprovalRecord`]s. Feeds both the Approvals Inbox
    /// (`pending == true`) and the decided-history list (PHASE2.md).
    pub fn list_approvals(&self) -> Result<Vec<ApprovalRecord>, WardryxError> {
        let approvals = self.runtime.block_on(self.client.list_approvals())?;
        Ok(approvals.iter().map(ApprovalRecord::from).collect())
    }

    /// `GET /v1/policies` - every stored policy, mapped into
    /// [`PolicyRecord`]s. Read-only in this wave (PHASE2.md: "the guarded
    /// PUT/DELETE editor is v1").
    pub fn list_policies(&self) -> Result<Vec<PolicyRecord>, WardryxError> {
        let policies = self.runtime.block_on(self.client.list_policies())?;
        Ok(policies.iter().map(PolicyRecord::from).collect())
    }

    // ---- the one privileged mutation ------------------------------------
    // ALWAYS attempts to journal a `console_command`, even when the Wardryx
    // call itself failed or was rejected - see `finish_decision`'s doc.

    /// Grant or deny a pending approval. `decided_by` is never a parameter
    /// here (unlike `WardryxClient::decide_approval`'s own signature): this
    /// handle always supplies its own resolved [`Self::operator`] principal,
    /// exactly like every `CloudHandle` mutation supplies its own operator
    /// rather than trusting a caller-provided identity string (PHASE2.md:
    /// "`decide_approval(id, Grant|Deny, decided_by=<operator principal>)`").
    ///
    /// The caller (SwiftUI) MUST have already cleared a local hardware
    /// confirmation (Touch ID) before calling this - see the module doc.
    /// This method has no way to enforce that itself (`LocalAuthentication`
    /// is a Swift/AppKit-only API, out of reach for this cross-platform
    /// crate), so it trusts the shell's own gate, the same trust boundary
    /// every UniFFI Object in this crate places on its Swift caller.
    pub fn decide_approval(
        &self,
        id: String,
        verdict: ApprovalVerdict,
    ) -> Result<ApprovalDecideOutcome, WardryxError> {
        let action: &'static str = match verdict {
            ApprovalVerdict::Grant => "console.grant_approval",
            ApprovalVerdict::Deny => "console.deny_approval",
        };
        let result = self.runtime.block_on(self.client.decide_approval(
            &id,
            verdict.into(),
            &self.operator,
        ));
        self.finish_decision(action, &id, result)
    }

    // ---- C2 audit link (docs/PHASE6-C2.md) ------------------------------

    /// Journal the fact that the operator approved a Felyx `ProposedAction`
    /// of `kind` targeting `target` (a GrantDeny proposal's approval id),
    /// ON TOP OF (never instead of) `decide_approval`'s own
    /// `console.grant_approval`/`console.deny_approval` line - mirrors
    /// [`crate::cloud::CloudHandle::journal_copilot_proposal_approved`]
    /// exactly (same fixed action name, same best-effort/infallible
    /// contract, same `self.journal` mechanism); see that method's own doc
    /// comment for the full rationale, not repeated here.
    pub fn journal_copilot_proposal_approved(&self, kind: String, target: String) -> bool {
        let rec = CommandRecord {
            operator: self.operator.clone(),
            env: "local".to_string(),
            action: "console.copilot_proposal_approved".to_string(),
            target,
            params: json!({ "kind": kind }),
            decision: "allow".to_string(),
            sig_alg: SIG_ALG.to_string(),
            sig_fpr: SIG_FPR.to_string(),
            http_status: 200,
            verify_result: "approved".to_string(),
        };
        self.journal(&rec).0
    }
}

// ---- private helpers (not exported over FFI) -------------------------------

impl WardryxHandle {
    /// Shared constructor body: build the bearer client, derive an
    /// org-domain-shaped label and operator principal (see the module doc),
    /// then seed a small local Store + events file so a decision can journal
    /// a `console_command`. Never panics; every fallible step folds into a
    /// [`WardryxError`].
    fn build(resolved: ResolvedEnv) -> Result<Self, WardryxError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| WardryxError::ConnectFailed {
                reason: format!("could not start async runtime: {e}"),
            })?;

        // `WardryxClient::new` is a plain local constructor (builds a
        // `reqwest::Client`; never touches the network) - no `block_on`
        // needed here, unlike `CloudHandle::build`'s pairing handshake.
        let client =
            WardryxClient::new(resolved.wardryx_url.clone(), resolved.admin_bearer.clone())
                .map_err(|e| WardryxError::ConnectFailed {
                    reason: e.to_string(),
                })?;

        let org_domain = match &resolved.source {
            WardryxEnvSource::Taipan { name } => sanitize_domain(name),
            WardryxEnvSource::EnvFallback => "genaryx.local".to_string(),
        };
        let operator = operator_principal(&org_domain);
        let host = local_hostname();

        let dir = fresh_world_dir().map_err(fs_error)?;
        let events_dir = dir.join("events");
        std::fs::create_dir_all(&events_dir).map_err(fs_error)?;
        let store_db_path = dir.join("console.sqlite");
        // Opened once here to run migrations / prove the store is writable;
        // `journal` reopens per call, matching `CloudHandle::build`'s
        // identical pattern (itself matching the Tauri shell's own
        // per-call `Store::open`).
        Store::open(&store_db_path).map_err(|e| WardryxError::Api {
            status: None,
            message: e.to_string(),
        })?;
        let console_events_path = events_dir.join("wardryx.ndjson");

        Ok(Self {
            runtime,
            client,
            source: resolved.source,
            wardryx_url: resolved.wardryx_url,
            org_domain,
            operator,
            host,
            store_db_path,
            console_events_path,
            dir,
        })
    }

    /// Journal one `CommandRecord` (best-effort: a journal failure is
    /// reported, never panics and never blocks the caller from learning
    /// Wardryx's own verdict).
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

    /// Shared tail end of [`WardryxHandle::decide_approval`]: build the
    /// `CommandRecord` from the already-resolved Wardryx outcome, ALWAYS
    /// attempt to journal it (regardless of that outcome - a rejected
    /// privileged attempt is itself part of the audit trail), then fold
    /// everything into either an [`ApprovalDecideOutcome`] or a
    /// [`WardryxError`] for the caller. Mirrors
    /// `CloudHandle::finish_mutation`'s shape (PHASE2.md: "reusing every
    /// existing convention... the fail-closed 'always journal the attempt'
    /// rule from `finish_mutation`"), adapted for Wardryx's `decision`
    /// (`"allow"` unconditionally - PHASE2.md: "the sanctioned
    /// human-in-the-loop path; a `break_glass` override of a DENY is
    /// separate Wave-3 work") and its decoded-token outcome shape instead of
    /// a generic `on_ok` closure.
    fn finish_decision(
        &self,
        action: &'static str,
        id: &str,
        call_result: Result<genaryx_connectors::ApprovalDecideResponse, ConnWardryxError>,
    ) -> Result<ApprovalDecideOutcome, WardryxError> {
        let now = SystemTime::now();
        let (http_status, verify_result, claims) = match &call_result {
            Ok(resp) => {
                let (text, claims) = describe_decision(resp, now);
                (200u16, text, claims)
            }
            Err(e) => (status_of(e), format!("error: {e}"), None),
        };

        let rec = CommandRecord {
            operator: self.operator.clone(),
            env: "local".to_string(),
            action: action.to_string(),
            target: id.to_string(),
            params: json!({}),
            decision: "allow".to_string(),
            sig_alg: SIG_ALG.to_string(),
            sig_fpr: SIG_FPR.to_string(),
            http_status,
            verify_result: verify_result.clone(),
        };
        let (bus_recorded, bus_error) = self.journal(&rec);

        match call_result {
            Ok(resp) => {
                let granted = resp.decision == "grant";
                Ok(ApprovalDecideOutcome {
                    approval_id: resp.approval_id,
                    granted,
                    summary: format!(
                        "approval {id} {}",
                        if granted { "granted" } else { "denied" }
                    ),
                    verify_result,
                    cost_ceiling_usd: claims.as_ref().map(|c| c.cost_ceiling_usd()),
                    ttl_seconds: claims.as_ref().map(|c| c.ttl_remaining(now).as_secs()),
                    expires_at_unix: claims.as_ref().map(|c| c.exp),
                    tools: claims.map(|c| c.tools).unwrap_or_default(),
                    bus_recorded,
                    bus_error,
                })
            }
            Err(e) => Err(WardryxError::from(e)),
        }
    }
}

impl Drop for WardryxHandle {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Fold an environment name (or, elsewhere, an org) into the `agent_id`-safe
/// charset `command::console_command_line` requires (07 §1,
/// `^agent://[a-z0-9.-]+/...`). A deliberate copy of
/// `cloud::mod::sanitize_domain` (that function is private to its own
/// sibling module) rather than a shared import - the same rationale
/// `cloud::mod::relock`'s own doc comment gives for its own tiny duplicated
/// helper.
fn sanitize_domain(raw: &str) -> String {
    let sanitized: String = raw
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

/// `user://<org_domain>/<local-user>`. A deliberate copy of
/// `cloud::mod::operator_principal` - see [`sanitize_domain`]'s doc comment.
fn operator_principal(org_domain: &str) -> String {
    let user = std::env::var("USER")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| "operator".to_string());
    format!("user://{org_domain}/{user}")
}

/// Best-effort local hostname, dependency-free by design. A deliberate copy
/// of `cloud::mod::local_hostname` - see [`sanitize_domain`]'s doc comment.
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
/// world: pid + per-process counter + nanos. Same shape as `cloud::mod`'s
/// own `fresh_world_dir`, disambiguated with a `-wardryx-` infix so a
/// `FleetHandle`, a `CloudHandle`, and a `WardryxHandle` constructed in the
/// same process never collide.
fn fresh_world_dir() -> std::io::Result<PathBuf> {
    static INSTANCE: AtomicU64 = AtomicU64::new(0);
    let n = INSTANCE.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-ffi-wardryx-{}-{n}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn fs_error(e: std::io::Error) -> WardryxError {
    WardryxError::Api {
        status: None,
        message: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    /// Rust-side stand-in proving `WardryxHandle` never panics when
    /// discovery finds nothing - the far more common case in CI than a live
    /// Wardryx being available at all.
    #[test]
    fn discover_without_an_environment_is_a_clean_error_not_a_panic() {
        // Does not touch `~/.taipan` or env vars; only proves the `Result`
        // shape, regardless of whether this box happens to have a real
        // `taipan up` environment or `WARDRYX_ADMIN_KEY` set (either a
        // `NoEnvironment`/`ConnectFailed` error or a genuine `Ok` are all
        // valid, non-panicking outcomes).
        match WardryxHandle::discover() {
            Ok(_) | Err(WardryxError::NoEnvironment | WardryxError::ConnectFailed { .. }) => {}
            Err(other) => panic!("unexpected error shape from discover(): {other:?}"),
        }
    }

    /// Unlike `CloudHandle::connect` (which fails on an unreachable URL
    /// because pairing is a real network round trip), `WardryxHandle::connect`
    /// never touches the network at construction time at all (see the module
    /// doc) - so this must succeed even against a port nothing is listening
    /// on. The actual unreachability only ever surfaces once a real call
    /// (`list_approvals`, ...) is made.
    #[test]
    fn connect_never_touches_the_network_even_against_an_unreachable_url() {
        let handle = WardryxHandle::connect("http://127.0.0.1:1".to_string(), "tk_dev".to_string())
            .expect("connect() must succeed locally regardless of reachability");
        assert_eq!(handle.wardryx_url(), "http://127.0.0.1:1");
        assert!(matches!(handle.source(), WardryxEnvSource::EnvFallback));
        assert_eq!(handle.org_domain(), "genaryx.local");
    }

    /// C2 (docs/PHASE6-C2.md): `journal_copilot_proposal_approved` is a pure
    /// local journal write (`self.journal`, exactly like every other
    /// mutation on this handle) - it never calls Wardryx at all, so this
    /// proves it end to end with no live server, mirroring the test above's
    /// own "never touches the network" setup. Checks the SAME two things
    /// every other journal test on this handle checks: the line lands on
    /// disk and it conforms to the agent-event schema.
    #[test]
    fn journal_copilot_proposal_approved_appends_a_conforming_link_line() {
        let handle = WardryxHandle::connect("http://127.0.0.1:1".to_string(), "tk_dev".to_string())
            .expect("connect() must succeed locally regardless of reachability");

        let recorded =
            handle.journal_copilot_proposal_approved("grant_deny".to_string(), "ap_1".to_string());
        assert!(recorded, "a fresh temp world must always be writable");

        let body = std::fs::read_to_string(&handle.console_events_path)
            .expect("read the console events file back");
        let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 1, "exactly one console_command line appended");

        let conformer = genaryx_core::Conformer::new().expect("embedded schemas must compile");
        let report = conformer.check_line(lines[0]);
        assert!(
            report.valid,
            "the appended link line must conform: {:?}\n  line: {}",
            report.errors, lines[0]
        );

        let value: serde_json::Value =
            serde_json::from_str(lines[0]).expect("parse the appended line");
        assert_eq!(
            value.get("type").and_then(|v| v.as_str()),
            Some("console_command")
        );
        assert_eq!(
            value
                .get("data")
                .and_then(|d| d.get("action"))
                .and_then(|v| v.as_str()),
            Some("console.copilot_proposal_approved"),
            "the link must journal under its OWN action name, distinct from console.grant_approval/console.deny_approval"
        );
        assert_eq!(
            value
                .get("data")
                .and_then(|d| d.get("target"))
                .and_then(|v| v.as_str()),
            Some("ap_1")
        );
        assert_eq!(
            value
                .get("data")
                .and_then(|d| d.get("decision"))
                .and_then(|v| v.as_str()),
            Some("allow"),
            "approving a proposal is not itself a break-glass override"
        );
    }

    // ==========================================================================
    // live e2e: real wardryx, a real policy seed + hold, a real grant through
    // the handle, a real console_command appended and re-read back off disk.
    // ==========================================================================
    // Same gated, hermetic, single-test-function shape as
    // `crates/connectors/tests/wardryx_test.rs` and `cloud::mod`'s own
    // live_e2e test (builds `wardryx` from `~/Development/wardryx` with
    // `go build`, on a fresh ephemeral port, torn down after), reused here
    // rather than reimplemented from scratch. The readiness probe is a plain
    // TCP connect rather than an HTTP `/healthz` GET (unlike
    // `wardryx_test.rs`): `genaryx-ffi` has no HTTP client dependency of its
    // own (same rationale `cloud::mod`'s own live_e2e test gives for the
    // same choice), and this test should not add one just to poll readiness
    // when a connect-then-grace-sleep is good enough for a local spawned
    // process.

    struct ChildGuard {
        child: Child,
        bin_path: PathBuf,
        events_path: PathBuf,
    }

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_file(&self.bin_path);
            let _ = std::fs::remove_file(&self.events_path);
        }
    }

    const BEARER: &str = "tk_ffi_test";
    const WARDRYX_KEYS: &str = "tk_ffi_test:test-org:admin";
    const APPROVAL_SECRET: &str = "genaryx-ffi-wardryx-test-secret-0123456789";

    fn free_port() -> Option<u16> {
        std::net::TcpListener::bind("127.0.0.1:0")
            .ok()
            .and_then(|l| l.local_addr().ok())
            .map(|a| a.port())
    }

    fn wardryx_repo() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok()?;
        let dir = PathBuf::from(home).join("Development/wardryx");
        dir.join("go.mod").is_file().then_some(dir)
    }

    fn build_wardryx(repo: &std::path::Path, bin_path: &std::path::Path) -> Result<(), String> {
        match Command::new("go")
            .arg("build")
            .arg("-o")
            .arg(bin_path)
            .arg("./cmd/wardryx")
            .current_dir(repo)
            .status()
        {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(format!(
                "`go build -o {} ./cmd/wardryx` failed ({status})",
                bin_path.display()
            )),
            Err(e) => Err(format!("could not run `go`: {e}")),
        }
    }

    fn spawn_wardryx(
        bin_path: &std::path::Path,
        addr: &str,
        events_path: &std::path::Path,
    ) -> Option<Child> {
        Command::new(bin_path)
            .arg("serve")
            .arg("-addr")
            .arg(addr)
            .arg("-events")
            .arg(events_path)
            .env("WARDRYX_KEYS", WARDRYX_KEYS)
            .env("WARDRYX_APPROVAL_SECRET", APPROVAL_SECRET)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()
    }

    /// Stand up a real `wardryx serve` on an ephemeral port and wait for it
    /// to start accepting TCP connections, plus a short grace sleep so the
    /// server has finished route setup before the real test traffic starts.
    fn try_start_wardryx() -> Option<(ChildGuard, String)> {
        let Some(repo) = wardryx_repo() else {
            eprintln!("genaryx-ffi wardryx live_e2e: SKIPPING: ~/Development/wardryx not found");
            return None;
        };
        let Some(port) = free_port() else {
            eprintln!("genaryx-ffi wardryx live_e2e: SKIPPING: could not reserve a port");
            return None;
        };

        let tmp = std::env::temp_dir();
        let unique = format!("genaryx-ffi-wardryx-test-{}-{port}", std::process::id());
        let bin_path = tmp.join(&unique);
        let events_path = tmp.join(format!("{unique}.ndjson"));

        if let Err(reason) = build_wardryx(&repo, &bin_path) {
            eprintln!("genaryx-ffi wardryx live_e2e: SKIPPING: {reason}");
            return None;
        }
        if !bin_path.is_file() {
            eprintln!(
                "genaryx-ffi wardryx live_e2e: SKIPPING: build succeeded but {} is missing",
                bin_path.display()
            );
            return None;
        }

        let addr = format!("127.0.0.1:{port}");
        let Some(mut child) = spawn_wardryx(&bin_path, &addr, &events_path) else {
            eprintln!(
                "genaryx-ffi wardryx live_e2e: SKIPPING: failed to spawn {}",
                bin_path.display()
            );
            let _ = std::fs::remove_file(&bin_path);
            return None;
        };

        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                eprintln!(
                    "genaryx-ffi wardryx live_e2e: SKIPPING: wardryx exited early ({status})"
                );
                let _ = std::fs::remove_file(&bin_path);
                let _ = std::fs::remove_file(&events_path);
                return None;
            }
            if std::net::TcpStream::connect(&addr).is_ok() {
                std::thread::sleep(Duration::from_millis(300));
                return Some((
                    ChildGuard {
                        child,
                        bin_path,
                        events_path,
                    },
                    format!("http://{addr}"),
                ));
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                eprintln!("genaryx-ffi wardryx live_e2e: SKIPPING: wardryx never opened its port");
                let _ = std::fs::remove_file(&bin_path);
                let _ = std::fs::remove_file(&events_path);
                return None;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    #[test]
    fn live_e2e_hold_grant_via_handle_and_console_command_journal() {
        let Some((_guard, base)) = try_start_wardryx() else {
            return; // already explained why via eprintln! above
        };

        let handle = WardryxHandle::connect(base.clone(), BEARER.to_string())
            .expect("WardryxHandle::connect must build a bearer client");
        assert_eq!(handle.wardryx_url(), base);
        assert!(matches!(handle.source(), WardryxEnvSource::EnvFallback));

        // ---- seed a require_human_above_usd policy, via the raw connector
        // client directly (this handle exposes no PUT - read-only in this
        // wave), reusing the handle's own runtime + client so this is
        // genuinely the same bearer session `list_approvals`/
        // `decide_approval` below will use. ----
        let policy = genaryx_connectors::Policy {
            target: "agent://test-org/*".to_string(),
            require_human_above_usd: 1.0,
            ..Default::default()
        };
        handle
            .runtime
            .block_on(handle.client.put_policy("demo", &policy))
            .expect("PUT /v1/policies/demo");

        // ---- drive a hold directly via WardryxClient::decide (this handle
        // exposes no decide() either - only list_approvals/list_policies/
        // decide_approval, per PHASE2.md's method list). ----
        let decide_req = genaryx_connectors::DecideRequest {
            agent_id: "agent://test-org/payments".to_string(),
            run_id: "run-1".to_string(),
            tool_names: vec!["charge".to_string()],
            est_cost_usd: 50.0,
            ..Default::default()
        };
        let hold = handle
            .runtime
            .block_on(handle.client.decide(&decide_req))
            .expect("POST /v1/decide over threshold");
        assert_eq!(hold.decision, "hold");
        assert!(!hold.approval_id.is_empty());

        // ---- list_approvals: the handle's own exported read ----
        let approvals = handle.list_approvals().expect("list_approvals");
        let pending = approvals
            .iter()
            .find(|a| a.approval_id == hold.approval_id)
            .expect("the just-created hold must appear via the handle");
        assert!(pending.pending);
        assert_eq!(pending.tools, vec!["charge".to_string()]);
        assert!((pending.est_cost_usd.unwrap_or(0.0) - 50.0).abs() < 0.01);
        assert!(!pending.reason.clone().unwrap_or_default().is_empty());

        // ---- list_policies: the handle's own exported read ----
        let policies = handle.list_policies().expect("list_policies");
        assert!(
            policies.iter().any(|p| p.id == "demo"),
            "the freshly seeded policy must appear via the handle"
        );

        // ---- decide_approval(grant): the one privileged mutation ----
        let outcome = handle
            .decide_approval(hold.approval_id.clone(), ApprovalVerdict::Grant)
            .expect("grant must be accepted");
        assert!(outcome.granted);
        assert_eq!(outcome.approval_id, hold.approval_id);
        let ceiling = outcome
            .cost_ceiling_usd
            .expect("a decoded grant must carry a cost ceiling");
        assert!((ceiling - 50.0).abs() < 0.01);
        assert!(outcome.ttl_seconds.unwrap_or(0) > 0);
        assert!(outcome.expires_at_unix.unwrap_or(0) > 0);
        assert_eq!(outcome.tools, vec!["charge".to_string()]);
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
                .and_then(|d| d.get("action"))
                .and_then(|v| v.as_str()),
            Some("console.grant_approval")
        );
        assert_eq!(
            value
                .get("data")
                .and_then(|d| d.get("decision"))
                .and_then(|v| v.as_str()),
            Some("allow")
        );
        assert_eq!(
            value
                .get("data")
                .and_then(|d| d.get("target"))
                .and_then(|v| v.as_str()),
            Some(hold.approval_id.as_str())
        );

        // ---- decide_approval(deny): the same privileged mutation, other
        // verdict - a second hold, denied this time, proving
        // `finish_decision`'s "denied" branch (action
        // `console.deny_approval`, no decoded-claim fields) journals a
        // second conforming line too, not just the grant path above. ----
        let decide_req_2 = genaryx_connectors::DecideRequest {
            agent_id: "agent://test-org/payments".to_string(),
            run_id: "run-2".to_string(),
            tool_names: vec!["charge".to_string()],
            est_cost_usd: 50.0,
            ..Default::default()
        };
        let hold2 = handle
            .runtime
            .block_on(handle.client.decide(&decide_req_2))
            .expect("POST /v1/decide over threshold, second hold");
        assert_eq!(hold2.decision, "hold");

        let deny_outcome = handle
            .decide_approval(hold2.approval_id.clone(), ApprovalVerdict::Deny)
            .expect("deny must be accepted");
        assert!(!deny_outcome.granted);
        assert_eq!(deny_outcome.approval_id, hold2.approval_id);
        assert_eq!(deny_outcome.verify_result, "denied");
        assert!(deny_outcome.cost_ceiling_usd.is_none());
        assert!(deny_outcome.ttl_seconds.is_none());
        assert!(deny_outcome.expires_at_unix.is_none());
        assert!(deny_outcome.tools.is_empty());
        assert!(
            deny_outcome.bus_recorded,
            "deny must also be journaled: {:?}",
            deny_outcome.bus_error
        );

        let body2 = std::fs::read_to_string(&handle.console_events_path)
            .expect("read the console events file back, after the deny");
        let lines2: Vec<&str> = body2.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(
            lines2.len(),
            2,
            "the grant line plus the deny line, both appended"
        );

        let deny_report = conformer.check_line(lines2[1]);
        assert!(
            deny_report.valid,
            "appended deny console_command must conform: {:?}\n  line: {}",
            deny_report.errors, lines2[1]
        );
        let deny_value: serde_json::Value =
            serde_json::from_str(lines2[1]).expect("parse the appended deny line");
        assert_eq!(
            deny_value
                .get("data")
                .and_then(|d| d.get("action"))
                .and_then(|v| v.as_str()),
            Some("console.deny_approval")
        );
        assert_eq!(
            deny_value
                .get("data")
                .and_then(|d| d.get("decision"))
                .and_then(|v| v.as_str()),
            Some("allow"),
            "a sanctioned deny still journals decision:allow (PHASE2.md - break_glass is separate Wave-3 work)"
        );
        assert_eq!(
            deny_value
                .get("data")
                .and_then(|d| d.get("target"))
                .and_then(|v| v.as_str()),
            Some(hold2.approval_id.as_str())
        );

        eprintln!(
            "genaryx-ffi wardryx live_e2e: PASSED - policy seeded, hold {} granted and hold {} \
             denied via the handle, both console_command lines appended to {} and conform",
            hold.approval_id,
            hold2.approval_id,
            handle.console_events_path.display()
        );
    }
}
