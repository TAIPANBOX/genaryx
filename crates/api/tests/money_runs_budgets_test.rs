//! The Money panel's runs table shows every run budget the Cloud holds, and a
//! budget it could not read is never shown as "no budget".
//!
//! The defect, measured 2026-10-05 on the forge lab: `money_runs` read a run's
//! budget only from `GET /v1/alerts` plus budgets set in the current console
//! session. `/v1/alerts` lists a run only once it is near or over its limit,
//! so mig-flint (budget 4500 uUSD, spent 3210, under the alert threshold)
//! showed no budget at all, and the table rendered it as "no cap". A real
//! budget read as an absence: invariant 8.
//!
//! The fixture is the same forge Cloud `crates/copilot/tests/
//! felyx_reads_budgets_test.rs` carries, the same run ids and figures. The
//! stub is hand-rolled and routes by path, the same approach that file takes.

use genaryx_api::money::commands::{RunDto, money_runs};
use genaryx_api::money::env::EnvSource;
use genaryx_api::money::state::{MoneyClient, MoneyInner, MoneyState};
use genaryx_connectors::CloudClient;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ---- the forge Cloud, 2026-10-05 -------------------------------------------

const BUDGETS: &str =
    r#"{"genaryx-copilot":100000,"mig-flint":4500,"p1-brume":1000,"mig-brume":1278}"#;

fn run(run_id: &str, agent: &str, spent: i64) -> Value {
    json!({
        "run_id": run_id, "model": "claude-haiku-4-5", "agent_id": agent,
        "spent_microusd": spent, "calls": 1, "cache_hits": 0, "steps": 1,
        "last_seen_millis": 0, "killed": false,
    })
}

fn runs_body() -> String {
    let r = "agent://taipanbox.dev/routers";
    json!([
        run("p1-beryl2", &format!("{r}/beryl2"), 4019),
        run("mig-flint", &format!("{r}/flint"), 3210),
        run("p1-brume", &format!("{r}/brume"), 2008),
        run("mig-brume", &format!("{r}/brume"), 1278),
        run("genaryx-copilot", "agent://local/genaryx/felyx", 92576),
    ])
    .to_string()
}

const ALERTS: &str = r#"[
  {"run_id":"p1-brume","spent_microusd":2008,"budget_micros":1000,"fraction":2.008,"killed":false},
  {"run_id":"mig-brume","spent_microusd":1278,"budget_micros":1278,"fraction":1.0,"killed":false}
]"#;

// ---- stub Cloud ------------------------------------------------------------

/// path -> (status line, body); anything else is a 404.
type Routes = HashMap<&'static str, (&'static str, String)>;

fn spawn_cloud(routes: Vec<(&'static str, &'static str, String)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
    let base = format!("http://{}", listener.local_addr().unwrap());
    let routes: Routes = routes
        .into_iter()
        .map(|(path, status, body)| (path, (status, body)))
        .collect();
    let routes = Arc::new(Mutex::new(routes));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            serve(stream, &routes.lock().unwrap());
        }
    });
    base
}

fn serve(stream: TcpStream, routes: &Routes) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let target = line.split_whitespace().nth(1).unwrap_or("").to_string();
    let path = target.split('?').next().unwrap_or("").to_string();
    let mut content_length = 0usize;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).is_err() {
            return;
        }
        let t = h.trim_end_matches(['\r', '\n']);
        if t.is_empty() {
            break;
        }
        if let Some((n, v)) = t.split_once(':')
            && n.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).ok();
    }
    let (status, out) = routes
        .get(path.as_str())
        .cloned()
        .unwrap_or(("404 Not Found", r#"{"error":"not found"}"#.to_string()));
    let mut w = reader.into_inner();
    let _ = write!(
        w,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
        out.len()
    );
    let _ = w.flush();
}

/// A Ready Money panel pointed at `base`. No device is attached: the runs
/// table is reads only.
async fn money_at(base: &str) -> MoneyState {
    let state = MoneyState::pending();
    *state.inner.lock().await = MoneyInner::Ready(MoneyClient {
        client: Arc::new(CloudClient::new(base, "test-token").expect("client")),
        source: EnvSource::EnvFallback,
        cloud_url: base.to_string(),
        org_domain: "local".to_string(),
        operator: "user://local/test".to_string(),
        host: "test".to_string(),
        sig_fpr: "software-signed",
        bus: None,
    });
    state
}

fn forge_cloud() -> String {
    spawn_cloud(vec![
        ("/v1/runs", "200 OK", runs_body()),
        ("/v1/budgets", "200 OK", BUDGETS.to_string()),
        ("/v1/alerts", "200 OK", ALERTS.to_string()),
    ])
}

fn row<'a>(rows: &'a [RunDto], run_id: &str) -> &'a RunDto {
    rows.iter()
        .find(|r| r.run_id == run_id)
        .unwrap_or_else(|| panic!("{run_id} is missing from the runs table"))
}

// ---- the defect ------------------------------------------------------------

#[tokio::test]
async fn a_run_with_a_cloud_budget_and_no_alert_shows_its_budget() {
    let state = money_at(&forge_cloud()).await;
    let rows = money_runs(&state).await.expect("runs table");

    // mig-flint: 4500 uUSD budget, 3210 spent, under the alert threshold, so
    // absent from /v1/alerts. Its budget is still a fact the Cloud holds.
    let flint = row(&rows, "mig-flint");
    assert_eq!(
        flint.budget_usd,
        Some(0.0045),
        "mig-flint has a 4500 uUSD budget on the Cloud and the table must show it"
    );

    // And every other budgeted run, alerted or not.
    assert_eq!(row(&rows, "genaryx-copilot").budget_usd, Some(0.1));
    assert_eq!(row(&rows, "p1-brume").budget_usd, Some(0.001));
    assert_eq!(row(&rows, "mig-brume").budget_usd, Some(0.001278));
}

#[tokio::test]
async fn a_run_with_no_cloud_budget_reads_as_no_budget() {
    let state = money_at(&forge_cloud()).await;
    let rows = money_runs(&state).await.expect("runs table");

    // p1-beryl2 has spend and no budget anywhere. That is a real "none", and
    // the table may say so, because the budget map answered.
    let beryl = row(&rows, "p1-beryl2");
    assert_eq!(beryl.budget_usd, None);
    assert!(
        rows.iter().all(|r| r.budgets_read),
        "the budget map answered, so every row must say its budget was read"
    );
}

#[tokio::test]
async fn the_cloud_budget_map_wins_over_the_alert_figure() {
    // The two come from the same store and normally agree. When they do not,
    // the budget map is the source: alerts are a derived view of it.
    let base = spawn_cloud(vec![
        ("/v1/runs", "200 OK", runs_body()),
        (
            "/v1/budgets",
            "200 OK",
            r#"{"mig-brume":2000,"mig-flint":4500}"#.to_string(),
        ),
        ("/v1/alerts", "200 OK", ALERTS.to_string()),
    ]);
    let state = money_at(&base).await;
    let rows = money_runs(&state).await.expect("runs table");
    assert_eq!(row(&rows, "mig-brume").budget_usd, Some(0.002));
    // p1-brume is alerted with a 1000 budget but absent from this map: the
    // map is the answer, so it has no budget.
    assert_eq!(row(&rows, "p1-brume").budget_usd, None);
}

/// A Cloud whose budget map cannot be read, for each way that happens.
async fn unreadable_budgets(budgets_status: &'static str) -> Vec<RunDto> {
    let mut routes = vec![
        ("/v1/runs", "200 OK", runs_body()),
        ("/v1/alerts", "200 OK", ALERTS.to_string()),
    ];
    if budgets_status != "404 Not Found" {
        routes.push((
            "/v1/budgets",
            budgets_status,
            r#"{"error":"boom"}"#.to_string(),
        ));
    }
    let state = money_at(&spawn_cloud(routes)).await;
    money_runs(&state)
        .await
        .expect("a budget map that cannot be read must not cost the runs table")
}

#[tokio::test]
async fn a_cloud_that_cannot_answer_budgets_says_so_rather_than_no_budget() {
    for status in [
        "404 Not Found",
        "500 Internal Server Error",
        "403 Forbidden",
    ] {
        let rows = unreadable_budgets(status).await;
        assert_eq!(rows.len(), 5, "{status}: every run must still be listed");
        assert!(
            rows.iter().all(|r| !r.budgets_read),
            "{status}: no row may claim its budget was read"
        );
        // What /v1/alerts knows is still a real figure and is still shown.
        assert_eq!(row(&rows, "p1-brume").budget_usd, Some(0.001), "{status}");
        // mig-flint's budget is unknown here, and the row says so.
        let flint = row(&rows, "mig-flint");
        assert_eq!(flint.budget_usd, None, "{status}");
        assert!(!flint.budgets_read, "{status}");
    }
}

#[tokio::test]
async fn a_cloud_that_answers_neither_budgets_nor_alerts_still_lists_its_runs() {
    let base = spawn_cloud(vec![("/v1/runs", "200 OK", runs_body())]);
    let state = money_at(&base).await;
    let rows = money_runs(&state).await.expect("runs table");
    assert_eq!(rows.len(), 5);
    assert!(
        rows.iter()
            .all(|r| r.budget_usd.is_none() && !r.budgets_read)
    );
}

#[tokio::test]
async fn a_cloud_that_cannot_list_runs_is_an_error_not_an_empty_table() {
    let base = spawn_cloud(vec![
        ("/v1/runs", "500 Internal Server Error", "{}".to_string()),
        ("/v1/budgets", "200 OK", BUDGETS.to_string()),
    ]);
    let state = money_at(&base).await;
    assert!(money_runs(&state).await.is_err());
}

// ---- hostile budget bodies -------------------------------------------------

/// The budget map is bytes from outside the process. None of these may panic,
/// cost the runs table, or produce a budget the body did not contain.
#[tokio::test]
async fn hostile_budget_bodies_never_cost_the_table_or_invent_a_budget() {
    let named = [
        "",
        "null",
        "[]",
        "{",
        r#"{"mig-flint":"4500"}"#,
        r#"{"mig-flint":4.5}"#,
        r#"{"mig-flint":99999999999999999999999}"#,
        r#"{"mig-flint":null}"#,
        r#"[{"run_id":"mig-flint","budget_micros":4500}]"#,
        "<html>not the Cloud</html>",
    ];
    for body in named {
        let rows = rows_for_budget_body(body.to_string()).await;
        assert!(
            rows.iter().all(|r| !r.budgets_read),
            "body {body:?} cannot be a budget map, no row may claim it was read"
        );
        assert_eq!(row(&rows, "mig-flint").budget_usd, None, "body {body:?}");
    }

    // A 200-seed sweep of JSON-shaped noise: whatever parses must be exactly
    // what the body said, and whatever does not must say it was not read.
    const ATOMS: &[&str] = &[
        "{",
        "}",
        "[",
        "]",
        ":",
        ",",
        "\"mig-flint\"",
        "\"p1-brume\"",
        "4500",
        "-1",
        "0",
        "1e400",
        "null",
        "true",
        "\"x\"",
        " ",
    ];
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    for seed in 0..200u32 {
        let mut body = String::new();
        let len = 1 + (seed as usize % 12);
        for _ in 0..len {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            body.push_str(ATOMS[(state >> 33) as usize % ATOMS.len()]);
        }
        let parsed: Option<HashMap<String, i64>> = serde_json::from_str(&body).ok();
        let rows = rows_for_budget_body(body.clone()).await;
        assert_eq!(rows.len(), 5, "seed {seed}, body {body:?}");
        for r in &rows {
            assert_eq!(
                r.budgets_read,
                parsed.is_some(),
                "seed {seed}, body {body:?}"
            );
            if let Some(map) = &parsed {
                assert_eq!(
                    r.budget_usd,
                    map.get(&r.run_id).map(|m| *m as f64 / 1_000_000.0),
                    "seed {seed}, body {body:?}, run {}",
                    r.run_id
                );
            }
        }
    }
}

async fn rows_for_budget_body(body: String) -> Vec<RunDto> {
    let base = spawn_cloud(vec![
        ("/v1/runs", "200 OK", runs_body()),
        ("/v1/budgets", "200 OK", body),
    ]);
    let state = money_at(&base).await;
    money_runs(&state).await.expect("runs table")
}
