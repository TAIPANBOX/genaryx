//! Tauri wrappers over [`genaryx_api`], and nothing else.
//!
//! Each function restores the one thing the shared command layer cannot have:
//! the `#[tauri::command]` attribute and the `tauri::State` wrapper that comes
//! with it. Every body is a single call into `genaryx_api`, so both shells run
//! the same logic and a fix cannot land in one shell and miss the other.
//!
//! `remote`'s SSH tail is the one partial exception: its reader thread
//! streams lines through a generic `TailSink` (see
//! `genaryx_api::remote::commands`'s module doc, "Streaming the remote
//! tail") rather than returning them in one batch, so
//! `remote::remote_ssh_tail_start` below does one extra thing no other
//! wrapper needs to - build this shell's own `TauriTailSink` from the
//! `AppHandle` Tauri hands it, then pass that in as the sink. Every other
//! `remote_*` command, and every command in every other module here, is
//! still the same single passthrough call.

pub mod money {
    use genaryx_api::money::commands::*;
    #[allow(unused_imports)]
    use genaryx_api::money::env::*;
    #[allow(unused_imports)]
    use genaryx_api::money::state::*;
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;

    #[tauri::command]
    pub async fn money_status(state: tauri::State<'_, MoneyState>) -> Result<MoneyStatusDto, ()> {
        genaryx_api::money::commands::money_status(&state).await
    }

    #[tauri::command]
    pub async fn money_overview(
        state: tauri::State<'_, MoneyState>,
    ) -> Result<OverviewDto, MoneyError> {
        genaryx_api::money::commands::money_overview(&state).await
    }

    #[tauri::command]
    pub async fn money_runs(
        state: tauri::State<'_, MoneyState>,
    ) -> Result<Vec<RunDto>, MoneyError> {
        genaryx_api::money::commands::money_runs(&state).await
    }

    #[tauri::command]
    pub async fn money_incidents(
        state: tauri::State<'_, MoneyState>,
    ) -> Result<Vec<IncidentDto>, MoneyError> {
        genaryx_api::money::commands::money_incidents(&state).await
    }

    #[tauri::command]
    pub async fn money_savings(
        state: tauri::State<'_, MoneyState>,
    ) -> Result<SavingsDto, MoneyError> {
        genaryx_api::money::commands::money_savings(&state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn money_kill_run(
        run_id: String,
        reason: String,
        state: tauri::State<'_, MoneyState>,
    ) -> Result<MutationOutcome, MoneyError> {
        genaryx_api::money::commands::money_kill_run(run_id, reason, &state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn money_set_budget(
        run_id: String,
        budget_usd: f64,
        reason: String,
        state: tauri::State<'_, MoneyState>,
    ) -> Result<MutationOutcome, MoneyError> {
        genaryx_api::money::commands::money_set_budget(run_id, budget_usd, reason, &state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn money_ack_incident(
        id: String,
        state: tauri::State<'_, MoneyState>,
    ) -> Result<MutationOutcome, MoneyError> {
        genaryx_api::money::commands::money_ack_incident(id, &state).await
    }
}

pub mod policy {
    use genaryx_api::policy::commands::*;
    #[allow(unused_imports)]
    use genaryx_api::policy::env::*;
    #[allow(unused_imports)]
    use genaryx_api::policy::state::*;
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;

    #[tauri::command]
    pub async fn policy_status(
        state: tauri::State<'_, PolicyState>,
    ) -> Result<PolicyStatusDto, ()> {
        genaryx_api::policy::commands::policy_status(&state).await
    }

    #[tauri::command]
    pub async fn policy_list_approvals(
        state: tauri::State<'_, PolicyState>,
    ) -> Result<Vec<ApprovalDto>, PolicyError> {
        genaryx_api::policy::commands::policy_list_approvals(&state).await
    }

    #[tauri::command]
    pub async fn policy_list_policies(
        state: tauri::State<'_, PolicyState>,
    ) -> Result<Vec<PolicyRecordDto>, PolicyError> {
        genaryx_api::policy::commands::policy_list_policies(&state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn policy_decide_approval(
        id: String,
        decision: DecisionDto,
        state: tauri::State<'_, PolicyState>,
    ) -> Result<DecideOutcome, PolicyError> {
        genaryx_api::policy::commands::policy_decide_approval(id, decision, &state).await
    }
}

pub mod identity {
    use genaryx_api::identity::commands::*;
    #[allow(unused_imports)]
    use genaryx_api::identity::env::*;
    #[allow(unused_imports)]
    use genaryx_api::identity::state::*;
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;

    #[tauri::command]
    pub async fn identity_status(
        state: tauri::State<'_, IdentityState>,
    ) -> Result<IdentityStatusDto, ()> {
        genaryx_api::identity::commands::identity_status(&state).await
    }

    #[tauri::command]
    pub async fn identity_list_identities(
        state: tauri::State<'_, IdentityState>,
    ) -> Result<Vec<IdryxIdentity>, IdentityError> {
        genaryx_api::identity::commands::identity_list_identities(&state).await
    }

    #[tauri::command]
    pub async fn identity_list_alerts(
        state: tauri::State<'_, IdentityState>,
    ) -> Result<Vec<IdryxAlert>, IdentityError> {
        genaryx_api::identity::commands::identity_list_alerts(&state).await
    }

    #[tauri::command]
    pub async fn identity_list_remediations(
        state: tauri::State<'_, IdentityState>,
    ) -> Result<Vec<IdryxRecommendation>, IdentityError> {
        genaryx_api::identity::commands::identity_list_remediations(&state).await
    }

    #[tauri::command]
    pub async fn identity_rescan(
        state: tauri::State<'_, IdentityState>,
    ) -> Result<Vec<IdryxAlert>, IdentityError> {
        genaryx_api::identity::commands::identity_rescan(&state).await
    }
}

pub mod credentials {
    use genaryx_api::credentials::commands::*;
    #[allow(unused_imports)]
    use genaryx_api::credentials::env::*;
    #[allow(unused_imports)]
    use genaryx_api::credentials::state::*;
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;

    #[tauri::command]
    pub async fn credentials_status(
        state: tauri::State<'_, CredentialsState>,
    ) -> Result<CredentialsStatusDto, ()> {
        genaryx_api::credentials::commands::credentials_status(&state).await
    }

    #[tauri::command]
    pub async fn credentials_keys(
        state: tauri::State<'_, CredentialsState>,
    ) -> Result<GatewayKeysReport, CredentialsError> {
        genaryx_api::credentials::commands::credentials_keys(&state).await
    }
}

pub mod onboard {
    use genaryx_api::onboard::commands::*;
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;

    #[tauri::command(rename_all = "snake_case")]
    pub async fn onboard_status(
        request: OnboardStatusRequest,
    ) -> Result<OnboardStatusDto, OnboardError> {
        genaryx_api::onboard::commands::onboard_status(request).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn onboard_generate(
        request: OnboardGenerateRequest,
    ) -> Result<OnboardBundleDto, OnboardError> {
        genaryx_api::onboard::commands::onboard_generate(request).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn onboard_write_passport(
        request: OnboardWritePassportRequest,
    ) -> Result<OnboardWriteDto, OnboardError> {
        genaryx_api::onboard::commands::onboard_write_passport(request).await
    }
}

pub mod admission {
    use genaryx_api::admission::commands::*;
    #[allow(unused_imports)]
    use genaryx_api::admission::env::*;
    #[allow(unused_imports)]
    use genaryx_api::admission::state::*;
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;

    #[tauri::command]
    pub async fn admission_status(
        state: tauri::State<'_, AdmissionState>,
    ) -> Result<AdmissionStatusDto, ()> {
        genaryx_api::admission::commands::admission_status(&state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn admission_check(
        key_id: String,
        agent_id: String,
        state: tauri::State<'_, AdmissionState>,
    ) -> Result<AdmissionCheckDto, AdmissionError> {
        genaryx_api::admission::commands::admission_check(key_id, agent_id, &state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn admission_baseline(
        evalset_path: String,
        model: String,
        agent_id: String,
        api_key: String,
        state: tauri::State<'_, AdmissionState>,
    ) -> Result<AdmissionBaselineDto, AdmissionError> {
        genaryx_api::admission::commands::admission_baseline(
            evalset_path,
            model,
            agent_id,
            api_key,
            &state,
        )
        .await
    }
}

pub mod quality {
    use genaryx_api::quality::commands::*;
    #[allow(unused_imports)]
    use genaryx_api::quality::env::*;
    #[allow(unused_imports)]
    use genaryx_api::quality::state::*;
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;

    #[tauri::command]
    pub async fn quality_status(
        state: tauri::State<'_, QualityState>,
    ) -> Result<QualityStatusDto, ()> {
        genaryx_api::quality::commands::quality_status(&state).await
    }

    #[tauri::command]
    pub async fn quality_list_run_summaries(
        state: tauri::State<'_, QualityState>,
    ) -> Result<Vec<VerdryxRunSummary>, QualityError> {
        genaryx_api::quality::commands::quality_list_run_summaries(&state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn quality_run_scores(
        run_id: String,
        state: tauri::State<'_, QualityState>,
    ) -> Result<Vec<VerdryxScore>, QualityError> {
        genaryx_api::quality::commands::quality_run_scores(run_id, &state).await
    }

    #[tauri::command]
    pub async fn quality_list_baselines(
        state: tauri::State<'_, QualityState>,
    ) -> Result<Vec<VerdryxBaseline>, QualityError> {
        genaryx_api::quality::commands::quality_list_baselines(&state).await
    }
}

pub mod crypto {
    use genaryx_api::crypto::commands::*;
    #[allow(unused_imports)]
    use genaryx_api::crypto::env::*;
    #[allow(unused_imports)]
    use genaryx_api::crypto::state::*;
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;

    #[tauri::command]
    pub async fn crypto_status(
        state: tauri::State<'_, CryptoState>,
    ) -> Result<CryptoStatusDto, ()> {
        genaryx_api::crypto::commands::crypto_status(&state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn crypto_scan_ncsc(
        path: String,
        state: tauri::State<'_, CryptoState>,
    ) -> Result<NcscReport, CryptoError> {
        genaryx_api::crypto::commands::crypto_scan_ncsc(path, &state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn crypto_scan_cbom(
        path: String,
        state: tauri::State<'_, CryptoState>,
    ) -> Result<serde_json::Value, CryptoError> {
        genaryx_api::crypto::commands::crypto_scan_cbom(path, &state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn crypto_scan_evidence(
        path: String,
        sign_key: Option<String>,
        state: tauri::State<'_, CryptoState>,
    ) -> Result<EvidenceReport, CryptoError> {
        genaryx_api::crypto::commands::crypto_scan_evidence(path, sign_key, &state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn crypto_verify_evidence(
        file: String,
        state: tauri::State<'_, CryptoState>,
    ) -> Result<VerifyOutcome, CryptoError> {
        genaryx_api::crypto::commands::crypto_verify_evidence(file, &state).await
    }
}

pub mod memory {
    use genaryx_api::memory::commands::*;
    #[allow(unused_imports)]
    use genaryx_api::memory::env::*;
    #[allow(unused_imports)]
    use genaryx_api::memory::state::*;
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;

    #[tauri::command]
    pub async fn memory_status(
        state: tauri::State<'_, MemoryState>,
    ) -> Result<MemoryStatusDto, ()> {
        genaryx_api::memory::commands::memory_status(&state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn memory_stats(
        agent_id: Option<String>,
        state: tauri::State<'_, MemoryState>,
    ) -> Result<EngramStats, MemoryError> {
        genaryx_api::memory::commands::memory_stats(agent_id, &state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn memory_recall(
        query: String,
        limit: u32,
        mode: String,
        agent_id: Option<String>,
        state: tauri::State<'_, MemoryState>,
    ) -> Result<Vec<EngramMemory>, MemoryError> {
        genaryx_api::memory::commands::memory_recall(query, limit, mode, agent_id, &state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn memory_why(
        memory_id: String,
        state: tauri::State<'_, MemoryState>,
    ) -> Result<EngramProvenance, MemoryError> {
        genaryx_api::memory::commands::memory_why(memory_id, &state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn memory_forget(
        memory_id: String,
        state: tauri::State<'_, MemoryState>,
    ) -> Result<EngramForgetResult, MemoryError> {
        genaryx_api::memory::commands::memory_forget(memory_id, &state).await
    }
}

pub mod drills {
    use genaryx_api::drills::commands::*;
    #[allow(unused_imports)]
    use genaryx_api::drills::env::*;
    #[allow(unused_imports)]
    use genaryx_api::drills::state::*;
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;

    #[tauri::command]
    pub async fn drills_status(
        state: tauri::State<'_, DrillsState>,
    ) -> Result<DrillsStatusDto, ()> {
        genaryx_api::drills::commands::drills_status(&state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn drills_run(
        scenario_dir: String,
        api_key: Option<String>,
        fail_on_skip: bool,
        save_path: Option<String>,
        state: tauri::State<'_, DrillsState>,
    ) -> Result<MockryxReport, DrillsError> {
        genaryx_api::drills::commands::drills_run(
            scenario_dir,
            api_key,
            fail_on_skip,
            save_path,
            &state,
        )
        .await
    }
}

pub mod evidence {
    use genaryx_api::evidence::commands::*;
    #[allow(unused_imports)]
    use genaryx_api::evidence::env::*;
    #[allow(unused_imports)]
    use genaryx_api::evidence::state::*;
    use genaryx_api::money::MoneyState;
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;

    #[tauri::command]
    pub async fn evidence_status(
        state: tauri::State<'_, EvidenceState>,
    ) -> Result<EvidenceStatusDto, ()> {
        genaryx_api::evidence::commands::evidence_status(&state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn evidence_build(
        request: EvidenceBuildRequest,
        evidence_state: tauri::State<'_, EvidenceState>,
        money_state: tauri::State<'_, MoneyState>,
    ) -> Result<EvidenceBuildDto, EvidenceError> {
        genaryx_api::evidence::commands::evidence_build(request, &evidence_state, &money_state)
            .await
    }
}

pub mod remote {
    use genaryx_api::remote::commands::*;
    #[allow(unused_imports)]
    use genaryx_api::remote::env::*;
    #[allow(unused_imports)]
    use genaryx_api::remote::state::*;
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;
    use tauri::Emitter;

    /// Tauri event name for one streamed remote-tail line - the frontend's
    /// `RemoteSshOps.tsx` `TAIL_LINE_EVENT` constant matches this exactly.
    /// Lives here, not in `genaryx_api`, because it names a Tauri event,
    /// which is a desktop-shell detail the shared command layer has no
    /// opinion on (see `genaryx_api::remote::commands`'s module doc).
    pub const TAIL_LINE_EVENT: &str = "remote:tail-line";
    /// Emitted once, when the tail reader loop ends - see [`TAIL_LINE_EVENT`].
    pub const TAIL_ENDED_EVENT: &str = "remote:tail-ended";

    /// This shell's [`TailSink`]: forwards each remote-tail line/ended event
    /// as a Tauri window event - mirrors `live::TauriSink`'s identical role
    /// for the live bus feed. A failed emit is logged and dropped rather
    /// than propagated, the same reasoning `TauriSink::emit`'s own doc
    /// comment gives.
    struct TauriTailSink(tauri::AppHandle);

    impl TailSink for TauriTailSink {
        fn line(&self, line: RemoteTailLine) {
            if let Err(e) = self.0.emit(TAIL_LINE_EVENT, line) {
                eprintln!("genaryx: failed to emit remote tail line: {e}");
            }
        }

        fn ended(&self, ended: RemoteTailEnded) {
            if let Err(e) = self.0.emit(TAIL_ENDED_EVENT, ended) {
                eprintln!("genaryx: failed to emit remote tail ended: {e}");
            }
        }
    }

    #[tauri::command]
    pub async fn remote_status(
        state: tauri::State<'_, RemoteState>,
    ) -> Result<RemoteStatusDto, ()> {
        genaryx_api::remote::commands::remote_status(&state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn remote_set_environment(
        request: RemoteEnvironmentRequest,
        state: tauri::State<'_, RemoteState>,
    ) -> Result<RemoteStatusDto, RemoteError> {
        genaryx_api::remote::commands::remote_set_environment(request, &state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn remote_hetzner_list(
        token: String,
        label_selector: Option<String>,
    ) -> Result<Vec<HetznerServer>, RemoteError> {
        genaryx_api::remote::commands::remote_hetzner_list(token, label_selector).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn remote_cloud_list(
        provider: String,
        options: Option<CloudListOptions>,
    ) -> Result<Vec<CloudServer>, RemoteError> {
        genaryx_api::remote::commands::remote_cloud_list(provider, options).await
    }

    #[tauri::command]
    pub async fn remote_wg_connect(
        state: tauri::State<'_, RemoteState>,
    ) -> Result<RemoteStatusDto, RemoteError> {
        genaryx_api::remote::commands::remote_wg_connect(&state).await
    }

    #[tauri::command]
    pub async fn remote_wg_disconnect(
        state: tauri::State<'_, RemoteState>,
    ) -> Result<RemoteStatusDto, RemoteError> {
        genaryx_api::remote::commands::remote_wg_disconnect(&state).await
    }

    #[tauri::command]
    pub async fn remote_ssh_check_reachable(
        state: tauri::State<'_, RemoteState>,
    ) -> Result<(), RemoteError> {
        genaryx_api::remote::commands::remote_ssh_check_reachable(&state).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn remote_ssh_read_file(
        path: String,
        state: tauri::State<'_, RemoteState>,
    ) -> Result<RemoteFileDto, RemoteError> {
        genaryx_api::remote::commands::remote_ssh_read_file(path, &state).await
    }

    /// Unlike every other wrapper in this file, this one does not just await
    /// the shared function: it builds this shell's own [`TauriTailSink`] from
    /// the `AppHandle` Tauri injects (a plain constructor parameter no
    /// `#[tauri::command]` argument list needs to name specially) and hands
    /// it to `genaryx_api` as the generic `S: TailSink` the tail's reader
    /// thread streams through - see `genaryx_api::remote::commands`'s module
    /// doc ("Streaming the remote tail").
    #[tauri::command(rename_all = "snake_case")]
    pub async fn remote_ssh_tail_start(
        path: String,
        from_offset: u64,
        app: tauri::AppHandle,
        state: tauri::State<'_, RemoteState>,
    ) -> Result<RemoteStatusDto, RemoteError> {
        genaryx_api::remote::commands::remote_ssh_tail_start(
            path,
            from_offset,
            TauriTailSink(app),
            &state,
        )
        .await
    }

    #[tauri::command]
    pub async fn remote_ssh_tail_stop(
        state: tauri::State<'_, RemoteState>,
    ) -> Result<RemoteStatusDto, RemoteError> {
        genaryx_api::remote::commands::remote_ssh_tail_stop(&state).await
    }
}

pub mod pocket {
    use genaryx_api::pocket::commands::*;
    #[allow(unused_imports)]
    use genaryx_api::pocket::env::*;
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;

    #[tauri::command]
    pub async fn pocket_status() -> Result<PocketStatusDto, ()> {
        genaryx_api::pocket::commands::pocket_status().await
    }

    #[tauri::command]
    pub async fn pocket_connect() -> Result<PocketQrDto, PocketError> {
        genaryx_api::pocket::commands::pocket_connect().await
    }

    #[tauri::command]
    pub async fn pocket_disconnect() -> Result<PocketStatusDto, PocketError> {
        genaryx_api::pocket::commands::pocket_disconnect().await
    }
}

pub mod graph {
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;
    use genaryx_core::{AgentSlice, LayoutView};

    #[tauri::command]
    pub fn agent_graph(state: tauri::State<'_, AppState>) -> LayoutView {
        genaryx_api::graph::agent_graph(&state)
    }

    #[tauri::command(rename_all = "snake_case")]
    pub fn agent_slice(agent_id: String, state: tauri::State<'_, AppState>) -> AgentSlice {
        genaryx_api::graph::agent_slice(agent_id, &state)
    }

    #[tauri::command(rename_all = "snake_case")]
    pub fn agent_events(
        agent_id: String,
        limit: usize,
        state: tauri::State<'_, AppState>,
    ) -> Vec<UiEvent> {
        genaryx_api::graph::agent_events(agent_id, limit, &state)
    }
}

pub mod replay {
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;

    #[tauri::command(rename_all = "snake_case")]
    pub fn run_events(
        run_id: String,
        limit: usize,
        state: tauri::State<'_, AppState>,
    ) -> Vec<UiEvent> {
        genaryx_api::replay::run_events(run_id, limit, &state)
    }
}

pub mod copilot {
    use genaryx_api::copilot::commands::*;
    #[allow(unused_imports)]
    use genaryx_api::copilot::state::*;
    use genaryx_api::money::MoneyState;
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;
    use genaryx_copilot::Answer;
    use serde_json::Value;

    #[tauri::command]
    pub async fn copilot_status(
        state: tauri::State<'_, CopilotState>,
    ) -> Result<CopilotStatusDto, ()> {
        genaryx_api::copilot::commands::copilot_status(&state).await
    }

    #[tauri::command]
    pub async fn copilot_ask(
        state: tauri::State<'_, CopilotState>,
        question: String,
    ) -> Result<Answer, String> {
        genaryx_api::copilot::commands::copilot_ask(&state, question).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn copilot_explain(
        state: tauri::State<'_, CopilotState>,
        incident_id: String,
    ) -> Result<Answer, String> {
        genaryx_api::copilot::commands::copilot_explain(&state, incident_id).await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn copilot_log_proposal_approved(
        kind: String,
        target: String,
        params: Value,
        money_state: tauri::State<'_, MoneyState>,
    ) -> Result<ProposalApprovedOutcome, ()> {
        genaryx_api::copilot::commands::copilot_log_proposal_approved(
            kind,
            target,
            params,
            &money_state,
        )
        .await
    }
}

pub mod routines {
    use genaryx_api::routines::commands::*;
    #[allow(unused_imports)]
    use genaryx_api::routines::env::*;
    #[allow(unused_imports)]
    use genaryx_api::{bus::AppState, bus::BusMode, events::UiEvent};
    #[allow(unused_imports)]
    use genaryx_connectors::*;

    /// Read-only, like Onboard/Pocket above: every call re-resolves
    /// `$STACK_UP_HOME/routines` and re-reads its files fresh, so there is
    /// nothing for `setup` to `app.manage` here (see `genaryx_api::routines`'s
    /// module doc).
    #[tauri::command]
    pub async fn routines_status() -> Result<RoutinesStatusDto, ()> {
        genaryx_api::routines::commands::routines_status().await
    }

    #[tauri::command(rename_all = "snake_case")]
    pub async fn routines_history(
        routine: Option<String>,
        limit: Option<u32>,
    ) -> Result<RoutinesHistoryDto, ()> {
        genaryx_api::routines::commands::routines_history(routine, limit).await
    }
}
