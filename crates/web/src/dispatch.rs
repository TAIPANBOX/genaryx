//! One HTTP route for every console command: `POST /api/command/<name>`.
//!
//! The contract is a deliberate mirror of what this app's Tauri shell once
//! exposed as `invoke` (that shell is gone; the React frontend now reaches
//! this through `apps/web/src/lib/transport.ts` instead): the request body is
//! the args object the frontend already passes, a 2xx body is the command's
//! Ok value, and a non-2xx body is the command's Err value itself (not
//! wrapped, not restringified), so each plane's existing error normaliser
//! keeps working unchanged.
//!
//! Every arm calls straight into the same `genaryx-api` function signature
//! the command is defined with: a command added there and forgotten here
//! shows up as a compile error, not as a route that quietly 404s in the
//! browser.
//!
//! `remote_ssh_tail_start` is the one arm that is not a plain passthrough:
//! its reader thread streams through a generic `TailSink` rather than
//! returning in one batch (see `genaryx_api::remote::commands`'s module
//! doc), so this arm builds this shell's own `crate::ctx::SseTailSink` from
//! `ctx.remote_tail` and hands that in - the lines then reach the browser
//! over `GET /api/events`'s own `remote:tail-line`/`remote:tail-ended` named
//! SSE events (see `main.rs`'s `events` handler), not the `bus` one every
//! other live update rides.

use crate::ctx::Ctx;
// Argument types a command takes by value. They live with their plane, and
// naming them here is what makes a shape change in the plane a compile error
// in this shell rather than a silent 400 in the browser.
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use genaryx_api::evidence::commands::EvidenceBuildRequest;
use genaryx_api::onboard::commands::{
    OnboardGenerateRequest, OnboardStatusRequest, OnboardWritePassportRequest,
};
use genaryx_api::policy::commands::DecisionDto;
use genaryx_api::remote::commands::RemoteEnvironmentRequest;
// `remote_cloud_list`'s options come straight from the connectors crate (a
// direct dependency of this shell), not re-exported through genaryx-api: the
// same "name the arg type so a shape change is a compile error here" rule as
// the genaryx-api requests above.
use genaryx_connectors::CloudListOptions;
use serde_json::Value;
use std::sync::Arc;

/// A command's Ok value.
fn ok<T: serde::Serialize>(v: T) -> Response {
    (
        StatusCode::OK,
        Json(serde_json::to_value(v).unwrap_or(Value::Null)),
    )
        .into_response()
}

/// A command's `Result`: Ok as 200, Err as the error value itself on a 422.
///
/// 422 rather than 500 because a command that refuses is not a server fault:
/// "no environment", "the Cloud rejected this", "break-glass needs a reason"
/// are all answers, and the frontend renders them as such.
fn reply<T: serde::Serialize, E: serde::Serialize>(r: Result<T, E>) -> Response {
    match r {
        Ok(v) => ok(v),
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::to_value(e).unwrap_or(Value::Null)),
        )
            .into_response(),
    }
}

// axum's `Response` is a large value, and clippy would rather it were boxed.
// It is deliberately not: a ready-made `Response` IS the error here (the exact
// status and JSON body the browser should get), and boxing it would add an
// allocation and a deref at every call site to satisfy a lint about a type we
// do not control.
#[allow(clippy::result_large_err)]
fn decode<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, Response> {
    serde_json::from_value(args).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "bad arguments", "detail": e.to_string() })),
        )
            .into_response()
    })
}

/// Every agent id idryx knows for one business unit, falling back to the ids
/// the money plane has seen runs for. Two sources because neither is complete
/// on its own: an agent with no identity record still spends (so it must be
/// blockable), and an agent with no runs yet still exists (so it must not be
/// missed). Ids are deduplicated and returned sorted for a stable audit trail.
async fn agents_in_unit(ctx: &Arc<Ctx>, team: &str) -> Vec<String> {
    let mut found: std::collections::BTreeSet<String> = Default::default();
    if let Ok(identities) = genaryx_api::identity::commands::identity_list_identities(&ctx.identity)
        .await
    {
        for identity in identities {
            if crate::lifecycle::team_of(&identity.id) == Some(team) {
                found.insert(identity.id);
            }
        }
    }
    if let Ok(runs) = genaryx_api::money::commands::money_runs(&ctx.money).await {
        for run in runs {
            if crate::lifecycle::team_of(&run.agent_id) == Some(team) {
                found.insert(run.agent_id);
            }
        }
    }
    found.into_iter().collect()
}

/// Every agent id idryx says `user` owns. Unlike a unit there is no runs-based
/// fallback: `RunDto` carries no owner at all, so the identity plane is the
/// only join. Also refreshes the store's `agent_owners` cache so a stopped
/// user's agents keep projecting after this call, without an identity fetch
/// on every read.
async fn agents_of_user(ctx: &Arc<Ctx>, user: &str) -> Vec<String> {
    let Ok(identities) =
        genaryx_api::identity::commands::identity_list_identities(&ctx.identity).await
    else {
        return Vec::new();
    };
    let mut owned: std::collections::BTreeSet<String> = Default::default();
    {
        let mut store = ctx.lifecycle.write().expect("lifecycle store lock");
        for identity in &identities {
            if !identity.id.starts_with("agent://") || identity.owner.is_empty() {
                continue;
            }
            store
                .agent_owners
                .insert(identity.id.clone(), identity.owner.clone());
            if owner_matches(&identity.owner, user) {
                owned.insert(identity.id.clone());
            }
        }
    }
    owned.into_iter().collect()
}

/// idryx records an owner as either a bare handle (`d.hayes`) or a full
/// `user://org/d.hayes` URI, and the console's own watch dock pins the bare
/// handle. Compare on the last path segment so both forms match.
fn owner_matches(owner: &str, user: &str) -> bool {
    let tail = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
    owner == user || tail(owner) == tail(user)
}

/// Enforce a block/unblock in wardryx, then record it in the console store.
/// The store is only updated after wardryx accepted the change, so a refused
/// enforcement can never leave the console claiming an entity is blocked.
async fn apply_block(
    ctx: &Arc<Ctx>,
    kind: &str,
    key: &str,
    blocked: bool,
    members: Vec<String>,
) -> Result<Value, Value> {
    let outcome = if blocked {
        crate::lifecycle::block(kind, key, &members).await
    } else {
        crate::lifecycle::unblock(kind, key).await
    };
    let affected = outcome.map_err(|e| serde_json::json!({ "error": e }))?;

    {
        let mut store = ctx.lifecycle.write().expect("lifecycle store lock");
        let set = match kind {
            "agent" => &mut store.frozen_agents,
            "unit" => &mut store.stopped_units,
            _ => &mut store.stopped_users,
        };
        if blocked {
            set.insert(key.to_string());
        } else {
            set.remove(key);
        }
    }

    // The durable audit record of this action is the wardryx policy itself:
    // it carries `console-block:<kind>:<key>` as its name and survives a
    // console restart (which is also what `lifecycle::rehydrate` reads back).
    // Mirroring it onto the console's own bus needs the money plane's signing
    // path (`money::state`'s `console_command_line`), which is not reachable
    // from here; that mirror is tracked follow-up work, not a silent gap.
    let _ = affected;
    Ok(Value::Null)
}

/// Route one command by name. `Err` here means the name is unknown; a command
/// that ran and failed comes back as `Ok(response)` carrying its own error.
// axum's `Response` is a large value, and clippy would rather it were boxed.
// It is deliberately not: a ready-made `Response` IS the error here (the exact
// status and JSON body the browser should get), and boxing it would add an
// allocation and a deref at every call site to satisfy a lint about a type we
// do not control.
#[allow(clippy::result_large_err)]
pub async fn dispatch(ctx: &Arc<Ctx>, name: &str, args: Value) -> Result<Response, Response> {
    match name {
        "admission_baseline" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                evalset_path: String,
                model: String,
                agent_id: String,
                api_key: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::admission::commands::admission_baseline(
                    a.evalset_path,
                    a.model,
                    a.agent_id,
                    a.api_key,
                    &ctx.admission,
                )
                .await,
            ))
        }
        "admission_check" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                key_id: String,
                agent_id: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::admission::commands::admission_check(
                    a.key_id,
                    a.agent_id,
                    &ctx.admission,
                )
                .await,
            ))
        }
        "admission_status" => Ok(reply(
            genaryx_api::admission::commands::admission_status(&ctx.admission).await,
        )),
        "agent_events" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                agent_id: String,
                limit: usize,
            }
            let a: A = decode(args)?;
            Ok(ok(genaryx_api::graph::agent_events(
                a.agent_id, a.limit, &ctx.bus,
            )))
        }
        "agent_graph" => {
            // Same block projection `money_runs` gets: a blocked agent's node
            // carries `lifecycle`, so the Graph tab tints it like every other
            // surface rather than showing a stopped fleet as running.
            let mut graph = serde_json::to_value(genaryx_api::graph::agent_graph(&ctx.bus))
                .unwrap_or(Value::Null);
            crate::lifecycle::project_graph(
                &mut graph,
                &ctx.lifecycle.read().expect("lifecycle store lock"),
            );
            Ok(ok(graph))
        }
        "agent_slice" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                agent_id: String,
            }
            let a: A = decode(args)?;
            Ok(ok(genaryx_api::graph::agent_slice(a.agent_id, &ctx.bus)))
        }
        "bus_status" => Ok(ok(genaryx_api::bus::bus_status(&ctx.bus))),
        "recent_events" => {
            #[derive(serde::Deserialize)]
            struct A {
                limit: usize,
            }
            let a: A = decode(args)?;
            Ok(ok(genaryx_api::bus::recent_events(a.limit, &ctx.bus)))
        }
        "copilot_ask" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                question: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::copilot::commands::copilot_ask(&ctx.copilot, a.question).await,
            ))
        }
        "copilot_explain" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                incident_id: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::copilot::commands::copilot_explain(&ctx.copilot, a.incident_id).await,
            ))
        }
        "copilot_log_proposal_approved" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                kind: String,
                target: String,
                params: Value,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::copilot::commands::copilot_log_proposal_approved(
                    a.kind, a.target, a.params, &ctx.money,
                )
                .await,
            ))
        }
        "copilot_status" => Ok(reply(
            genaryx_api::copilot::commands::copilot_status(&ctx.copilot).await,
        )),
        "credentials_keys" => Ok(reply(
            genaryx_api::credentials::commands::credentials_keys(&ctx.credentials).await,
        )),
        "credentials_status" => Ok(reply(
            genaryx_api::credentials::commands::credentials_status(&ctx.credentials).await,
        )),
        "crypto_scan_cbom" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                path: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::crypto::commands::crypto_scan_cbom(a.path, &ctx.crypto).await,
            ))
        }
        "crypto_scan_evidence" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                path: String,
                sign_key: Option<String>,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::crypto::commands::crypto_scan_evidence(
                    a.path,
                    a.sign_key,
                    &ctx.crypto,
                )
                .await,
            ))
        }
        "crypto_scan_ncsc" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                path: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::crypto::commands::crypto_scan_ncsc(a.path, &ctx.crypto).await,
            ))
        }
        "crypto_status" => Ok(reply(
            genaryx_api::crypto::commands::crypto_status(&ctx.crypto).await,
        )),
        "crypto_verify_evidence" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                file: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::crypto::commands::crypto_verify_evidence(a.file, &ctx.crypto).await,
            ))
        }
        "drills_run" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                scenario_dir: String,
                api_key: Option<String>,
                fail_on_skip: bool,
                save_path: Option<String>,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::drills::commands::drills_run(
                    a.scenario_dir,
                    a.api_key,
                    a.fail_on_skip,
                    a.save_path,
                    &ctx.drills,
                )
                .await,
            ))
        }
        "drills_status" => Ok(reply(
            genaryx_api::drills::commands::drills_status(&ctx.drills).await,
        )),
        "evidence_build" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                request: EvidenceBuildRequest,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::evidence::commands::evidence_build(
                    a.request,
                    &ctx.evidence,
                    &ctx.money,
                )
                .await,
            ))
        }
        "evidence_status" => Ok(reply(
            genaryx_api::evidence::commands::evidence_status(&ctx.evidence).await,
        )),
        "identity_list_alerts" => Ok(reply(
            genaryx_api::identity::commands::identity_list_alerts(&ctx.identity).await,
        )),
        "identity_list_identities" => Ok(reply(
            genaryx_api::identity::commands::identity_list_identities(&ctx.identity).await,
        )),
        "identity_list_remediations" => Ok(reply(
            genaryx_api::identity::commands::identity_list_remediations(&ctx.identity).await,
        )),
        "identity_rescan" => Ok(reply(
            genaryx_api::identity::commands::identity_rescan(&ctx.identity).await,
        )),
        "identity_status" => Ok(reply(
            genaryx_api::identity::commands::identity_status(&ctx.identity).await,
        )),
        "memory_forget" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                memory_id: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::memory::commands::memory_forget(a.memory_id, &ctx.memory).await,
            ))
        }
        "memory_recall" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                query: String,
                limit: u32,
                mode: String,
                agent_id: Option<String>,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::memory::commands::memory_recall(
                    a.query,
                    a.limit,
                    a.mode,
                    a.agent_id,
                    &ctx.memory,
                )
                .await,
            ))
        }
        "memory_stats" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                agent_id: Option<String>,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::memory::commands::memory_stats(a.agent_id, &ctx.memory).await,
            ))
        }
        "memory_status" => Ok(reply(
            genaryx_api::memory::commands::memory_status(&ctx.memory).await,
        )),
        "memory_why" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                memory_id: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::memory::commands::memory_why(a.memory_id, &ctx.memory).await,
            ))
        }
        "money_ack_incident" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                id: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::money::commands::money_ack_incident(a.id, &ctx.money).await,
            ))
        }
        "money_incidents" => Ok(reply(
            genaryx_api::money::commands::money_incidents(&ctx.money).await,
        )),
        "money_kill_run" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                run_id: String,
                reason: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::money::commands::money_kill_run(a.run_id, a.reason, &ctx.money).await,
            ))
        }
        // Lifecycle blocks (Yurii, 2026-07-24). Each of the three toggles
        // enforces first (a deny-all wardryx policy per affected agent) and
        // only records the block in `ctx.lifecycle` once wardryx accepted it,
        // so the console never shows a block that did not actually take. The
        // reply is `null` on purpose: this box keeps no per-entity record to
        // return (`agent_record`/`unit_record`/`user_record` are preview-only),
        // and the frontend re-reads `lifecycle_blocks` + `money_runs` for the
        // truth rather than trusting a mutation's own echo.
        "agent_block" => {
            #[derive(serde::Deserialize)]
            struct A {
                agent_id: String,
                blocked: bool,
            }
            let a: A = decode(args)?;
            let done = apply_block(ctx, "agent", &a.agent_id, a.blocked, vec![a.agent_id.clone()])
                .await;
            Ok(reply(done))
        }
        "unit_block" => {
            #[derive(serde::Deserialize)]
            struct A {
                team: String,
                blocked: bool,
            }
            let a: A = decode(args)?;
            let members = agents_in_unit(ctx, &a.team).await;
            let done = apply_block(ctx, "unit", &a.team, a.blocked, members).await;
            Ok(reply(done))
        }
        "user_block" => {
            #[derive(serde::Deserialize)]
            struct A {
                user: String,
                blocked: bool,
            }
            let a: A = decode(args)?;
            let members = agents_of_user(ctx, &a.user).await;
            let done = apply_block(ctx, "user", &a.user, a.blocked, members).await;
            Ok(reply(done))
        }
        "lifecycle_blocks" => {
            let store = ctx.lifecycle.read().expect("lifecycle store lock");
            Ok(ok(store.to_json()))
        }
        "money_overview" => Ok(reply(
            genaryx_api::money::commands::money_overview(&ctx.money).await,
        )),
        "money_runs" => {
            // Project the runs through the operator block store so a frozen
            // agent's runs read not-live and carry a `lifecycle` badge
            // app-wide (Overview spend-by-agent, the watch dock, Agent 360's
            // run list all read this same `money_runs`).
            let projected = genaryx_api::money::commands::money_runs(&ctx.money)
                .await
                .map(|runs| {
                    let mut v = serde_json::to_value(runs).unwrap_or(Value::Null);
                    crate::lifecycle::project_runs(
                        &mut v,
                        &ctx.lifecycle.read().expect("lifecycle store lock"),
                    );
                    v
                });
            Ok(reply(projected))
        }
        "money_savings" => Ok(reply(
            genaryx_api::money::commands::money_savings(&ctx.money).await,
        )),
        "money_set_budget" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                run_id: String,
                budget_usd: f64,
                reason: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::money::commands::money_set_budget(
                    a.run_id,
                    a.budget_usd,
                    a.reason,
                    &ctx.money,
                )
                .await,
            ))
        }
        "money_status" => Ok(reply(
            genaryx_api::money::commands::money_status(&ctx.money).await,
        )),
        "onboard_generate" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                request: OnboardGenerateRequest,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::onboard::commands::onboard_generate(a.request).await,
            ))
        }
        "onboard_status" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                request: OnboardStatusRequest,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::onboard::commands::onboard_status(a.request).await,
            ))
        }
        "onboard_write_passport" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                request: OnboardWritePassportRequest,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::onboard::commands::onboard_write_passport(a.request).await,
            ))
        }
        "pocket_connect" => Ok(reply(genaryx_api::pocket::commands::pocket_connect().await)),
        "pocket_disconnect" => Ok(reply(
            genaryx_api::pocket::commands::pocket_disconnect().await,
        )),
        "pocket_status" => Ok(reply(genaryx_api::pocket::commands::pocket_status().await)),
        "policy_decide_approval" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                id: String,
                decision: DecisionDto,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::policy::commands::policy_decide_approval(
                    a.id,
                    a.decision,
                    &ctx.policy,
                )
                .await,
            ))
        }
        "policy_list_approvals" => Ok(reply(
            genaryx_api::policy::commands::policy_list_approvals(&ctx.policy).await,
        )),
        "policy_list_policies" => Ok(reply(
            genaryx_api::policy::commands::policy_list_policies(&ctx.policy).await,
        )),
        "policy_status" => Ok(reply(
            genaryx_api::policy::commands::policy_status(&ctx.policy).await,
        )),
        "quality_list_baselines" => Ok(reply(
            genaryx_api::quality::commands::quality_list_baselines(&ctx.quality).await,
        )),
        "quality_list_run_summaries" => Ok(reply(
            genaryx_api::quality::commands::quality_list_run_summaries(&ctx.quality).await,
        )),
        "quality_run_scores" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                run_id: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::quality::commands::quality_run_scores(a.run_id, &ctx.quality).await,
            ))
        }
        "quality_status" => Ok(reply(
            genaryx_api::quality::commands::quality_status(&ctx.quality).await,
        )),
        "remote_cloud_list" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                provider: String,
                options: Option<CloudListOptions>,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::remote::commands::remote_cloud_list(a.provider, a.options).await,
            ))
        }
        "remote_hetzner_list" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                token: String,
                label_selector: Option<String>,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::remote::commands::remote_hetzner_list(a.token, a.label_selector).await,
            ))
        }
        "remote_operator_wg_config" => Ok(reply(
            genaryx_api::remote::commands::remote_operator_wg_config().await,
        )),
        "remote_operator_wg_peers" => Ok(reply(
            genaryx_api::remote::commands::remote_operator_wg_peers().await,
        )),
        "remote_operator_wg_revoke" => {
            #[derive(serde::Deserialize)]
            struct A {
                public_key: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::remote::commands::remote_operator_wg_revoke(a.public_key).await,
            ))
        }
        "remote_set_environment" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                request: RemoteEnvironmentRequest,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::remote::commands::remote_set_environment(a.request, &ctx.remote).await,
            ))
        }
        "remote_ssh_check_reachable" => Ok(reply(
            genaryx_api::remote::commands::remote_ssh_check_reachable(&ctx.remote).await,
        )),
        "remote_ssh_read_file" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                path: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::remote::commands::remote_ssh_read_file(a.path, &ctx.remote).await,
            ))
        }
        "remote_ssh_tail_start" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                path: String,
                from_offset: u64,
            }
            let a: A = decode(args)?;
            let sink = crate::ctx::SseTailSink(ctx.remote_tail.clone());
            Ok(reply(
                genaryx_api::remote::commands::remote_ssh_tail_start(
                    a.path,
                    a.from_offset,
                    sink,
                    &ctx.remote,
                )
                .await,
            ))
        }
        "remote_ssh_tail_stop" => Ok(reply(
            genaryx_api::remote::commands::remote_ssh_tail_stop(&ctx.remote).await,
        )),
        "remote_status" => Ok(reply(
            genaryx_api::remote::commands::remote_status(&ctx.remote).await,
        )),
        "remote_wg_connect" => Ok(reply(
            genaryx_api::remote::commands::remote_wg_connect(&ctx.remote).await,
        )),
        "remote_wg_disconnect" => Ok(reply(
            genaryx_api::remote::commands::remote_wg_disconnect(&ctx.remote).await,
        )),
        "routines_history" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                routine: Option<String>,
                limit: Option<u32>,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::routines::commands::routines_history(a.routine, a.limit).await,
            ))
        }
        "routines_status" => Ok(reply(
            genaryx_api::routines::commands::routines_status().await,
        )),
        "run_events" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                run_id: String,
                limit: usize,
            }
            let a: A = decode(args)?;
            Ok(ok(genaryx_api::replay::run_events(
                a.run_id, a.limit, &ctx.bus,
            )))
        }
        other => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "unknown command",
                "command": other,
            })),
        )
            .into_response()),
    }
}
