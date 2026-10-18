//! "Stop agent" and "Lift" against a stub vouchryx and a stub TokenFuse
//! Cloud, both the hand-rolled recording stub in `cloud_stub.rs`.
//!
//! What is under test is the console's own sequence: the ban it asks
//! vouchryx for, the runs it kills on the Cloud, and the line it writes into
//! its own chain for every step. vouchryx's ban and the Cloud's kill are
//! their own repositories' tests; the stubs answer the way their handlers
//! answer at vouchryx `1c16afe` (`revokeHandler`, `liftHandler`) and
//! tokenfuse `ddf8dc5` (`http.rs::kill`, `http.rs::runs`).

use crate::cloud_stub::{Seen, money_with_device_at, spawn_recording_cloud};
use genaryx_api::console_actor::{ConsoleSignature, with_signature};
use genaryx_api::delegation::env::{RevokeConfig, RevokeKey};
use genaryx_api::delegation::stop::{
    LiftOutcome, StopError, StopOutcome, StopStep, agent_lift, agent_stop,
};
use genaryx_api::money::state::{BusHandle, MoneyState};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const AGENT: &str = "agent://acme.example/bots/researcher";
/// Starts with AGENT's id: a prefix match would take its runs too.
const NEIGHBOUR: &str = "agent://acme.example/bots/researcher-2";
const OTHER: &str = "agent://acme.example/bots/writer";
const KEY: &str = "stop-test-revoke-key-7f3a9c";

const BANNED: &str = r#"{"revoked":true,"expires":1791990000,"banned":true,"durable":true}"#;

type Log = Arc<Mutex<Vec<Seen>>>;

fn run(id: &str, agent: &str, killed: bool, last_seen: i64) -> Value {
    json!({
        "run_id": id, "model": "claude-haiku-5-5", "agent_id": agent,
        "spent_microusd": 1200, "calls": 3, "cache_hits": 0, "steps": 3,
        "last_seen_millis": last_seen, "killed": killed,
    })
}

/// Two open runs of AGENT, one it already had killed, one of a neighbour
/// whose id starts with AGENT's, and one of another agent.
fn fleet() -> String {
    json!([
        run("r-old", AGENT, false, 1_791_985_000_000),
        run("r-new", AGENT, false, 1_791_985_900_000),
        run("r-done", AGENT, true, 1_791_985_500_000),
        run("r-neighbour", NEIGHBOUR, false, 1_791_985_950_000),
        run("r-other", OTHER, false, 1_791_985_960_000),
    ])
    .to_string()
}

fn killed(id: &str) -> String {
    json!({ "killed": id }).to_string()
}

fn kill_routes() -> Vec<(&'static str, &'static str, String)> {
    vec![
        ("/v1/runs", "200 OK", fleet()),
        ("/v1/runs/r-old/kill", "200 OK", killed("r-old")),
        ("/v1/runs/r-new/kill", "200 OK", killed("r-new")),
        ("/v1/runs/r-done/kill", "200 OK", killed("r-done")),
        ("/v1/runs/r-neighbour/kill", "200 OK", killed("r-neighbour")),
        ("/v1/runs/r-other/kill", "200 OK", killed("r-other")),
    ]
}

fn vouchryx(routes: Vec<(&'static str, &'static str, String)>) -> (RevokeConfig, Log) {
    let (base, log) = spawn_recording_cloud(routes);
    (configured(&base), log)
}

fn configured(base: &str) -> RevokeConfig {
    RevokeConfig::Configured {
        vouchryx_url: base.to_string(),
        revoke_key: RevokeKey::new(KEY).expect("a key"),
    }
}

async fn cloud(routes: Vec<(&'static str, &'static str, String)>) -> (MoneyState, Log) {
    let (base, log) = spawn_recording_cloud(routes);
    (money_with_device_at(&base).await, log)
}

fn scratch(tag: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-agent-stop-{tag}-{}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn bus(tag: &str) -> BusHandle {
    let dir = scratch(tag);
    BusHandle::from_dirs(&dir, &dir)
}

/// Every line in the console's chain, as the emitted event's `data`.
fn chain(bus: &BusHandle) -> Vec<Value> {
    let text = std::fs::read_to_string(&bus.console_events_path).unwrap_or_default();
    let conformer = genaryx_core::Conformer::new().expect("schemas compile");
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let report = conformer.check_line(l);
            assert!(report.valid, "{:?}\n{l}", report.errors);
            let v: Value = serde_json::from_str(l).expect("a JSON line");
            v["data"].clone()
        })
        .collect()
}

fn actions(lines: &[Value]) -> Vec<String> {
    lines
        .iter()
        .map(|d| d["action"].as_str().unwrap_or("").to_string())
        .collect()
}

fn posts(log: &Log) -> Vec<Seen> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|s| s.method == "POST")
        .cloned()
        .collect()
}

fn killed_paths(log: &Log) -> Vec<String> {
    let mut paths: Vec<String> = posts(log).into_iter().map(|s| s.path).collect();
    paths.sort();
    paths
}

fn not_stopped(r: Result<StopOutcome, StopError>) -> StopOutcome {
    match r {
        Err(StopError::NotStopped { outcome }) => *outcome,
        other => panic!("expected not_stopped, got {other:?}"),
    }
}

fn not_lifted(r: Result<LiftOutcome, StopError>) -> LiftOutcome {
    match r {
        Err(StopError::NotLifted { outcome }) => *outcome,
        other => panic!("expected not_lifted, got {other:?}"),
    }
}

fn step<'a>(o: &'a [StopStep], name: &str) -> &'a StopStep {
    o.iter()
        .find(|s| s.step == name)
        .unwrap_or_else(|| panic!("no {name} step in {o:?}"))
}

#[tokio::test]
async fn stopping_an_agent_bans_it_and_kills_each_open_run() {
    let (config, vlog) = vouchryx(vec![("/v1/revoke", "200 OK", BANNED.to_string())]);
    let (money, clog) = cloud(kill_routes()).await;
    let bus = bus("stops");

    let outcome = agent_stop(
        AGENT.into(),
        "compromised".into(),
        &config,
        &money,
        Some(&bus),
    )
    .await
    .expect("a complete stop");

    assert!(outcome.stopped);
    assert_eq!(outcome.ban_durable, Some(true));
    assert_eq!(outcome.open_runs, Some(2));
    assert_eq!(outcome.runs_killed, 2);
    assert_eq!(
        killed_paths(&clog),
        vec!["/v1/runs/r-new/kill", "/v1/runs/r-old/kill"],
        "exactly the agent's two open runs"
    );
    assert_eq!(posts(&vlog).len(), 1, "one ban, nothing else");
    assert_eq!(posts(&vlog)[0].path, "/v1/revoke");
}

#[tokio::test]
async fn the_stop_asks_vouchryx_for_a_ban_not_a_revocation() {
    let (config, vlog) = vouchryx(vec![("/v1/revoke", "200 OK", BANNED.to_string())]);
    let (money, _) = cloud(kill_routes()).await;

    agent_stop(
        AGENT.into(),
        "  compromised  ".into(),
        &config,
        &money,
        None,
    )
    .await
    .expect("stopped");

    let sent = posts(&vlog);
    assert_eq!(sent.len(), 1);
    let body: Value = serde_json::from_str(&sent[0].body).expect("a JSON body");
    assert_eq!(body["ban"], json!(true), "the ban flag is the whole point");
    assert_eq!(body["subject"], json!(AGENT));
    assert_eq!(body["jti"], json!(""), "a ban names a subject and no token");
    assert_eq!(body["reason"], json!("compromised"));
    assert!(
        body["actor"]
            .as_str()
            .is_some_and(|a| a.starts_with("user://")),
        "{body}"
    );
    assert_eq!(
        sent[0].header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
}

#[tokio::test]
async fn every_step_of_a_stop_is_journaled_in_order_ceremony_first() {
    let (config, _) = vouchryx(vec![("/v1/revoke", "200 OK", BANNED.to_string())]);
    let (money, _) = cloud(kill_routes()).await;
    let bus = bus("order");

    let outcome = agent_stop(
        AGENT.into(),
        "compromised".into(),
        &config,
        &money,
        Some(&bus),
    )
    .await
    .expect("stopped");

    let lines = chain(&bus);
    assert_eq!(
        actions(&lines),
        vec![
            "console.stop_agent.ceremony",
            "console.stop_agent.ban",
            "console.stop_agent.list_runs",
            "console.stop_agent.kill_run",
            "console.stop_agent.kill_run",
            "console.stop_agent",
        ]
    );
    // Newest run first: the one most likely still spending.
    assert_eq!(lines[3]["target"], json!("r-new"));
    assert_eq!(lines[4]["target"], json!("r-old"));
    assert_eq!(lines[1]["verify_result"], json!("banned:durable"));
    assert_eq!(lines[1]["http_status"], json!(200));
    assert_eq!(lines[5]["verify_result"], json!("stopped"));
    // No passkey ran in this test, and the line says so.
    assert_eq!(
        lines[0]["verify_result"],
        json!("not_confirmed:no_passkey_enrolled")
    );
    for d in &lines {
        assert_eq!(d["decision"], json!("break_glass"));
    }
    assert!(
        outcome.steps.iter().all(|s| s.journaled),
        "{:?}",
        outcome.steps
    );
    assert_eq!(outcome.steps.len(), lines.len());
}

#[tokio::test]
async fn a_confirmed_ceremony_is_the_first_line_and_signs_every_step() {
    let (config, _) = vouchryx(vec![("/v1/revoke", "200 OK", BANNED.to_string())]);
    let (money, _) = cloud(kill_routes()).await;
    let bus = bus("signed");
    let sig = ConsoleSignature {
        alg: "webauthn-es256".into(),
        fpr: "cred-stop-1".into(),
    };

    with_signature(Some(sig), async {
        agent_stop(
            AGENT.into(),
            "compromised".into(),
            &config,
            &money,
            Some(&bus),
        )
        .await
        .expect("stopped")
    })
    .await;

    let lines = chain(&bus);
    assert_eq!(lines[0]["action"], json!("console.stop_agent.ceremony"));
    assert_eq!(lines[0]["verify_result"], json!("confirmed:webauthn-es256"));
    for d in &lines {
        assert_eq!(d["sig_alg"], json!("webauthn-es256"), "{d}");
        assert_eq!(d["sig_fpr"], json!("cred-stop-1"), "{d}");
    }
}

#[tokio::test]
async fn a_revocation_without_a_confirmed_ban_is_never_a_stop() {
    // A vouchryx older than the ban ignores the field and answers this.
    let (config, _) = vouchryx(vec![(
        "/v1/revoke",
        "200 OK",
        r#"{"revoked":true,"expires":1791990000}"#.to_string(),
    )]);
    let (money, clog) = cloud(kill_routes()).await;
    let bus = bus("nobanned");

    let outcome = not_stopped(
        agent_stop(
            AGENT.into(),
            "compromised".into(),
            &config,
            &money,
            Some(&bus),
        )
        .await,
    );

    assert!(!outcome.stopped);
    assert_eq!(outcome.ban_durable, None);
    let ban = step(&outcome.steps, "ban");
    assert!(!ban.ok);
    assert!(
        ban.verify_result.starts_with("revoked_not_banned"),
        "{}",
        ban.verify_result
    );
    // The runs are still killed: stopping spend is never worse.
    assert_eq!(outcome.runs_killed, 2);
    assert_eq!(killed_paths(&clog).len(), 2);
    let lines = chain(&bus);
    let verdict = lines.last().expect("a verdict line");
    assert_eq!(verdict["action"], json!("console.stop_agent"));
    assert!(
        verdict["verify_result"]
            .as_str()
            .is_some_and(|v| v.starts_with("not_stopped") && v.contains("revoked_not_banned")),
        "{verdict}"
    );
}

#[tokio::test]
async fn a_refused_ban_is_shown_and_recorded_and_the_runs_are_still_killed() {
    let cases: [(&'static str, String, u16, &str); 6] = [
        (
            "401 Unauthorized",
            r#"{"error":"invalid_client"}"#.into(),
            401,
            "refused:invalid_client",
        ),
        (
            "403 Forbidden",
            r#"{"error":"access_denied"}"#.into(),
            403,
            "refused:access_denied",
        ),
        (
            "400 Bad Request",
            r#"{"error":"invalid_request"}"#.into(),
            400,
            "refused:invalid_request",
        ),
        (
            "503 Service Unavailable",
            r#"{"error":"temporarily_unavailable"}"#.into(),
            503,
            "not_durable:temporarily_unavailable",
        ),
        (
            "200 OK",
            "<html>not vouchryx</html>".into(),
            200,
            "200 without revoked:true",
        ),
        (
            "200 OK",
            r#"{"revoked":false}"#.into(),
            200,
            "200 without revoked:true",
        ),
    ];
    for (status, body, code, verify) in cases {
        let (config, _) = vouchryx(vec![("/v1/revoke", status, body.clone())]);
        let (money, clog) = cloud(kill_routes()).await;
        let bus = bus("refused");
        let outcome = not_stopped(
            agent_stop(
                AGENT.into(),
                "compromised".into(),
                &config,
                &money,
                Some(&bus),
            )
            .await,
        );
        let ban = step(&outcome.steps, "ban");
        assert!(!ban.ok, "{status} {body}");
        assert_eq!(ban.http_status, code, "{status} {body}");
        assert!(
            ban.verify_result.starts_with(verify),
            "{status}: {}",
            ban.verify_result
        );
        assert!(ban.journaled, "{status}: the failed ban is recorded");
        assert_eq!(outcome.runs_killed, 2, "{status}");
        assert_eq!(killed_paths(&clog).len(), 2, "{status}");
        let lines = chain(&bus);
        assert_eq!(
            lines[1]["http_status"],
            json!(code),
            "{status}: the real status"
        );
    }

    // No answer at all: a port nothing listens on.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let (money, _) = cloud(kill_routes()).await;
    let outcome = not_stopped(
        agent_stop(
            AGENT.into(),
            "compromised".into(),
            &configured(&base),
            &money,
            None,
        )
        .await,
    );
    let ban = step(&outcome.steps, "ban");
    assert_eq!(ban.http_status, 0);
    assert!(
        ban.verify_result.starts_with("unreachable"),
        "{}",
        ban.verify_result
    );
    assert_eq!(outcome.runs_killed, 2);

    // A console with no vouchryx configured says so and still kills.
    let (money, _) = cloud(kill_routes()).await;
    let outcome = not_stopped(
        agent_stop(
            AGENT.into(),
            "compromised".into(),
            &RevokeConfig::NotConfigured,
            &money,
            None,
        )
        .await,
    );
    let ban = step(&outcome.steps, "ban");
    assert!(
        ban.verify_result.starts_with("not_configured"),
        "{}",
        ban.verify_result
    );
    assert_eq!(outcome.runs_killed, 2);
}

#[tokio::test]
async fn a_run_the_cloud_will_not_kill_makes_the_stop_incomplete() {
    for (status, body, verify) in [
        (
            "500 Internal Server Error",
            r#"{"error":"x"}"#.to_string(),
            "error:",
        ),
        ("200 OK", killed("r-somewhere-else"), "unconfirmed"),
    ] {
        let (config, _) = vouchryx(vec![("/v1/revoke", "200 OK", BANNED.to_string())]);
        let mut routes = kill_routes();
        routes.retain(|(p, _, _)| *p != "/v1/runs/r-old/kill");
        routes.push(("/v1/runs/r-old/kill", status, body));
        let (money, _) = cloud(routes).await;
        let bus = bus("killfail");

        let outcome = not_stopped(
            agent_stop(
                AGENT.into(),
                "compromised".into(),
                &config,
                &money,
                Some(&bus),
            )
            .await,
        );

        assert!(!outcome.stopped, "{status}");
        assert_eq!(outcome.runs_killed, 1, "{status}");
        let failed = outcome
            .steps
            .iter()
            .find(|s| s.step == "kill_run" && s.target == "r-old")
            .expect("the r-old kill step");
        assert!(!failed.ok, "{status}");
        assert!(
            failed.verify_result.starts_with(verify),
            "{status}: {}",
            failed.verify_result
        );
        assert!(failed.journaled, "{status}");
        let verdict = chain(&bus).last().cloned().expect("verdict");
        assert!(
            verdict["verify_result"]
                .as_str()
                .is_some_and(|v| v.contains("1 of 2 run kill(s) failed")),
            "{status}: {verdict}"
        );
    }
}

#[tokio::test]
async fn runs_that_cannot_be_listed_make_the_stop_incomplete() {
    let (config, vlog) = vouchryx(vec![("/v1/revoke", "200 OK", BANNED.to_string())]);
    let (money, clog) = cloud(vec![(
        "/v1/runs",
        "500 Internal Server Error",
        r#"{"error":"x"}"#.to_string(),
    )])
    .await;
    let bus = bus("nolist");

    let outcome = not_stopped(
        agent_stop(
            AGENT.into(),
            "compromised".into(),
            &config,
            &money,
            Some(&bus),
        )
        .await,
    );
    assert_eq!(outcome.open_runs, None, "unknown, never zero");
    let list = step(&outcome.steps, "list_runs");
    assert!(!list.ok);
    assert_eq!(list.http_status, 500);
    assert!(list.journaled);
    assert!(step(&outcome.steps, "ban").ok, "the ban itself held");
    assert_eq!(posts(&vlog).len(), 1);
    assert!(posts(&clog).is_empty(), "nothing to kill was known");

    // A Money panel that never connected: the same, with its reason.
    let outcome = not_stopped(
        agent_stop(
            AGENT.into(),
            "compromised".into(),
            &config,
            &MoneyState::pending(),
            None,
        )
        .await,
    );
    let list = step(&outcome.steps, "list_runs");
    assert!(!list.ok);
    assert!(
        list.verify_result.starts_with("error:"),
        "{}",
        list.verify_result
    );
}

#[tokio::test]
async fn only_the_agents_own_open_runs_are_killed() {
    let (config, _) = vouchryx(vec![("/v1/revoke", "200 OK", BANNED.to_string())]);
    let (money, clog) = cloud(kill_routes()).await;

    agent_stop(AGENT.into(), "compromised".into(), &config, &money, None)
        .await
        .expect("stopped");

    let paths = killed_paths(&clog);
    for spared in ["r-done", "r-neighbour", "r-other"] {
        assert!(
            !paths.iter().any(|p| p.contains(spared)),
            "{spared} must not be killed: {paths:?}"
        );
    }
}

#[tokio::test]
async fn a_ban_vouchryx_cannot_keep_is_a_stop_that_says_so() {
    let (config, _) = vouchryx(vec![(
        "/v1/revoke",
        "200 OK",
        r#"{"revoked":true,"expires":1791990000,"banned":true,"durable":false}"#.to_string(),
    )]);
    let (money, _) = cloud(kill_routes()).await;
    let bus = bus("notdurable");

    let outcome = agent_stop(
        AGENT.into(),
        "compromised".into(),
        &config,
        &money,
        Some(&bus),
    )
    .await
    .expect("the ban is in force now, so it is a stop");

    assert_eq!(outcome.ban_durable, Some(false));
    assert!(
        outcome.warnings.iter().any(|w| w.contains("not durable")),
        "{:?}",
        outcome.warnings
    );
    assert_eq!(
        step(&outcome.steps, "ban").verify_result,
        "banned:not_durable"
    );
    let verdict = chain(&bus).last().cloned().expect("verdict");
    assert_eq!(verdict["verify_result"], json!("stopped:ban_not_durable"));
}

#[tokio::test]
async fn lifting_an_agent_asks_vouchryx_to_lift_the_ban_and_records_it() {
    let (config, vlog) = vouchryx(vec![(
        "/v1/revoke/lift",
        "200 OK",
        r#"{"lifted":true,"durable":true}"#.to_string(),
    )]);
    let bus = bus("lift");

    let outcome = agent_lift(
        AGENT.into(),
        "cleared after review".into(),
        &config,
        Some(&bus),
    )
    .await
    .expect("lifted");

    assert!(outcome.lifted);
    assert_eq!(outcome.lift_durable, Some(true));
    let sent = posts(&vlog);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].path, "/v1/revoke/lift");
    let body: Value = serde_json::from_str(&sent[0].body).expect("JSON");
    assert_eq!(body["subject"], json!(AGENT));
    assert_eq!(body["reason"], json!("cleared after review"));
    assert!(
        body["actor"]
            .as_str()
            .is_some_and(|a| a.starts_with("user://"))
    );
    assert!(
        body.get("ban").is_none(),
        "a lift carries no ban flag: {body}"
    );
    assert_eq!(
        sent[0].header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    let lines = chain(&bus);
    assert_eq!(
        actions(&lines),
        vec![
            "console.lift_agent.ceremony",
            "console.lift_agent.lift",
            "console.lift_agent",
        ]
    );
    assert_eq!(lines[1]["verify_result"], json!("lifted:durable"));
    assert_eq!(lines[2]["verify_result"], json!("lifted"));
}

#[tokio::test]
async fn a_lift_vouchryx_refuses_is_never_reported_as_lifted() {
    for (status, body, code, verify) in [
        (
            "400 Bad Request",
            r#"{"error":"invalid_request"}"#,
            400,
            "refused:invalid_request",
        ),
        (
            "401 Unauthorized",
            r#"{"error":"invalid_client"}"#,
            401,
            "refused:invalid_client",
        ),
        (
            "503 Service Unavailable",
            r#"{"error":"temporarily_unavailable"}"#,
            503,
            "not_durable:temporarily_unavailable",
        ),
        ("200 OK", "{}", 200, "200 without lifted:true"),
    ] {
        let (config, _) = vouchryx(vec![("/v1/revoke/lift", status, body.to_string())]);
        let bus = bus("liftrefused");
        let outcome =
            not_lifted(agent_lift(AGENT.into(), "cleared".into(), &config, Some(&bus)).await);
        assert!(!outcome.lifted, "{status}");
        let lift = step(&outcome.steps, "lift");
        assert_eq!(lift.http_status, code, "{status}");
        assert!(
            lift.verify_result.starts_with(verify),
            "{status}: {}",
            lift.verify_result
        );
        let lines = chain(&bus);
        let verdict = lines.last().expect("verdict");
        assert_eq!(verdict["action"], json!("console.lift_agent"));
        assert!(
            verdict["verify_result"]
                .as_str()
                .is_some_and(|v| v.starts_with("not_lifted")),
            "{status}: {verdict}"
        );
    }
}

#[tokio::test]
async fn a_stop_naming_no_agent_reaches_nobody() {
    let (config, vlog) = vouchryx(vec![("/v1/revoke", "200 OK", BANNED.to_string())]);
    let (money, clog) = cloud(kill_routes()).await;
    let bus = bus("refusedargs");

    for subject in [
        "",
        "user://acme.example/alice",
        "agent://Acme.example/bots/x",
        "agent://acme.example",
        "agent://acme.example/bots/x?all=1",
    ] {
        let got = agent_stop(
            subject.into(),
            "compromised".into(),
            &config,
            &money,
            Some(&bus),
        )
        .await;
        assert!(
            matches!(got, Err(StopError::InvalidSubject { .. })),
            "{subject:?}: {got:?}"
        );
        let got = agent_lift(subject.into(), "cleared".into(), &config, Some(&bus)).await;
        assert!(
            matches!(got, Err(StopError::InvalidSubject { .. })),
            "{subject:?}: {got:?}"
        );
    }
    let long = "x".repeat(501);
    for reason in ["", "   ", long.as_str()] {
        let got = agent_stop(
            AGENT.into(),
            reason.to_string(),
            &config,
            &money,
            Some(&bus),
        )
        .await;
        assert!(
            matches!(got, Err(StopError::InvalidReason { .. })),
            "{}: {got:?}",
            reason.len()
        );
    }
    assert!(posts(&vlog).is_empty(), "vouchryx must not be called");
    assert!(posts(&clog).is_empty(), "the Cloud must not be called");
    assert!(chain(&bus).is_empty(), "an argument refusal is not a step");
}

#[tokio::test]
async fn the_revoke_key_never_appears_in_a_stop_journal_or_answer() {
    let (config, _) = vouchryx(vec![(
        "/v1/revoke",
        "401 Unauthorized",
        r#"{"error":"invalid_client"}"#.into(),
    )]);
    let (money, _) = cloud(kill_routes()).await;
    let bus = bus("key");

    let got = agent_stop(
        AGENT.into(),
        "compromised".into(),
        &config,
        &money,
        Some(&bus),
    )
    .await;
    let answer = serde_json::to_string(&got.as_ref().err()).unwrap();
    let file = std::fs::read_to_string(&bus.console_events_path).unwrap_or_default();
    assert!(!file.is_empty(), "the attempt was journaled");
    assert!(!answer.contains(KEY), "{answer}");
    assert!(!file.contains(KEY), "{file}");
    assert!(!format!("{got:?}").contains(KEY));
}
