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
use axum::http::header::CACHE_CONTROL;
use axum::http::{HeaderValue, StatusCode};
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

/// Where a peer-lifecycle action gets journaled, built from the same live
/// events directory the Bus Explorer tails. `None` when live-wire startup
/// failed, which the command reports rather than silently skipping the record.
fn wg_journal(ctx: &Arc<Ctx>) -> Option<genaryx_api::money::state::BusHandle> {
    match (
        ctx.bus.events_dir.as_ref(),
        ctx.bus.source_events_dir.as_ref(),
    ) {
        (Some(store), Some(source)) => Some(genaryx_api::money::state::BusHandle::from_dirs(
            store, source,
        )),
        _ => None,
    }
}

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

/// The same response, marked so that nothing between here and the browser
/// keeps a copy of it.
fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
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
    if let Ok(identities) =
        genaryx_api::identity::commands::identity_list_identities(&ctx.identity).await
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

    journal_block(ctx, kind, key, blocked, affected, &members);

    // The reply stays `null` deliberately: this box keeps no per-entity record
    // to echo, and the frontend re-reads `lifecycle_blocks` + `money_runs` for
    // the truth rather than trusting a mutation's own body.
    Ok(Value::Null)
}

/// Put a completed block or unblock on the console's own bus.
///
/// The wardryx policy is the durable record of the block (it carries
/// `console-block:<kind>:<key>` as its name and is what `lifecycle::rehydrate`
/// reads back), but a policy sitting in wardryx is not an audit trail of WHO
/// did it and WHEN. This is the most far-reaching action the console has:
/// stopping a unit writes a real deny-all policy for every agent in it, and
/// until now it was the only privileged mutation that reached no bus at all,
/// so an evidence pack could show a single killed run and not a halted fleet.
///
/// The old note here said the mirror needed a signing path "not reachable
/// from here". It is reachable: `wg_journal` builds a `BusHandle` from this
/// same `ctx` two functions up, and the WireGuard peer commands have been
/// journaling through it for some time. What was missing was the signing
/// METADATA, and `crate::journal` supplies it the same way they do, from the
/// request's own ceremony.
fn journal_block(
    ctx: &Arc<Ctx>,
    kind: &str,
    key: &str,
    blocked: bool,
    affected: usize,
    members: &[String],
) {
    // `members` as well as the count, because "3 policies written" does not
    // say WHICH agents an operator halted, and a unit stop can halt a dozen.
    // A reader of the chain (or of the Statistics tab, which credits the stop
    // to each agent named here) otherwise has to reconstruct the unit's
    // membership as it was at that moment, which nothing records.
    let _ = genaryx_api::journal::record_console_action(
        wg_journal(ctx).as_ref(),
        block_action(kind, blocked),
        key,
        serde_json::json!({ "policies": affected, "members": members }),
        format!("blocked:{blocked}"),
    );
}

/// The `data.action` name for one lifecycle toggle.
///
/// Two directions, two names, matching `console.issue_wg_peer` /
/// `console.revoke_wg_peer`: an auditor filtering for who restarted a stopped
/// unit should not have to read `verify_result` to find out. `kind` comes
/// from this module's own three call sites, and anything other than `agent`
/// or `unit` is a user block, mirroring `apply_block`'s own dispatch on the
/// same value so the two can never disagree about what a `kind` means.
fn block_action(kind: &str, blocked: bool) -> &'static str {
    match (kind, blocked) {
        ("agent", true) => "console.block_agent",
        ("agent", false) => "console.unblock_agent",
        ("unit", true) => "console.block_unit",
        ("unit", false) => "console.unblock_unit",
        (_, true) => "console.block_user",
        (_, false) => "console.unblock_user",
    }
}

/// Route one command by name. `Err` here means the name is unknown; a command
/// that ran and failed comes back as `Ok(response)` carrying its own error.
// axum's `Response` is a large value, and clippy would rather it were boxed.
// It is deliberately not: a ready-made `Response` IS the error here (the exact
// status and JSON body the browser should get), and boxing it would add an
// allocation and a deref at every call site to satisfy a lint about a type we
// do not control.
#[allow(clippy::result_large_err)]
/// The window the Agent card's profile asks for when the caller does not say.
///
/// A quarter. Long enough that a monthly rhythm has been seen three times, and
/// the same span the retention default keeps (`GENARYX_HISTORY_DAYS`), so the
/// card never asks for history the box has already dropped.
fn default_profile_window() -> i64 {
    90
}

/// A month, matching the Cloud's own default and the period an operator asks
/// for first.
fn default_spend_days() -> i64 {
    30
}

/// How many stop-shaped events to read before folding one agent's list out of
/// them. Generous: these are a small slice of the bus, and a card that showed
/// "the last few" of somebody's stops would be the wrong answer to "how many
/// times was this stopped".
fn default_stops_scan() -> usize {
    20_000
}

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
        // What the envelope REFUSED. A read, and classified as one in roles.rs.
        // Takes no arguments: a refusal is bus-wide and has no window, because
        // a line refused an hour ago is still not on the bus now.
        "bus_quarantine" => Ok(ok(genaryx_api::bus::quarantine(&ctx.bus))),
        // The Keys panel: Keyryx custody's inventory and the witness's latest
        // tree head, over their public GET routes (invariant 53). A read with
        // no control: rotation and revocation are custody's own CLI.
        "keys_custody" => {
            let (cfg, refused) = genaryx_api::keys::KeysConfig::from_env();
            Ok(ok(genaryx_api::keys::keys_custody(&cfg, &refused).await))
        }
        // The Record panel: what Trailryx sealed and whether the current root
        // is signed, from the node's own files (invariant 54). A read.
        "record_status" => Ok(ok(genaryx_api::record::record_status(
            &genaryx_api::record::RecordConfig::from_env(),
        ))),
        // Verify the record now: pack it and run trailryx-verify, under the
        // proof slots. A read, and classified as one: it changes nothing.
        "record_verify" => Ok(ok(genaryx_api::record::record_verify(
            genaryx_api::record::RecordConfig::from_env(),
        )
        .await)),
        // The Notifications panel: what heraldyx recorded sending, read from
        // its dispatch journal where GENARYX_HERALDYX_SENT names it (invariant
        // 51). A read, and classified as one in roles.rs.
        "notifications_sent" => Ok(ok(genaryx_api::notifications::notifications_sent(
            genaryx_api::notifications::sent_path_from_env().as_deref(),
        ))),
        // The egress panel. A read, and classified as one in roles.rs.
        "egress_recent" => {
            #[derive(serde::Deserialize)]
            struct A {
                limit: usize,
            }
            let a: A = decode(args)?;
            Ok(ok(genaryx_api::egress::egress_recent(a.limit, &ctx.bus)))
        }
        // Per-agent event counts for the Statistics view. A read, and
        // classified as one in roles.rs. `scan` bounds the bus lines read, not
        // the agents returned: see `genaryx_api::stats`.
        "stats_counts" => {
            #[derive(serde::Deserialize)]
            struct A {
                scan: usize,
                /// Days of history to count over. `0` (and an older frontend
                /// that omits it) means the recent slice of any age, which is
                /// what this command did before durable history existed.
                #[serde(default)]
                window_days: u32,
            }
            let a: A = decode(args)?;
            Ok(ok(genaryx_api::stats::stats_counts(
                a.scan,
                a.window_days,
                &ctx.bus,
            )))
        }
        // ONE agent's rhythm, and how its latest complete day sits in it. A
        // read, and classified as one in roles.rs.
        //
        // `window_days` is the caller's, defaulted rather than required, so an
        // older frontend gets the quarter the card was designed around instead
        // of an error.
        "agent_profile" => {
            #[derive(serde::Deserialize)]
            struct A {
                agent_id: String,
                #[serde(default = "default_profile_window")]
                window_days: i64,
            }
            let a: A = decode(args)?;
            Ok(ok(genaryx_api::stats::profile::agent_profile(
                &a.agent_id,
                a.window_days,
                chrono::Utc::now().timestamp_millis(),
                &ctx.bus,
            )))
        }
        // When, why and by whom this agent was stopped. A read, and classified
        // as one in roles.rs.
        //
        // A backend fold rather than a filter in the view: the classification
        // is pinned in `genaryx_api::stats` and a second copy in TypeScript
        // would drift silently, and a FREEZE is journaled against the console
        // rather than the agent, so a view reading only `agent_events` cannot
        // see the one stop a person definitely caused.
        "agent_stops" => {
            #[derive(serde::Deserialize)]
            struct A {
                agent_id: String,
                #[serde(default = "default_stops_scan")]
                limit: usize,
            }
            let a: A = decode(args)?;
            Ok(ok(genaryx_api::stats::agent_stops(
                &a.agent_id,
                a.limit,
                &ctx.bus,
            )))
        }
        "recent_events" => {
            #[derive(serde::Deserialize)]
            struct A {
                limit: usize,
            }
            let a: A = decode(args)?;
            Ok(ok(genaryx_api::bus::recent_events(a.limit, &ctx.bus)))
        }
        // The case file for one incident: the console's own store and every
        // plane that has something about it, joined with no model. A read, and
        // classified as one in roles.rs. A plane that does not answer is a line
        // in the case, so the reply is a case even with every plane down; only
        // an incident with no event row is refused.
        "case_file" => {
            #[derive(serde::Deserialize)]
            struct A {
                incident_id: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::case::commands::case_file(
                    a.incident_id,
                    genaryx_api::case::commands::CasePlanes {
                        bus: &ctx.bus,
                        money: &ctx.money,
                        policy: &ctx.policy,
                        identity: &ctx.identity,
                        memory: &ctx.memory,
                    },
                )
                .await,
            ))
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
        // Felyx reads the case file of one incident: the console's join of its
        // store and the planes, with no tool loop. A read, and classified as
        // one in roles.rs. A refusal (no event row, an unknown row) is the
        // Err string, like every copilot command.
        "copilot_explain_case" => {
            #[derive(serde::Deserialize)]
            struct A {
                incident_id: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::copilot::commands::copilot_explain_case(
                    &ctx.copilot,
                    genaryx_api::case::commands::CasePlanes {
                        bus: &ctx.bus,
                        money: &ctx.money,
                        policy: &ctx.policy,
                        identity: &ctx.identity,
                        memory: &ctx.memory,
                    },
                    a.incident_id,
                )
                .await,
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
        "credentials_gateway_retained_runs" => Ok(reply(
            genaryx_api::credentials::commands::credentials_gateway_retained_runs(&ctx.credentials)
                .await,
        )),
        // The gateway's posture: settings, keys and identity map, price
        // book. A read, through the gateway admin key the plane holds.
        "credentials_posture" => Ok(reply(
            genaryx_api::credentials::posture::credentials_posture(&ctx.credentials).await,
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
        // Cuts an agent's or a user's delegated authority at vouchryx. Admin
        // only, sensitive-ceremony-gated (see `main.rs`'s
        // the ceremony column and the admin rows of `roles.rs`), the same
        // class of act as `money_kill_run` and `remote_operator_wg_revoke`.
        "delegation_revoke" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                #[serde(default)]
                subject: Option<String>,
                #[serde(default)]
                jti: Option<String>,
                reason: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::delegation::commands::delegation_revoke(
                    a.subject,
                    a.jti,
                    a.reason,
                    &ctx.cfg.delegation,
                    wg_journal(ctx).as_ref(),
                )
                .await,
            ))
        }
        // Stops one agent everywhere: a ban at vouchryx, its open runs killed
        // on the Cloud, every step in the console's own chain. Admin only and
        // ceremony-gated (`roles.rs`), the same class of act as
        // `delegation_revoke` and `money_kill_run`, which it combines.
        "agent_stop" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                subject: String,
                reason: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::delegation::stop::agent_stop(
                    a.subject,
                    a.reason,
                    &ctx.cfg.delegation,
                    &ctx.money,
                    wg_journal(ctx).as_ref(),
                )
                .await,
            ))
        }
        // Lifts the ban `agent_stop` set. The same gate as the stop.
        "agent_lift" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                subject: String,
                reason: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::delegation::stop::agent_lift(
                    a.subject,
                    a.reason,
                    &ctx.cfg.delegation,
                    wg_journal(ctx).as_ref(),
                )
                .await,
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
        // D8: the gateway's retained reservations, through the same gateway
        // client and admin key the Credentials plane reads GET /v1/runs with.
        // A read, and classified as one in roles.rs.
        "money_list_retained" => Ok(reply(
            genaryx_api::money::retained::money_list_retained(&ctx.credentials).await,
        )),
        // D8: close one retained reservation with evidence. Admin only and
        // ceremony-gated (the ceremony column of `roles.rs`): the console
        // mints the gateway's closure token only once that gate has passed,
        // and journals every attempt into the console's own chain.
        "money_close_retained" => {
            let a: genaryx_api::money::retained::CloseRetainedArgs = decode(args)?;
            Ok(reply(
                genaryx_api::money::retained::money_close_retained(
                    a,
                    &ctx.cfg.closure,
                    &ctx.credentials,
                    wg_journal(ctx).as_ref(),
                )
                .await,
            ))
        }
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
        // Lifecycle blocks (@decided 2026-07-24). Each of the three toggles
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
            let done = apply_block(
                ctx,
                "agent",
                &a.agent_id,
                a.blocked,
                vec![a.agent_id.clone()],
            )
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
        // Spend over a PERIOD, which is the only shape that can answer one.
        // A read, and classified as one in roles.rs.
        //
        // `null` when this Cloud has no `/v1/spend` (older than tokenfuse
        // #199). The frontend must keep that distinct from zero: a box that
        // cannot answer the question is not a box that answered nothing.
        "money_spend_window" => {
            #[derive(serde::Deserialize)]
            struct A {
                #[serde(default = "default_spend_days")]
                days: i64,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::money::commands::money_spend_window(&ctx.money, a.days).await,
            ))
        }
        // The money plane's own per-person rollup. A read, and classified as
        // one in roles.rs. Deliberately a SEPARATE command from the console's
        // idryx-based owner grouping: they answer different questions and the
        // frontend keeps them apart (see `genaryx_api::money::money_owners`).
        "money_owners" => Ok(reply(
            genaryx_api::money::commands::money_owners(&ctx.money).await,
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
        // Unit month caps: a read, and the signed cap change (admin, in
        // the ceremony column, journaled like money_set_budget).
        // Which gateways the Cloud still hears from: a read.
        "money_gateways" => Ok(reply(
            genaryx_api::money::gateways::money_gateways(&ctx.money).await,
        )),
        "money_units" => Ok(reply(
            genaryx_api::money::units::money_units(&ctx.money).await,
        )),
        "money_set_unit_budget" => {
            #[derive(serde::Deserialize)]
            struct A {
                unit: String,
                budget_usd: f64,
                reason: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::money::units::money_set_unit_budget(
                    a.unit,
                    a.budget_usd,
                    a.reason,
                    &ctx.money,
                )
                .await,
            ))
        }
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
        "policy_decide_approval" => {
            #[derive(serde::Deserialize)]
            #[allow(non_snake_case)]
            struct A {
                id: String,
                decision: DecisionDto,
                /// The digest of the preview the browser showed, inside the
                /// arguments the passkey assertion confirmed (invariant 48).
                #[serde(default)]
                seen_pvd: Option<String>,
            }
            let a: A = decode(args)?;
            // A grant's answer is the one copy of a live credential, so it
            // tells every cache between here and the browser to keep nothing
            // (CLAUDE.md invariant 46).
            Ok(no_store(reply(
                genaryx_api::policy::commands::policy_decide_approval(
                    a.id,
                    a.decision,
                    a.seen_pvd,
                    &ctx.policy,
                    &ctx.approver,
                    &ctx.seen_previews,
                )
                .await,
            )))
        }
        "policy_approval_preview" => {
            #[derive(serde::Deserialize)]
            struct A {
                id: String,
            }
            let a: A = decode(args)?;
            // The preview is shown and dropped: nothing between here and the
            // browser keeps it (invariant 48).
            Ok(no_store(reply(
                genaryx_api::policy::preview::policy_approval_preview(
                    a.id,
                    &ctx.policy,
                    &ctx.preview_key,
                    &ctx.seen_previews,
                )
                .await,
            )))
        }
        "policy_grant_tool" => {
            #[derive(serde::Deserialize)]
            struct A {
                id: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::policy::commands::policy_grant_tool(a.id, &ctx.policy).await,
            ))
        }
        "policy_list_approvals" => Ok(reply(
            genaryx_api::policy::commands::policy_list_approvals(&ctx.policy).await,
        )),
        "policy_list_policies" => Ok(reply(
            genaryx_api::policy::commands::policy_list_policies(&ctx.policy).await,
        )),
        "policy_enforcement_status" => Ok(reply(
            genaryx_api::policy::commands::policy_enforcement_status(&ctx.policy).await,
        )),
        // Seal a case with the proofs asked for again here and the operator's
        // decision: one canonical document, its sha256 as the target of a
        // `console.case_close` line in the console's chain, and a file named by
        // it. An approver's act, because it puts an operator's name to a decision;
        // it changes nothing in any plane.
        "case_seal" => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct A {
                incident_id: String,
                #[serde(default)]
                proofs: Vec<genaryx_api::seal::ProofSpec>,
                decision: genaryx_api::seal::Decision,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::seal::commands::case_seal(
                    genaryx_api::seal::commands::SealRequest {
                        incident_id: a.incident_id,
                        proofs: a.proofs,
                        decision: a.decision,
                    },
                    genaryx_api::case::commands::CasePlanes {
                        bus: &ctx.bus,
                        money: &ctx.money,
                        policy: &ctx.policy,
                        identity: &ctx.identity,
                        memory: &ctx.memory,
                    },
                    wg_journal(ctx).as_ref(),
                )
                .await,
            ))
        }
        // A typed remedy checked against an incident: whether it fits, its proof
        // where one exists, the Mockryx scenario that rehearses the failure, and
        // the command a person would apply it with. A read, and classified as
        // one in roles.rs: it applies nothing and starts no drill.
        "remedy_check" => {
            #[derive(serde::Deserialize)]
            struct A {
                incident_id: String,
                remedy: genaryx_api::remedy::Remedy,
            }
            let a: A = decode(args)?;
            let scenario_dir = match genaryx_api::drills::commands::drills_status(&ctx.drills).await
            {
                Ok(genaryx_api::drills::commands::DrillsStatusDto::Ready {
                    scenario_dir, ..
                }) => scenario_dir.map(std::path::PathBuf::from),
                _ => None,
            };
            Ok(reply(
                genaryx_api::remedy::commands::remedy_check(
                    genaryx_api::remedy::commands::RemedyRequest {
                        incident_id: a.incident_id,
                        remedy: a.remedy,
                    },
                    &ctx.bus,
                    scenario_dir,
                )
                .await,
            ))
        }
        // A sealed pack handed back, only if it is still the pack its digest names.
        "case_pack" => {
            #[derive(serde::Deserialize)]
            struct A {
                digest: String,
            }
            let a: A = decode(args)?;
            Ok(reply(genaryx_api::seal::commands::case_pack(
                &a.digest,
                ctx.bus.events_dir.as_deref(),
            )))
        }
        // What a candidate budget or step limit would have done to the calls the
        // gateway recorded, from `tokenfuse backtest`, in both halves. A read,
        // and classified as one in roles.rs: it applies nothing.
        "budget_proof" => {
            // An amount arrives as the text the operator typed or as a JSON
            // number; either is read as dollars with at most six decimals.
            #[derive(serde::Deserialize)]
            #[serde(untagged)]
            enum Amount {
                Text(String),
                Number(serde_json::Number),
            }
            impl Amount {
                fn text(self) -> String {
                    match self {
                        Amount::Text(t) => t,
                        Amount::Number(n) => n.to_string(),
                    }
                }
            }
            #[derive(serde::Deserialize)]
            struct A {
                incident_id: String,
                per_run_usd: Option<Amount>,
                per_step_usd: Option<Amount>,
                max_steps: Option<u32>,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::proof::budget_commands::budget_proof(
                    genaryx_api::proof::budget_commands::BudgetRequest {
                        incident_id: a.incident_id,
                        per_run_usd: a.per_run_usd.map(Amount::text),
                        per_step_usd: a.per_step_usd.map(Amount::text),
                        max_steps: a.max_steps,
                    },
                    &ctx.bus,
                )
                .await,
            ))
        }
        // What a candidate policy would have done to the decisions already
        // taken, from `wardryx replay`, in both halves. A read, and classified
        // as one in roles.rs: it applies nothing.
        "policy_proof" => {
            #[derive(serde::Deserialize)]
            struct A {
                incident_id: String,
                candidate: String,
            }
            let a: A = decode(args)?;
            Ok(reply(
                genaryx_api::proof::commands::policy_proof(
                    genaryx_api::proof::commands::ProofRequest {
                        incident_id: a.incident_id,
                        candidate: a.candidate,
                    },
                    &ctx.bus,
                )
                .await,
            ))
        }
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
            genaryx_api::remote::commands::remote_operator_wg_config(wg_journal(ctx).as_ref())
                .await,
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
                genaryx_api::remote::commands::remote_operator_wg_revoke(
                    a.public_key,
                    wg_journal(ctx).as_ref(),
                )
                .await,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every lifecycle toggle names itself, and no two name themselves the
    /// same thing. Stopping a unit and starting it again are opposite acts on
    /// a whole fleet; an audit trail that spells them identically cannot
    /// answer the only question anybody asks it afterwards.
    #[test]
    fn every_block_direction_has_its_own_action_name() {
        let names: Vec<&str> = ["agent", "unit", "user"]
            .into_iter()
            .flat_map(|kind| [block_action(kind, true), block_action(kind, false)])
            .collect();

        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(
            unique.len(),
            names.len(),
            "duplicate action names: {names:?}"
        );
        for name in &names {
            assert!(
                name.starts_with("console."),
                "{name} must carry the console prefix every other journaled action uses"
            );
        }
    }

    #[test]
    fn block_and_unblock_are_not_the_same_action() {
        assert_eq!(block_action("agent", true), "console.block_agent");
        assert_eq!(block_action("agent", false), "console.unblock_agent");
        assert_eq!(block_action("unit", true), "console.block_unit");
        assert_eq!(block_action("unit", false), "console.unblock_unit");
        // `apply_block` treats anything that is not `agent`/`unit` as a user.
        assert_eq!(block_action("user", true), "console.block_user");
        assert_eq!(block_action("anything-else", false), "console.unblock_user");
    }

    // -- the whole dispatch table, on a box whose planes never resolved ------
    //
    // Every command below is driven through `dispatch` itself against a
    // hermetic context: every plane in its `pending()` state (never
    // `resolve()`d), no bus store, no vouchryx, no closure secret. Each one
    // was read first to make sure it answers from that state without
    // reaching a network outside loopback, spawning a binary or writing
    // outside a temporary directory. Left out on purpose: the commands that
    // read the process environment for an endpoint or a file (`keys_custody`,
    // `record_status`, `record_verify`, `notifications_sent`,
    // `routines_status`, `routines_history`, `onboard_status`), the three
    // operator WireGuard commands, and `agent_block` (it always has one
    // member, so it reaches the policy plane's environment).

    use http_body_util::BodyExt;
    use serde_json::json;

    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "gw-dispatch-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    fn ctx_over(bus: genaryx_api::bus::AppState) -> Arc<Ctx> {
        let cfg = crate::config::Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            state_dir: scratch_dir("state"),
            ui_dir: None,
            secure_cookies: false,
            require_passkey: false,
            delegation: genaryx_api::delegation::env::RevokeConfig::NotConfigured,
            closure: genaryx_api::money::closure::ClosureConfig::NotConfigured,
            approver_key_file: None,
            preview_key_file: None,
        };
        let (events_tx, _) = tokio::sync::broadcast::channel(16);
        Arc::new(Ctx::bootstrap(cfg, bus, events_tx))
    }

    fn pending_ctx() -> Arc<Ctx> {
        ctx_over(genaryx_api::bus::AppState {
            events_dir: None,
            source_events_dir: None,
            mode: genaryx_api::bus::BusMode::Unavailable {
                reason: "test".into(),
            },
        })
    }

    /// What one command answered: its status, its JSON body and its
    /// `cache-control` header, if any.
    struct Answer {
        status: StatusCode,
        body: Value,
        cache: Option<String>,
    }

    async fn answer_of(resp: Response) -> Answer {
        let status = resp.status();
        let cache = resp
            .headers()
            .get(CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        Answer {
            status,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            cache,
        }
    }

    /// Dispatch a command this table knows. An `Err` (the unknown-name 404)
    /// for one of them is itself the failure.
    async fn run(ctx: &Arc<Ctx>, name: &str, args: Value) -> Answer {
        match dispatch(ctx, name, args).await {
            Ok(resp) => answer_of(resp).await,
            Err(resp) => panic!(
                "{name} is a dispatch arm, yet it answered as an unknown command ({})",
                resp.status()
            ),
        }
    }

    /// Dispatch arguments that cannot decode: `decode` answers through the
    /// `Err` side with its own 400, before any plane is touched.
    async fn refused(ctx: &Arc<Ctx>, name: &str, args: Value) -> Answer {
        match dispatch(ctx, name, args).await {
            Ok(resp) => panic!(
                "{name} accepted arguments it cannot decode ({})",
                resp.status()
            ),
            Err(resp) => answer_of(resp).await,
        }
    }

    #[tokio::test]
    async fn every_command_on_a_pending_plane_answers_422_bootstrapping() {
        let ctx = pending_ctx();
        let cases = [
            (
                "admission_baseline",
                json!({ "evalset_path": "/nonexistent/evals.jsonl", "model": "m",
                        "agent_id": "agent://acme.example/bot/a",
                        "api_key": "test-key-not-a-secret" }),
            ),
            (
                "admission_check",
                json!({ "key_id": "k", "agent_id": "agent://acme.example/bot/a" }),
            ),
            ("credentials_gateway_retained_runs", json!({})),
            ("credentials_posture", json!({})),
            ("credentials_keys", json!({})),
            ("crypto_scan_cbom", json!({ "path": "/nonexistent" })),
            ("crypto_scan_evidence", json!({ "path": "/nonexistent" })),
            ("crypto_scan_ncsc", json!({ "path": "/nonexistent" })),
            ("crypto_verify_evidence", json!({ "file": "/nonexistent" })),
            (
                "drills_run",
                json!({ "scenario_dir": "/nonexistent", "fail_on_skip": false }),
            ),
            (
                "evidence_build",
                json!({ "request": { "include_cloud": false, "include_qryx": false,
                                     "include_idryx": false, "include_tokenfuse": false } }),
            ),
            ("identity_list_alerts", json!({})),
            ("identity_list_identities", json!({})),
            ("identity_list_remediations", json!({})),
            ("identity_rescan", json!({})),
            ("memory_forget", json!({ "memory_id": "m-1" })),
            (
                "memory_recall",
                json!({ "query": "q", "limit": 3, "mode": "hybrid" }),
            ),
            ("memory_stats", json!({})),
            ("memory_why", json!({ "memory_id": "m-1" })),
            ("money_ack_incident", json!({ "id": "inc-1" })),
            ("money_incidents", json!({})),
            ("money_list_retained", json!({})),
            (
                "money_kill_run",
                json!({ "run_id": "run-1", "reason": "runaway spend" }),
            ),
            ("money_overview", json!({})),
            ("money_runs", json!({})),
            ("money_spend_window", json!({})),
            ("money_owners", json!({})),
            ("money_savings", json!({})),
            (
                "money_set_budget",
                json!({ "run_id": "run-1", "budget_usd": 1.5, "reason": "cap it" }),
            ),
            ("money_gateways", json!({})),
            ("money_units", json!({})),
            (
                "money_set_unit_budget",
                json!({ "unit": "sre", "budget_usd": 5.0, "reason": "month cap" }),
            ),
            (
                "policy_decide_approval",
                json!({ "id": "hold-1", "decision": "deny" }),
            ),
            ("policy_approval_preview", json!({ "id": "hold-1" })),
            ("policy_grant_tool", json!({ "id": "hold-1" })),
            ("policy_list_approvals", json!({})),
            ("policy_list_policies", json!({})),
            ("policy_enforcement_status", json!({})),
            ("quality_list_baselines", json!({})),
            ("quality_list_run_summaries", json!({})),
            ("quality_run_scores", json!({ "run_id": "run-1" })),
            (
                "remote_set_environment",
                json!({ "request": {
                    "name": "campaign-1", "wireguard_go_bin": "",
                    "wg_peer_public_key_hex": "bb", "wg_endpoint": "192.0.2.1:51820",
                    "wg_allowed_ips": ["10.9.0.0/24"], "wg_persistent_keepalive": null,
                    "wg_listen_port": null, "wg_local_ip": "10.9.0.2",
                    "wg_peer_ip": "10.9.0.1", "ssh_host": "10.9.0.1", "ssh_port": 22,
                    "ssh_user": "ops", "ssh_identity_file": "/nonexistent/id",
                    "ssh_pinned_host_key": "ssh-ed25519 AAAA-test-only"
                } }),
            ),
            ("remote_ssh_check_reachable", json!({})),
            (
                "remote_ssh_read_file",
                json!({ "path": "/var/log/app.log" }),
            ),
            (
                "remote_ssh_tail_start",
                json!({ "path": "/var/log/app.log", "from_offset": 0 }),
            ),
            ("remote_ssh_tail_stop", json!({})),
            ("remote_wg_connect", json!({})),
            ("remote_wg_disconnect", json!({})),
        ];
        for (name, args) in cases {
            let a = run(&ctx, name, args).await;
            assert_eq!(
                a.status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{name}: {}",
                a.body
            );
            assert_eq!(a.body["kind"], "bootstrapping", "{name}: {}", a.body);
        }
    }

    #[tokio::test]
    async fn every_plane_status_read_answers_200_with_the_bootstrapping_state() {
        let ctx = pending_ctx();
        for name in [
            "money_status",
            "policy_status",
            "identity_status",
            "credentials_status",
            "crypto_status",
            "drills_status",
            "evidence_status",
            "memory_status",
            "quality_status",
            "remote_status",
        ] {
            // A read that takes no arguments ignores whatever body arrives.
            let a = run(&ctx, name, Value::Null).await;
            assert_eq!(a.status, StatusCode::OK, "{name}: {}", a.body);
            assert_eq!(a.body["state"], "bootstrapping", "{name}: {}", a.body);
            assert_eq!(a.cache, None, "{name} is not marked no-store");
        }
        let a = run(&ctx, "admission_status", json!({})).await;
        assert_eq!(a.status, StatusCode::OK, "{}", a.body);
        assert_eq!(a.body["gateway"]["state"], "bootstrapping", "{}", a.body);
    }

    #[tokio::test]
    async fn bus_reads_on_a_box_with_no_store_say_they_measured_nothing() {
        let ctx = pending_ctx();

        let a = run(&ctx, "bus_status", json!({})).await;
        assert_eq!(a.status, StatusCode::OK);
        assert_eq!(a.body, json!({ "kind": "unavailable", "reason": "test" }));

        for (name, args) in [
            ("bus_quarantine", json!({})),
            ("egress_recent", json!({ "limit": 5 })),
            ("stats_counts", json!({ "scan": 10 })),
            ("stats_counts", json!({ "scan": 10, "window_days": 30 })),
            (
                "agent_stops",
                json!({ "agent_id": "agent://acme.example/bot/a" }),
            ),
        ] {
            let a = run(&ctx, name, args).await;
            assert_eq!(a.status, StatusCode::OK, "{name}: {}", a.body);
            assert_eq!(a.body["measured"], false, "{name}: {}", a.body);
            assert!(
                a.body["note"]
                    .as_str()
                    .is_some_and(|n| n.contains("no event store")),
                "{name} must say why it measured nothing: {}",
                a.body
            );
        }

        let a = run(
            &ctx,
            "agent_profile",
            json!({ "agent_id": "agent://acme.example/bot/a" }),
        )
        .await;
        assert_eq!(a.status, StatusCode::OK);
        assert_eq!(a.body, Value::Null, "no store, no profile");

        for (name, args) in [
            (
                "agent_events",
                json!({ "agent_id": "agent://acme.example/bot/a", "limit": 5 }),
            ),
            ("run_events", json!({ "run_id": "run-1", "limit": 5 })),
        ] {
            let a = run(&ctx, name, args).await;
            assert_eq!(a.status, StatusCode::OK, "{name}");
            assert_eq!(a.body, json!([]), "{name}: {}", a.body);
        }

        let a = run(&ctx, "agent_graph", json!({})).await;
        assert_eq!(a.status, StatusCode::OK);
        assert_eq!(a.body["nodes"], json!([]), "{}", a.body);

        let a = run(
            &ctx,
            "agent_slice",
            json!({ "agent_id": "agent://acme.example/bot/a" }),
        )
        .await;
        assert_eq!(a.status, StatusCode::OK);
        assert_eq!(a.body["node"], Value::Null, "{}", a.body);
        assert_eq!(a.body["parents"], json!([]), "{}", a.body);

        // With no store the Bus Explorer falls back to its labelled demo rows,
        // and never more of them than were asked for.
        let a = run(&ctx, "recent_events", json!({ "limit": 3 })).await;
        assert_eq!(a.status, StatusCode::OK);
        let rows = a.body.as_array().expect("an array of events");
        assert!(!rows.is_empty() && rows.len() <= 3, "{}", a.body);
    }

    #[tokio::test]
    async fn copilot_commands_before_felyx_starts_answer_with_its_own_sentence() {
        let ctx = pending_ctx();
        for (name, args) in [
            (
                "copilot_ask",
                json!({ "question": "which runs are over budget?" }),
            ),
            ("copilot_explain", json!({ "incident_id": "bus:1" })),
            ("copilot_explain_case", json!({ "incident_id": "bus:1" })),
        ] {
            let a = run(&ctx, name, args).await;
            assert_eq!(a.status, StatusCode::UNPROCESSABLE_ENTITY, "{name}");
            assert_eq!(
                a.body,
                json!("Copilot is still starting up; try again in a moment."),
                "{name}"
            );
        }

        let a = run(&ctx, "copilot_status", json!({})).await;
        assert_eq!(a.status, StatusCode::OK);
        assert_eq!(a.body["enabled"], false);
        assert_eq!(a.body["disabled_reason"], "Copilot is still starting up.");

        // An approved proposal with no paired Money device is answered, and
        // says it was not journaled rather than claiming it was.
        let a = run(
            &ctx,
            "copilot_log_proposal_approved",
            json!({ "kind": "kill_run", "target": "run-1", "params": {} }),
        )
        .await;
        assert_eq!(a.status, StatusCode::OK, "{}", a.body);
        assert_eq!(a.body["journaled"], false, "{}", a.body);
        assert_eq!(
            a.body["journal_error"],
            "no paired Money device to journal against"
        );
    }

    #[tokio::test]
    async fn refusals_that_never_reach_a_plane_name_their_own_cause() {
        let ctx = pending_ctx();
        let agent = "agent://acme.example/bot/a";
        let cases = [
            (
                "delegation_revoke",
                json!({ "subject": agent, "reason": "compromised" }),
                "not_configured",
            ),
            (
                "delegation_revoke",
                json!({ "subject": agent, "jti": "t-1", "reason": "compromised" }),
                "exactly_one_target",
            ),
            (
                "delegation_revoke",
                json!({ "subject": "host://box", "reason": "compromised" }),
                "invalid_subject",
            ),
            (
                "agent_stop",
                json!({ "subject": "not-an-agent", "reason": "runaway" }),
                "invalid_subject",
            ),
            (
                "agent_stop",
                json!({ "subject": agent, "reason": "runaway" }),
                "not_stopped",
            ),
            (
                "agent_lift",
                json!({ "subject": agent, "reason": "fixed" }),
                "not_lifted",
            ),
            (
                "money_close_retained",
                json!({ "reservation_id": 1, "expected_revision": "0011223344556677",
                        "evidence_kind": "no_charge", "evidence_ref": "ticket-1" }),
                "not_configured",
            ),
            (
                "money_close_retained",
                json!({ "reservation_id": 1, "expected_revision": "not-hex",
                        "evidence_kind": "no_charge", "evidence_ref": "ticket-1" }),
                "invalid_args",
            ),
            (
                "money_kill_run",
                json!({ "run_id": "run-1", "reason": "   " }),
                "break_glass_missing_reason",
            ),
            (
                "money_set_unit_budget",
                json!({ "unit": "unassigned", "budget_usd": 5.0, "reason": "cap" }),
                "invalid_argument",
            ),
            (
                "remote_cloud_list",
                json!({ "provider": "no-such-cloud" }),
                "invalid",
            ),
            (
                "remote_hetzner_list",
                json!({ "token": "", "label_selector": null }),
                "invalid",
            ),
            ("remote_ssh_read_file", json!({ "path": " " }), "invalid"),
            (
                "remote_ssh_tail_start",
                json!({ "path": "", "from_offset": 0 }),
                "invalid",
            ),
            (
                "onboard_generate",
                json!({ "request": { "trust_domain": "NOT A DOMAIN", "path": "bot/a",
                                     "unit": "sre", "owner": "ops" } }),
                "invalid_input",
            ),
            (
                "onboard_write_passport",
                json!({ "request": { "passport_json": "not json",
                                     "passport_path": "a.json", "overwrite": false } }),
                "invalid_input",
            ),
            ("case_file", json!({ "incident_id": "bus:1" }), "no_store"),
            (
                "case_file",
                json!({ "incident_id": "money:7" }),
                "unsupported_source",
            ),
            (
                "case_file",
                json!({ "incident_id": "bus:seven" }),
                "bad_incident_id",
            ),
            (
                "case_seal",
                json!({ "incident_id": "bus:1", "decision": { "outcome": "declined" } }),
                "case",
            ),
            (
                "case_pack",
                json!({ "digest": "sha256:short" }),
                "bad_digest",
            ),
            (
                "case_pack",
                json!({ "digest": format!("sha256:{}", "a".repeat(64)) }),
                "no_store",
            ),
        ];
        for (name, args, kind) in cases {
            let a = run(&ctx, name, args).await;
            assert_eq!(
                a.status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{name}: {}",
                a.body
            );
            assert_eq!(a.body["kind"], kind, "{name}: {}", a.body);
        }

        // The seal names the case refusal it stopped at.
        let a = run(
            &ctx,
            "case_seal",
            json!({ "incident_id": "bus:1", "decision": { "outcome": "applied" } }),
        )
        .await;
        assert_eq!(a.body["error"]["kind"], "no_store", "{}", a.body);
    }

    #[tokio::test]
    async fn proofs_and_remedies_with_no_store_are_refused_not_proven() {
        let ctx = pending_ctx();
        for (name, args) in [
            (
                "budget_proof",
                json!({ "incident_id": "bus:1", "per_run_usd": "0.5" }),
            ),
            (
                "budget_proof",
                json!({ "incident_id": "bus:1", "per_step_usd": 0.25, "max_steps": 10 }),
            ),
            (
                "policy_proof",
                json!({ "incident_id": "bus:1", "candidate": "deny_above_usd: 1" }),
            ),
            (
                "remedy_check",
                json!({ "incident_id": "bus:1",
                        "remedy": { "kind": "stop_agent", "subject": "agent://a.example/b/c" } }),
            ),
        ] {
            let a = run(&ctx, name, args).await;
            assert_eq!(
                a.status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{name}: {}",
                a.body
            );
            assert!(
                a.body["kind"].is_string(),
                "{name} names its refusal: {}",
                a.body
            );
        }
    }

    #[tokio::test]
    async fn a_unit_or_user_with_no_members_is_refused_and_nothing_is_recorded() {
        let ctx = pending_ctx();
        let a = run(
            &ctx,
            "unit_block",
            json!({ "team": "sre", "blocked": true }),
        )
        .await;
        assert_eq!(a.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            a.body,
            json!({ "error": "nothing to block: no agents resolved for this unit" })
        );
        let a = run(
            &ctx,
            "user_block",
            json!({ "user": "ops.lead", "blocked": true }),
        )
        .await;
        assert_eq!(a.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            a.body,
            json!({ "error": "nothing to block: no agents resolved for this user" })
        );

        // A refused enforcement leaves the console claiming nothing.
        let a = run(&ctx, "lifecycle_blocks", json!({})).await;
        assert_eq!(a.status, StatusCode::OK);
        assert_eq!(a.body, json!({ "agents": [], "units": [], "users": [] }));
        let store = ctx.lifecycle.read().unwrap();
        assert!(store.stopped_units.is_empty());
        assert!(store.stopped_users.is_empty());
    }

    #[tokio::test]
    async fn a_grant_answer_and_a_preview_are_never_cached() {
        let ctx = pending_ctx();
        for (name, args) in [
            (
                "policy_decide_approval",
                json!({ "id": "hold-1", "decision": "grant", "seen_pvd": "sha256:x" }),
            ),
            ("policy_approval_preview", json!({ "id": "hold-1" })),
        ] {
            let a = run(&ctx, name, args).await;
            assert_eq!(a.cache.as_deref(), Some("no-store"), "{name}");
        }
        let a = run(&ctx, "policy_list_approvals", json!({})).await;
        assert_eq!(a.cache, None, "an ordinary read carries no cache rule");
    }

    #[tokio::test]
    async fn malformed_arguments_are_a_400_that_says_what_did_not_decode() {
        let ctx = pending_ctx();
        let needs_args = [
            "admission_baseline",
            "admission_check",
            "agent_events",
            "agent_slice",
            "egress_recent",
            "stats_counts",
            "agent_profile",
            "agent_stops",
            "recent_events",
            "case_file",
            "copilot_ask",
            "copilot_explain",
            "copilot_explain_case",
            "copilot_log_proposal_approved",
            "crypto_scan_cbom",
            "crypto_scan_evidence",
            "crypto_scan_ncsc",
            "crypto_verify_evidence",
            "delegation_revoke",
            "agent_stop",
            "agent_lift",
            "drills_run",
            "evidence_build",
            "memory_forget",
            "memory_recall",
            "memory_why",
            "money_ack_incident",
            "money_close_retained",
            "money_kill_run",
            "agent_block",
            "unit_block",
            "user_block",
            "money_set_budget",
            "money_set_unit_budget",
            "onboard_generate",
            "onboard_status",
            "onboard_write_passport",
            "policy_decide_approval",
            "policy_approval_preview",
            "policy_grant_tool",
            "case_seal",
            "remedy_check",
            "case_pack",
            "budget_proof",
            "policy_proof",
            "quality_run_scores",
            "remote_cloud_list",
            "remote_hetzner_list",
            "remote_operator_wg_revoke",
            "remote_set_environment",
            "remote_ssh_read_file",
            "remote_ssh_tail_start",
            "run_events",
        ];
        for name in needs_args {
            let a = refused(&ctx, name, json!({})).await;
            assert_eq!(a.status, StatusCode::BAD_REQUEST, "{name}: {}", a.body);
            assert_eq!(a.body["error"], "bad arguments", "{name}");
            assert!(
                a.body["detail"].as_str().is_some_and(|d| !d.is_empty()),
                "{name} says what did not decode: {}",
                a.body
            );
        }

        // Present but the wrong shape is refused the same way, never coerced.
        for (name, args) in [
            ("agent_events", json!({ "agent_id": "a", "limit": "ten" })),
            (
                "money_set_budget",
                json!({ "run_id": "r", "budget_usd": "lots", "reason": "x" }),
            ),
            (
                "policy_decide_approval",
                json!({ "id": "h", "decision": "maybe" }),
            ),
            // `deny_unknown_fields`: a seal request carries nothing unasked.
            (
                "case_seal",
                json!({ "incident_id": "bus:1", "decision": { "outcome": "declined" },
                        "extra": true }),
            ),
            // An amount is text or a number, never a boolean.
            (
                "budget_proof",
                json!({ "incident_id": "bus:1", "per_run_usd": true }),
            ),
        ] {
            let a = refused(&ctx, name, args).await;
            assert_eq!(a.status, StatusCode::BAD_REQUEST, "{name}: {}", a.body);
            assert_eq!(a.body["error"], "bad arguments", "{name}");
        }
    }

    #[tokio::test]
    async fn an_unknown_command_is_a_404_that_names_it() {
        let ctx = pending_ctx();
        let Err(resp) = dispatch(&ctx, "no_such_command", json!({})).await else {
            panic!("an unknown name must not dispatch");
        };
        let a = answer_of(resp).await;
        assert_eq!(a.status, StatusCode::NOT_FOUND);
        assert_eq!(
            a.body,
            json!({ "error": "unknown command", "command": "no_such_command" })
        );
    }

    #[tokio::test]
    async fn the_response_helpers_shape_ok_err_and_cache() {
        let a = answer_of(ok(json!({ "n": 1 }))).await;
        assert_eq!(a.status, StatusCode::OK);
        assert_eq!(a.body, json!({ "n": 1 }));
        assert_eq!(a.cache, None);

        let a = answer_of(reply::<u8, _>(Err(json!({ "kind": "refused" })))).await;
        assert_eq!(a.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            a.body,
            json!({ "kind": "refused" }),
            "the error itself, unwrapped"
        );

        let a = answer_of(reply::<_, String>(Ok(7u8))).await;
        assert_eq!(a.status, StatusCode::OK);
        assert_eq!(a.body, json!(7));

        let a = answer_of(no_store(reply::<u8, _>(Err("no".to_string())))).await;
        assert_eq!(a.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(a.body, json!("no"));
        assert_eq!(a.cache.as_deref(), Some("no-store"));

        let decoded: Result<u32, Response> = decode(json!(42));
        assert_eq!(decoded.ok(), Some(42));
        let Err(refused) = decode::<u32>(json!("forty-two")) else {
            panic!("a string is not a u32");
        };
        let a = answer_of(refused).await;
        assert_eq!(a.status, StatusCode::BAD_REQUEST);
        assert_eq!(a.body["error"], "bad arguments");
    }

    #[test]
    fn an_owner_matches_on_its_last_path_segment_either_way() {
        assert!(owner_matches("ops.lead", "ops.lead"));
        assert!(owner_matches("user://acme/ops.lead", "ops.lead"));
        assert!(owner_matches("ops.lead", "user://acme/ops.lead"));
        assert!(owner_matches("user://a/ops.lead", "user://b/ops.lead"));
        assert!(!owner_matches("user://acme/ops.lead", "ops"));
        assert!(!owner_matches("ops.lead", "ops.lead2"));
    }

    #[test]
    fn the_argument_defaults_are_a_quarter_a_month_and_twenty_thousand_lines() {
        assert_eq!(default_profile_window(), 90);
        assert_eq!(default_spend_days(), 30);
        assert_eq!(default_stops_scan(), 20_000);
    }

    #[tokio::test]
    async fn a_journal_handle_exists_only_when_both_bus_directories_do() {
        assert!(wg_journal(&pending_ctx()).is_none());

        let store = scratch_dir("store");
        let source = scratch_dir("source");
        let ctx = ctx_over(genaryx_api::bus::AppState {
            events_dir: Some(store.clone()),
            source_events_dir: None,
            mode: genaryx_api::bus::BusMode::Unavailable {
                reason: "test".into(),
            },
        });
        assert!(wg_journal(&ctx).is_none(), "one directory is not enough");

        let ctx = ctx_over(genaryx_api::bus::AppState {
            events_dir: Some(store.clone()),
            source_events_dir: Some(source.clone()),
            mode: genaryx_api::bus::BusMode::Unavailable {
                reason: "test".into(),
            },
        });
        let handle = wg_journal(&ctx).expect("both directories give a handle");
        assert_eq!(handle.store_db_path, store.join("console.sqlite"));
        assert_eq!(handle.console_events_path, source.join("console.ndjson"));

        // With a store directory, a well-formed digest is looked up there and
        // reported missing, never read from anywhere else.
        let a = run(
            &ctx,
            "case_pack",
            json!({ "digest": format!("sha256:{}", "b".repeat(64)) }),
        )
        .await;
        assert_eq!(a.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(a.body["kind"], "not_found", "{}", a.body);
    }
}
