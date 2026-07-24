//! Console commands for the Copilot view (Phase 6, C0 - docs/PHASE6.md,
//! itrat-console/13): [`copilot_status`] (a flat DTO for the residency
//! banner) and [`copilot_ask`] (one question/answer round trip through
//! Felyx, the hand-rolled agent loop `genaryx-copilot` owns).
//!
//! Unlike every other panel's status command, [`copilot_status`] returns a
//! flat struct rather than a tagged `#[serde(tag = "state", ...)]` enum:
//! there is no environment to discover and no reachability to probe here
//! (see `state.rs`'s module doc), so the only thing worth telling the
//! frontend is "is the copilot enabled, and if so with what residency" -
//! `CopilotService::is_enabled`/`descriptor`/`disabled_reason` map onto this
//! DTO's fields directly.
//!
//! [`copilot_ask`] returns `Result<Answer, String>`, not a structured error
//! DTO like `IdentityError`/`PolicyError`: `genaryx_copilot::Answer` already
//! derives `Serialize` (mirrors `identity::commands`'s reuse of
//! `genaryx_connectors::Idryx*` DTOs directly - no UI-facing mirror struct
//! needed), and `CopilotError` (`thiserror`-derived `Display`) has exactly
//! one message worth showing an operator, so its `.to_string()` - e.g.
//! `CopilotError::NoProvider`'s "no copilot provider is configured; set
//! [copilot].provider (local by default)" - IS the error, not a code the
//! frontend has to translate.
//!
//! C1 (docs/PHASE6-C1.md) adds [`copilot_explain`]: the same one-round-trip
//! shape as [`copilot_ask`], just seeded with `CopilotService::explain_incident`'s
//! fixed, incident-focused prompt instead of an operator-typed question - the
//! "Explain with Felyx" affordance on the Incidents surface calls this
//! directly rather than composing the prompt itself, so the prompt stays one
//! reviewable thing on the Rust side.
//!
//! C2 (docs/PHASE6-C2.md, "Felyx propose-and-confirm") adds
//! [`copilot_log_proposal_approved`]: NOT a mutation, and NOT a new signing
//! path. `Answer.proposals` (already flowing through [`copilot_ask`]/
//! [`copilot_explain`] as of the crate's C2 cut - `genaryx_copilot::Answer`
//! gained the field, `Answer` still derives `Serialize` so nothing here had
//! to change for it to reach the frontend) are display-only
//! `ProposedAction`s the crate holds no signer for
//! (`crates/copilot/src/action.rs`'s own doc comment: "There is deliberately
//! no `Act` here"). The shell renders each as an approve/dismiss card
//! (`CopilotView.tsx`'s `ProposalCard`); clicking Approve calls the EXACT
//! SAME existing signed command a manual click already would
//! (`money::commands::money_kill_run`/`money_set_budget`,
//! `policy::commands::policy_decide_approval`,
//! `identity::commands::identity_rescan` - see `CopilotView.tsx`'s
//! `runApproval`), never a copilot-specific mutation path. Only AFTER that
//! real signed call has already succeeded does the frontend call
//! [`copilot_log_proposal_approved`], which journals one
//! `console.copilot_proposal_approved` `CommandRecord` linking the proposal
//! to the human's decision - so the audit trail reads "human X approved
//! copilot proposal Y", never "copilot did Z". See
//! [`copilot_log_proposal_approved`]'s own doc comment for why it reuses the
//! Money plane's bus (`crate::money::state::MoneyState`) rather than owning
//! one, exactly the way `evidence::commands::evidence_build` already reuses
//! Money's paired `CloudClient`.

use std::sync::Arc;

use genaryx_copilot::{Answer, CopilotService};
use serde::Serialize;
use serde_json::{Value, json};

use super::state::{CopilotInner, CopilotState};
use crate::money::state::{MoneyClient, MoneyInner, MoneyState};
use genaryx_core::{CommandRecord, command};

// ============================================================================
// DTOs
// ============================================================================

/// The residency banner's whole data model - mirrors
/// `identity::commands::IdentityStatusDto` in spirit (never inferred from a
/// read command's error shape) but flat, not tagged (see this module's doc
/// comment). `provider`/`model`/`endpoint`/`local` are `Some` together
/// exactly when a [`genaryx_copilot::ProviderDescriptor`] exists to show;
/// `disabled_reason` is `Some` exactly when the service is disabled.
#[derive(Debug, Clone, Serialize)]
pub struct CopilotStatusDto {
    pub enabled: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub local: Option<bool>,
    pub disabled_reason: Option<String>,
}

impl CopilotStatusDto {
    /// The disabled shape, used for every non-`Ready` [`CopilotInner`]
    /// (`Bootstrapping`, `Failed`) - `reason` is always operator-readable
    /// text, never a bare error code.
    fn disabled(reason: impl Into<String>) -> Self {
        Self {
            enabled: false,
            provider: None,
            model: None,
            endpoint: None,
            local: None,
            disabled_reason: Some(reason.into()),
        }
    }

    /// Read the live DTO off a resolved [`CopilotService`]: the residency
    /// descriptor plus fields when a provider is configured, or
    /// [`Self::disabled`] with the service's own explanation when not.
    /// `enabled` always comes straight from `is_enabled()` rather than being
    /// inferred from `descriptor().is_some()`, so this stays correct even if
    /// that equivalence ever stops holding on the crate side.
    fn from_service(service: &CopilotService) -> Self {
        let enabled = service.is_enabled();
        match service.descriptor() {
            Some(d) => Self {
                enabled,
                provider: Some(d.provider),
                model: Some(d.model),
                endpoint: Some(d.endpoint),
                local: Some(d.local),
                disabled_reason: service.disabled_reason().map(str::to_string),
            },
            None => Self::disabled(
                service
                    .disabled_reason()
                    .unwrap_or("No copilot provider is configured.")
                    .to_string(),
            ),
        }
    }
}

/// What [`copilot_log_proposal_approved`] reports back - mirrors
/// `evidence::commands::EvidenceBuildDto`'s `journaled`/`journal_error`
/// pairing exactly: whether the audit link itself got journaled is surfaced
/// honestly to the frontend, never thrown, since the real signed mutation
/// this links back to has ALREADY succeeded by the time this command is
/// ever called (see this module's doc comment) - a journaling hiccup here
/// must never read as that mutation having failed.
#[derive(Debug, Clone, Serialize)]
pub struct ProposalApprovedOutcome {
    pub journaled: bool,
    pub journal_error: Option<String>,
}

// ============================================================================
// helpers
// ============================================================================

/// Resolve the current `Arc<CopilotService>` out of managed state, or a
/// plain operator-readable message when the panel is not ready yet - mirrors
/// `identity::commands::ready_client`'s "clone the cheap handle out of the
/// lock" shape, but returns `String` (not a structured error type) since
/// that is [`copilot_ask`]'s own error type (see this module's doc comment).
async fn ready_service(state: &&CopilotState) -> Result<Arc<CopilotService>, String> {
    let guard = state.inner.lock().await;
    match &*guard {
        CopilotInner::Ready(service) => Ok(Arc::clone(service)),
        CopilotInner::Bootstrapping => {
            Err("Copilot is still starting up; try again in a moment.".to_string())
        }
        CopilotInner::Failed(reason) => Err(reason.clone()),
    }
}

/// Journal one `console.copilot_proposal_approved` `CommandRecord` (C2,
/// docs/PHASE6-C2.md "Audit metadata"): the link between a copilot
/// `ProposedAction` and the human's own decision to approve it. Best-effort,
/// mirroring `money::commands::journal`/`evidence::commands::journal_build`'s
/// identical discipline exactly (a failure is reported, never panics, never
/// blocks the caller) - by the time this runs the real signed mutation this
/// entry links back to has ALREADY happened on its own existing path, so a
/// journal hiccup here is honestly reported, never escalated into looking
/// like that mutation failed.
///
/// `kind`/`target`/`params` (the proposal's own action parameters, e.g.
/// `{"usd_cap": 5}`) are folded into ONE `params` object on the
/// [`CommandRecord`] - `commands_journal`'s queryable audit row then reads
/// exactly "kind X approved for target Y with params Z", while the emitted
/// `console_command` bus event keeps the SAME fixed `data` shape every other
/// console mutation already produces (`action`/`target`/`decision`/`sig_alg`/
/// `sig_fpr`/`http_status`/`verify_result` - see
/// `genaryx_core::command::console_command_line`'s doc comment for why
/// `params` never rides in that fixed shape). `decision: "allow"`, not
/// `"break_glass"`: this entry does not itself override any governance
/// decision (whatever kill/budget override already happened journaled that
/// separately, under its own `console.kill_run`/`console.set_budget` action,
/// with its own operator-supplied reason) - it only records that a human
/// reviewed and approved a specific Felyx recommendation, the same
/// non-override shape `policy::commands::policy_decide_approval` and
/// `evidence::commands::journal_build` already use for their own links.
fn journal_proposal_approved(
    mc: &MoneyClient,
    kind: &str,
    target: &str,
    params: &Value,
) -> (bool, Option<String>) {
    let Some(bus) = &mc.bus else {
        return (
            false,
            Some("no live event bus available (startup seeding did not complete)".to_string()),
        );
    };
    let rec = CommandRecord {
        operator: mc.operator.clone(),
        env: "local".to_string(),
        action: "console.copilot_proposal_approved".to_string(),
        target: target.to_string(),
        params: json!({ "kind": kind, "target": target, "params": params }),
        decision: "allow".to_string(),
        sig_alg: "es256".to_string(),
        sig_fpr: mc.sig_fpr.to_string(),
        http_status: 200,
        verify_result: format!("copilot proposal approved: kind={kind} target={target}"),
    };
    match genaryx_core::store::Store::open(&bus.store_db_path) {
        Ok(store) => match command::record(
            &store,
            &bus.console_events_path,
            &mc.org_domain,
            &mc.host,
            &rec,
        ) {
            Ok(()) => (true, None),
            Err(e) => (false, Some(e.to_string())),
        },
        Err(e) => (false, Some(e.to_string())),
    }
}

// ============================================================================
// commands
// ============================================================================

/// Whole-panel status for the residency banner. Never fails: every
/// [`CopilotInner`] shape (including `Bootstrapping`/`Failed`) maps onto a
/// renderable [`CopilotStatusDto`].
pub async fn copilot_status(state: &CopilotState) -> Result<CopilotStatusDto, ()> {
    let guard = state.inner.lock().await;
    Ok(match &*guard {
        CopilotInner::Bootstrapping => CopilotStatusDto::disabled("Copilot is still starting up."),
        CopilotInner::Ready(service) => CopilotStatusDto::from_service(service),
        CopilotInner::Failed(reason) => CopilotStatusDto::disabled(reason.clone()),
    })
}

/// Ask Felyx one question. Runs the bounded agent loop
/// (`genaryx_copilot::Felyx::answer`, via `CopilotService::ask`) over
/// whatever tools are registered for today's `Clients::default()` (none yet
/// in C0 - see `state.rs`'s module doc), so C0 answers come from the model's
/// own text only; `tool_trace` is still always present on the returned
/// [`Answer`] (empty today) so the frontend's evidence rendering needs no
/// change once a later cut wires real clients through.
///
/// `Err` is always operator-readable text, not a code: with today's default
/// config this is virtually always `CopilotError::NoProvider`'s message (no
/// LLM configured on this box) - the frontend renders it as an assistant
/// note rather than treating it as a crash (see `CopilotView.tsx`).
pub async fn copilot_ask(state: &CopilotState, question: String) -> Result<Answer, String> {
    let service = ready_service(&state).await?;
    service.ask(&question).await.map_err(|e| e.to_string())
}

/// "Explain with Felyx" on an incident (docs/PHASE6-C1.md, itrat-console/13
/// D13.7 C1): mirrors [`copilot_ask`] field-for-field, running
/// `CopilotService::explain_incident` instead of a free-form `ask` - the
/// cross-plane root-cause chain over money/policy/identity/memory tools,
/// whichever of those are actually wired for this box (see `state.rs`'s
/// `resolve_clients`). Same disabled/not-ready/error contract as
/// `copilot_ask`: `Err` is always operator-readable text, rendered as an
/// assistant note by the frontend rather than a crash.
///
/// `rename_all = "snake_case"` (unlike `copilot_ask`'s single-word
/// `question`): `incident_id` has an underscore, and this app's
/// command-argument convention keeps every key snake_case - a pin that
/// predates the web shell, from back when Tauri's default was camelCase
/// (mirrors `money::commands::money_kill_run`'s identical pin and rationale).
pub async fn copilot_explain(state: &CopilotState, incident_id: String) -> Result<Answer, String> {
    let service = ready_service(&state).await?;
    service
        .explain_incident(&incident_id)
        .await
        .map_err(|e| e.to_string())
}

/// C2's audit link (docs/PHASE6-C2.md "Audit metadata"), called by the
/// frontend right AFTER a proposal card's Approve action has already
/// completed the real signed mutation through its own existing command
/// (`money_kill_run`/`money_set_budget`/`policy_decide_approval`/
/// `identity_rescan`) - never before, and never in place of it. This command
/// performs NO mutation of its own and holds no signer: it only journals the
/// fact that a human approved a specific copilot `ProposedAction`, via
/// [`journal_proposal_approved`].
///
/// Takes `&MoneyState` ALONGSIDE its own `CopilotState`,
/// exactly the way `evidence::commands::evidence_build` takes `MoneyState`
/// alongside `EvidenceState` (see that module's doc comment) - reusing
/// Money's already-paired bus/operator/org rather than this panel growing a
/// second, independent journaling identity. This is a deliberate, narrow
/// coupling: every plane's `console_command` journal already lands in the
/// SAME shared `commands_journal` table and `console.sqlite`
/// (`money::state::BusHandle`/`policy::state::BusHandle` differ only in
/// which physical `.ndjson` file the bus EVENT line is appended to, not in
/// which audit trail the JOURNAL row belongs to - see
/// `genaryx_core::command::record`'s doc comment), so which plane's client
/// happens to supply the `operator`/`org_domain`/`host` triple for THIS
/// entry is a cosmetic choice, not a correctness one.
///
/// Always `Ok`: whether the journal write itself succeeded is reported in
/// the returned [`ProposalApprovedOutcome`], never thrown as an `Err` - a
/// journaling hiccup must never make an already-successful, already-signed
/// mutation look like it failed to the operator (mirrors
/// `evidence_build`'s identical `journaled`/`journal_error` honesty, and
/// `money::commands::journal`'s "report, never panic, never block" rule one
/// level up).
pub async fn copilot_log_proposal_approved(
    kind: String,
    target: String,
    params: Value,
    money_state: &MoneyState,
) -> Result<ProposalApprovedOutcome, ()> {
    let mc = {
        let guard = money_state.inner.lock().await;
        match &*guard {
            MoneyInner::Ready(mc) => Some(mc.clone()),
            MoneyInner::Bootstrapping
            | MoneyInner::NoEnvironment
            | MoneyInner::PairingFailed { .. } => None,
        }
    };
    let Some(mc) = mc else {
        return Ok(ProposalApprovedOutcome {
            journaled: false,
            journal_error: Some("no paired Money device to journal against".to_string()),
        });
    };
    let (journaled, journal_error) = journal_proposal_approved(&mc, &kind, &target, &params);
    Ok(ProposalApprovedOutcome {
        journaled,
        journal_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use genaryx_copilot::{Clients, CopilotConfig};

    #[test]
    fn status_dto_disabled_carries_the_reason_and_no_descriptor_fields() {
        let dto = CopilotStatusDto::disabled("no provider configured");
        assert!(!dto.enabled);
        assert_eq!(
            dto.disabled_reason.as_deref(),
            Some("no provider configured")
        );
        assert!(dto.provider.is_none());
        assert!(dto.model.is_none());
        assert!(dto.endpoint.is_none());
        assert!(dto.local.is_none());
    }

    #[test]
    fn status_dto_from_a_disabled_default_service_is_honestly_disabled() {
        let service =
            CopilotService::from_config_and_clients(&CopilotConfig::default(), Clients::default())
                .expect("default config must construct");
        let dto = CopilotStatusDto::from_service(&service);
        assert!(!dto.enabled);
        assert!(dto.disabled_reason.is_some());
        assert!(dto.provider.is_none());
        assert!(dto.model.is_none());
        assert!(dto.endpoint.is_none());
        assert!(dto.local.is_none());
    }

    // ---- journal_proposal_approved ----
    //
    // Offline throughout (no network, no live Cloud/Wardryx): `CloudClient::new`
    // only builds an HTTP client (see its own doc comment - the same reason
    // `evidence::commands`'s `UNPAIRED_CLOUD_URL` fixture never needs to be
    // dialed), and `journal_proposal_approved` itself only ever touches the
    // local filesystem via `genaryx_core::command::record`.

    /// A `MoneyClient` fixture with no real pairing behind it - mirrors
    /// `identity::commands::tests::fixture_client`'s shape for its own panel.
    /// `bus: None` exercises the "Money resolved but startup seeding never
    /// completed" branch; `Some(..)` exercises a real (scratch-directory)
    /// journal write.
    fn fixture_money_client(bus: Option<crate::money::state::BusHandle>) -> MoneyClient {
        MoneyClient {
            client: Arc::new(
                genaryx_connectors::CloudClient::new("http://127.0.0.1:0", "")
                    .expect("building a never-dialed CloudClient must not fail"),
            ),
            source: crate::money::env::EnvSource::EnvFallback,
            cloud_url: "http://127.0.0.1:0".to_string(),
            org_domain: "acme.example".to_string(),
            operator: "user://acme.example/tester".to_string(),
            host: "test-host".to_string(),
            sig_fpr: "software-signed",
            bus,
        }
    }

    #[test]
    fn journal_proposal_approved_reports_honestly_with_no_bus() {
        let mc = fixture_money_client(None);
        let (journaled, journal_error) =
            journal_proposal_approved(&mc, "kill", "run-1", &json!({}));
        assert!(!journaled);
        assert!(
            journal_error
                .as_deref()
                .is_some_and(|m| m.contains("no live event bus")),
            "must explain why, never a silent false: {journal_error:?}"
        );
    }

    #[test]
    fn journal_proposal_approved_writes_a_conforming_console_command_line() {
        let dir = std::env::temp_dir().join(format!(
            "genaryx-copilot-commands-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let bus = crate::money::state::BusHandle::from_events_dir(&dir);
        let mc = fixture_money_client(Some(bus));

        let (journaled, journal_error) =
            journal_proposal_approved(&mc, "budget", "run-42", &json!({ "usd_cap": 5.0 }));
        assert!(journaled, "journal_error: {journal_error:?}");
        assert!(journal_error.is_none());

        // Money's `BusHandle` targets `tokenfuse.ndjson` - see
        // `money::state::CONSOLE_EVENTS_FILE`.
        let events_path = dir.join("tokenfuse.ndjson");
        let body = std::fs::read_to_string(&events_path).expect("read the appended events file");
        let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 1, "exactly one console_command line appended");

        // Same conformance check `money::state`'s own live_e2e test runs on a
        // real signed-kill journal entry - proves this entry is a fully
        // schema-valid `console_command`, not just "some JSON".
        let conformer = genaryx_core::Conformer::new().expect("embedded schemas must compile");
        let report = conformer.check_line(lines[0]);
        assert!(
            report.valid,
            "appended console_command must conform: {:?}\n  line: {}",
            report.errors, lines[0]
        );

        let value: Value = serde_json::from_str(lines[0]).expect("parse the appended line");
        assert_eq!(
            value.get("type").and_then(Value::as_str),
            Some("console_command")
        );
        assert_eq!(value.get("source").and_then(Value::as_str), Some("console"));
        let data = value.get("data").expect("data object present");
        assert_eq!(
            data.get("action").and_then(Value::as_str),
            Some("console.copilot_proposal_approved")
        );
        assert_eq!(data.get("target").and_then(Value::as_str), Some("run-42"));
        assert_eq!(data.get("decision").and_then(Value::as_str), Some("allow"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
