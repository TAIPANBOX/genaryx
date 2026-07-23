//! One HTTP route for every console command: `POST /api/command/<name>`.
//!
//! The contract is a deliberate mirror of Tauri's `invoke`, because the same
//! React code calls both through `apps/desktop/src/lib/transport.ts`: the
//! request body is the args object the frontend already passes, a 2xx body is
//! the command's Ok value, and a non-2xx body is the command's Err value
//! itself (not wrapped, not restringified), so each plane's existing error
//! normaliser keeps working unchanged.
//!
//! Every arm is generated from the same signatures the Tauri wrappers are
//! generated from, so the two shells cannot drift: a command added to one and
//! forgotten in the other shows up as a compile error, not as a route that
//! quietly 404s in the browser.
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
        "agent_graph" => Ok(ok(genaryx_api::graph::agent_graph(&ctx.bus))),
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
        "money_overview" => Ok(reply(
            genaryx_api::money::commands::money_overview(&ctx.money).await,
        )),
        "money_runs" => Ok(reply(
            genaryx_api::money::commands::money_runs(&ctx.money).await,
        )),
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
