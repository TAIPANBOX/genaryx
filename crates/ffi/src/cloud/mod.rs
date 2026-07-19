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
pub mod evidence;
pub mod evidence_env;

pub use dto::{CloudError, Incident, MutationOutcome, Overview, Run, Savings};
pub use env::EnvSource;
pub use evidence::{
    EvidenceBuildInputs, EvidenceError, EvidenceLoadEntry, EvidenceManifestRecord,
    EvidencePackRecord, ManifestArtifactRecord, MissingSourceRecord,
};
pub use evidence_env::EvidenceEnvDefaultsRecord;

use dto::{build_run, status_of};
use env::ResolvedEnv;
use evidence::non_blank;
use genaryx_connectors::{CloudClient, ConnectorError, QryxClient, TokenfuseClient};
use genaryx_core::CommandRecord;
use genaryx_core::command;
use genaryx_core::store::Store;
use genaryx_signing::{Es256Signer, SoftwareSigner};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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

    /// The `user://<org_domain>/<local-user>` principal every
    /// `console_command`/`console_evidence_built` record on this handle is
    /// journaled under - the Evidence Center's default "operator" label
    /// (docs/PHASE4.md W3). Named `console_operator`, not `operator`: the
    /// latter is a Swift keyword (see `evidence`'s own module doc for why
    /// this crate avoids that class of friction wherever a Swift binding
    /// would have to read the value back, not just construct it once) - and
    /// this crate already has a private free function called
    /// `operator_principal` (this handle's own pairing-time computation,
    /// below), so reusing that exact name here would additionally shadow-
    /// confuse two genuinely different things with the same identifier.
    pub fn console_operator(&self) -> String {
        self.operator.clone()
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

    // ---- C2 audit link (docs/PHASE6-C2.md) ------------------------------

    /// Journal the fact that the operator approved a Felyx `ProposedAction`
    /// of `kind` targeting `target`, ON TOP OF (never instead of) the
    /// mutation's own `console.kill_run`/`console.set_budget` line above -
    /// so the audit trail shows both WHAT happened (that line) and THAT a
    /// copilot recommended it and a human approved it (this one), never
    /// "copilot did Z". Uses the exact same `command::record` mechanism
    /// every other journal call on this handle uses (`self.journal`), just
    /// with its own fixed action name (`console.copilot_proposal_approved`)
    /// and `decision: "allow"` (approving a recommendation is not itself a
    /// governance override - the underlying mutation's own `decision`
    /// already carries that). Best-effort and infallible by design, exactly
    /// like every other journal call here: a failure to journal the LINK
    /// must never be mistaken for the approved action itself having failed
    /// (that already happened, successfully, before this is ever called -
    /// see `apps/macos`'s `GenaryxApp` approve routing, which only calls
    /// this after `killRun`/`setBudget` itself reports success). Returns
    /// whether the line was actually appended, so the shell can note an
    /// honest "not journaled" outcome rather than silently assuming success.
    pub fn journal_copilot_proposal_approved(&self, kind: String, target: String) -> bool {
        let rec = CommandRecord {
            operator: self.operator.clone(),
            env: "local".to_string(),
            action: "console.copilot_proposal_approved".to_string(),
            target,
            params: json!({ "kind": kind }),
            decision: "allow".to_string(),
            sig_alg: "es256".to_string(),
            sig_fpr: self.sig_fpr.to_string(),
            http_status: 200,
            verify_result: "approved".to_string(),
        };
        self.journal(&rec).0
    }

    // ---- Evidence Center (Phase-4 W3) -----------------------------------
    // Builds through the SAME paired `CloudClient` (compliance/audit reads
    // AND the ES256 manifest signer) Money/Overview already use - never a
    // second device pairing (see `evidence`'s own module doc). Unlike
    // `kill_run`/`set_budget`/`ack_incident` above, a build is not itself a
    // remote mutation (it reads Cloud/Qryx/idryx/TokenFuse and produces a
    // local artifact), so it does not go through `finish_mutation` - see
    // `build_evidence_pack`'s own doc for its bespoke journal step.

    /// Best-effort pre-fill for the Evidence panel's editable source fields
    /// (qryx/idryx/tokenfuse binaries + targets) - a pure local filesystem
    /// read, never a subprocess spawn or network call, so it is cheap to
    /// call once when the panel becomes usable (mirrors
    /// `CryptoHandle::default_scan_target`/`DrillsHandle::default_scenario_dir`'s
    /// own "operator can see/set it, never enforced" contract). See
    /// `evidence_env`'s own module doc for exactly what is resolved from
    /// where.
    pub fn evidence_env_defaults(&self) -> EvidenceEnvDefaultsRecord {
        evidence_env::resolve_defaults()
    }

    /// Assemble a signed evidence pack from every requested source (Cloud
    /// compliance + audit verdict when `include_cloud`, Qryx crypto evidence
    /// + CBOM when both `qryx_bin`/`qryx_target` resolve, idryx Agent-BOM
    /// when `idryx_bin` resolves, TokenFuse FOCUS export when both
    /// `tokenfuse_bin`/`tokenfuse_traces_dir` resolve), then journals a
    /// `console_evidence_built` record through the SAME Store +
    /// `command::record` path every mutation above uses (docs/PHASE4.md W3:
    /// "it IS a console action producing an artifact, so it records like
    /// every other mutation"). A source whose input is `None`/blank is
    /// simply left out - never an error on its own; a source that WAS
    /// requested but failed is recorded as a [`MissingSourceRecord`] in the
    /// returned manifest, never silently dropped (see
    /// `genaryx_connectors::build_evidence_pack`'s own doc). `include_cloud:
    /// false`, or no device signer attached, yields an honestly-UNSIGNED
    /// pack ([`EvidencePackRecord::signed`] `= false`) - never an error and
    /// never a false "signed" claim; only [`EvidenceError::NoArtifacts`]
    /// (nothing at all could be gathered) and a GENUINE signing/assembly
    /// failure surface as errors.
    ///
    /// The journal step is best-effort: journal failure is logged to
    /// stderr and reflected in [`EvidencePackRecord::journaled`], but never
    /// discards the pack the operator already successfully built - unlike
    /// `kill_run`/`set_budget`/`ack_incident`, there is no remote mutation
    /// outcome to report even on a journal failure (the pack was produced
    /// purely locally), so there is nothing this method should fail closed
    /// over here that would not also throw away a real, already-built
    /// artifact.
    pub fn build_evidence_pack(
        &self,
        inputs: EvidenceBuildInputs,
    ) -> Result<EvidencePackRecord, EvidenceError> {
        let operator = non_blank(inputs.operator_name).unwrap_or_else(|| self.operator.clone());
        let org = non_blank(inputs.org).unwrap_or_else(|| self.org_domain.clone());

        let qryx_bin = non_blank(inputs.qryx_bin);
        let qryx_target = non_blank(inputs.qryx_target).map(PathBuf::from);
        let qryx_sign_key = non_blank(inputs.qryx_sign_key).map(PathBuf::from);
        let qryx_client = qryx_bin.map(QryxClient::new);
        let qryx_input: Option<(&QryxClient, &Path, Option<&Path>)> =
            match (&qryx_client, &qryx_target) {
                (Some(client), Some(target)) => {
                    Some((client, target.as_path(), qryx_sign_key.as_deref()))
                }
                _ => None,
            };

        let idryx_bin = non_blank(inputs.idryx_bin).map(PathBuf::from);
        let idryx_loads: Vec<(&str, &str)> = inputs
            .idryx_loads
            .iter()
            .map(|e| (e.source.as_str(), e.path.as_str()))
            .collect();
        let idryx_input: Option<(&Path, &[(&str, &str)])> = idryx_bin
            .as_deref()
            .map(|bin| (bin, idryx_loads.as_slice()));

        let tokenfuse_bin = non_blank(inputs.tokenfuse_bin);
        let tokenfuse_traces = non_blank(inputs.tokenfuse_traces_dir).map(PathBuf::from);
        let tokenfuse_from = non_blank(inputs.tokenfuse_from);
        let tokenfuse_to = non_blank(inputs.tokenfuse_to);
        let tokenfuse_client = tokenfuse_bin.map(TokenfuseClient::new);
        let tokenfuse_input: Option<(&TokenfuseClient, &Path, Option<&str>, Option<&str>)> =
            match (&tokenfuse_client, &tokenfuse_traces) {
                (Some(client), Some(traces)) => Some((
                    client,
                    traces.as_path(),
                    tokenfuse_from.as_deref(),
                    tokenfuse_to.as_deref(),
                )),
                _ => None,
            };

        let evidence_inputs = genaryx_connectors::EvidenceInputs {
            operator: &operator,
            org: &org,
            generated_at: &inputs.generated_at,
            include_cloud: inputs.include_cloud,
            qryx: qryx_input,
            idryx: idryx_input,
            tokenfuse: tokenfuse_input,
        };

        let pack = self
            .runtime
            .block_on(genaryx_connectors::build_evidence_pack(
                &self.client,
                evidence_inputs,
            ))?;

        let sha256 = evidence::sha256_hex(&pack.zip_bytes);
        let artifact_count = pack.manifest.artifacts.len();
        let missing_count = pack.manifest.missing.len();
        let signed = pack.signed;

        let rec = CommandRecord {
            operator: self.operator.clone(),
            env: "local".to_string(),
            action: "console.evidence_built".to_string(),
            target: sha256.clone(),
            params: json!({
                "sha256": sha256,
                "artifact_count": artifact_count,
                "signed": signed,
                "missing_count": missing_count,
                "operator": operator,
                "org": org,
            }),
            decision: "allow".to_string(),
            sig_alg: "es256".to_string(),
            sig_fpr: self.sig_fpr.to_string(),
            http_status: 200,
            verify_result: format!(
                "signed:{signed} artifacts:{artifact_count} missing:{missing_count}"
            ),
        };
        let (journaled, journal_err) = self.journal(&rec);
        if let Some(err) = &journal_err {
            eprintln!(
                "genaryx-ffi cloud evidence: console_evidence_built journal failed (pack still \
                 returned): {err}"
            );
        }

        Ok(EvidencePackRecord {
            zip_bytes: pack.zip_bytes,
            manifest: EvidenceManifestRecord::from(&pack.manifest),
            signed,
            journaled,
        })
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

        // ---- C2 (docs/PHASE6-C2.md): the copilot-proposal audit link, ON
        // TOP OF the console.kill_run line above, never instead of it ----
        let linked = handle.journal_copilot_proposal_approved("kill".to_string(), run_id.clone());
        assert!(linked, "the audit link must journal successfully too");

        let body2 = std::fs::read_to_string(&handle.console_events_path)
            .expect("read the console events file back, after the audit link");
        let lines2: Vec<&str> = body2.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(
            lines2.len(),
            2,
            "the console.kill_run line plus the copilot_proposal_approved link line, both appended"
        );

        let link_report = conformer.check_line(lines2[1]);
        assert!(
            link_report.valid,
            "the appended link line must conform: {:?}\n  line: {}",
            link_report.errors, lines2[1]
        );
        let link_value: serde_json::Value =
            serde_json::from_str(lines2[1]).expect("parse the appended link line");
        assert_eq!(
            link_value
                .get("data")
                .and_then(|d| d.get("action"))
                .and_then(|v| v.as_str()),
            Some("console.copilot_proposal_approved"),
            "the link must journal under its OWN action name, distinct from console.kill_run"
        );
        assert_eq!(
            link_value
                .get("data")
                .and_then(|d| d.get("target"))
                .and_then(|v| v.as_str()),
            Some(run_id.as_str())
        );

        eprintln!(
            "genaryx-ffi cloud live_e2e: PASSED - paired against {base}, overview read, signed kill of \
             {run_id} accepted, console_command appended to {} and conforms, plus the \
             copilot_proposal_approved audit link",
            handle.console_events_path.display()
        );
    }

    /// Live e2e for the Evidence Center (docs/PHASE4.md W3): a real,
    /// freshly-paired `CloudHandle` builds a Cloud-only pack (every other
    /// source deliberately `None`, so this proves the CloudHandle integration
    /// itself - the hand-written, security-relevant part of this wave -
    /// without needing external qryx/idryx/tokenfuse checkouts too). Same
    /// gated, hermetic shape as the kill test above: reuses
    /// [`try_start_cloud`], skips gracefully when `~/Development/tokenfuse`
    /// is unavailable.
    #[test]
    fn live_e2e_build_evidence_pack_cloud_only_signs_and_journals() {
        let Some((_guard, base)) = try_start_cloud() else {
            return; // already explained why via eprintln! above
        };

        let handle = CloudHandle::connect(base.clone(), "devkey".to_string())
            .expect("CloudHandle::connect must pair against a live Cloud");

        let inputs = EvidenceBuildInputs {
            operator_name: None, // falls back to console_operator()
            org: None,           // falls back to org_domain()
            generated_at: "2026-07-17T12:00:00.000Z".to_string(),
            include_cloud: true,
            qryx_bin: None,
            qryx_target: None,
            qryx_sign_key: None,
            idryx_bin: None,
            idryx_loads: Vec::new(),
            tokenfuse_bin: None,
            tokenfuse_traces_dir: None,
            tokenfuse_from: None,
            tokenfuse_to: None,
        };

        let pack = handle
            .build_evidence_pack(inputs)
            .expect("a Cloud-only build against a live, freshly-paired Cloud must succeed");

        // ---- the pack itself ----
        assert!(
            !pack.zip_bytes.is_empty(),
            "a built pack must have real bytes"
        );
        assert_eq!(
            &pack.zip_bytes[..4],
            b"PK\x03\x04",
            "zip_bytes must start with the zip local-file-header magic"
        );
        assert!(
            pack.signed,
            "a freshly-paired CloudHandle always has a device signer attached, so this must be \
             signed, never a false UNSIGNED claim"
        );

        // ---- the manifest: exactly the two Cloud sources were attempted,
        // each landing as either an artifact or an honest MissingSource,
        // never silently dropped ----
        assert_eq!(pack.manifest.pack_version, "genaryx-evidence/v1");
        assert_eq!(pack.manifest.generated_at, "2026-07-17T12:00:00.000Z");
        assert_eq!(pack.manifest.operator_name, handle.console_operator());
        assert_eq!(pack.manifest.org, handle.org_domain());
        assert_eq!(
            pack.manifest.artifacts.len() + pack.manifest.missing.len(),
            2,
            "include_cloud alone attempts exactly two sources (compliance evidence + audit \
             verdict): {:?} / {:?}",
            pack.manifest.artifacts,
            pack.manifest.missing
        );
        for artifact in &pack.manifest.artifacts {
            assert!(artifact.sha256.starts_with("sha256:"));
            assert!(artifact.size_bytes > 0);
        }

        // ---- confirm the console_evidence_built line landed and conforms ----
        let body = std::fs::read_to_string(&handle.console_events_path)
            .expect("read the console events file back");
        let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(
            lines.len(),
            1,
            "exactly one console_command line appended for the evidence build"
        );

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
            value.get("type").and_then(|v| v.as_str()),
            Some("console_command")
        );
        assert_eq!(
            value
                .get("data")
                .and_then(|d| d.get("action"))
                .and_then(|v| v.as_str()),
            Some("console.evidence_built")
        );
        assert_eq!(
            value
                .get("data")
                .and_then(|d| d.get("decision"))
                .and_then(|v| v.as_str()),
            Some("allow"),
            "an evidence build is not a break-glass override"
        );

        eprintln!(
            "genaryx-ffi cloud live_e2e: PASSED - Evidence Center: paired against {base}, built a \
             {}-byte pack ({} artifacts, {} missing, signed={}), console_evidence_built appended and \
             conforms",
            pack.zip_bytes.len(),
            pack.manifest.artifacts.len(),
            pack.manifest.missing.len(),
            pack.signed
        );
    }
}
