//! The budget proof, with a stand-in `tokenfuse` that prints what it is told to
//! and records how it was run.
//!
//! The parsers and the two halves are held in
//! `crates/api/src/proof/budget/tests.rs`. What is held here is the process
//! around them: the four children in their order, the exact arguments and
//! directories, the empty environment, the copy of the run and its removal, one
//! deadline for all of them, the slot, and each way a proof is refused rather
//! than invented.

use genaryx_api::bus::{AppState, BusMode};
use genaryx_api::case::Data;
use genaryx_api::proof::budget::BudgetProof;
use genaryx_api::proof::budget_commands::{
    BudgetConfig, BudgetRequest, budget_proof_in, prove_budget_in,
};
use genaryx_api::proof::commands::ProofError;
use genaryx_api::proof::slots::Slots;
use genaryx_core::store::Store;
use genaryx_core::{AgentEvent, ConsoleEvent, Provenance, SchemaVersion};
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SCHEMA: &str = "taipanbox.dev/agent-event/v0.2";

fn report_text(
    candidate: &str,
    runs: (u64, u64),
    calls: (u64, u64),
    money: (&str, &str, &str),
) -> String {
    format!(
        "TokenFuse backtest\n  candidate policy   : {candidate}\n  runs               : {} ({} affected)\n  calls              : {} ({} would be blocked)\n  spend in trace     : ${}\n  would have saved   : ${} (50.0%)\n  projected spend    : ${}\n",
        runs.0, runs.1, calls.0, calls.1, money.0, money.1, money.2
    )
}

const LIMIT: &str = "budget/run $0.0020";

fn run_report() -> String {
    report_text(
        LIMIT,
        (1, 1),
        (35, 18),
        ("0.005674", "0.003810", "0.001864"),
    )
}

fn trace_report() -> String {
    report_text(
        LIMIT,
        (230, 15),
        (832, 70),
        ("0.256379", "0.204439", "0.051940"),
    )
}

fn copy_table(n: u64) -> String {
    format!("+-------+\n| count |\n+-------+\n| {n:<5} |\n+-------+\n")
}

fn steps_table(n: u64, steps: u64) -> String {
    format!(
        "+----+-------+\n| n  | steps |\n+----+-------+\n| {n:<2} | {steps:<5} |\n+----+-------+\n"
    )
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-budget-test-{tag}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create_dir_all");
    dir
}

/// What the stand-in prints for each of its four children, and how long each
/// takes.
struct Script {
    copy: String,
    steps: String,
    alone: String,
    trace: String,
    stderr: String,
    delay: u32,
}

impl Script {
    fn ok() -> Self {
        Self {
            copy: copy_table(35),
            steps: steps_table(35, 35),
            alone: run_report(),
            trace: trace_report(),
            stderr: String::new(),
            delay: 0,
        }
    }
}

/// A `tokenfuse` that records each call (its arguments and its environment)
/// and prints the part of `script` that call asks for. It tells the whole-trace
/// backtest from the copy's by `TOKENFUSE_DATA_DIR`.
fn fake_tokenfuse(dir: &Path, script: &Script) -> BudgetConfig {
    let trace_dir = dir.join("trace");
    std::fs::create_dir_all(&trace_dir).expect("trace dir");
    let trace_path = trace_dir.display().to_string();
    for (name, text) in [
        ("copy.out", script.copy.as_str()),
        ("steps.out", script.steps.as_str()),
        ("alone.out", script.alone.as_str()),
        ("trace.out", script.trace.as_str()),
        ("err.txt", script.stderr.as_str()),
        ("trace.path", trace_path.as_str()),
    ] {
        std::fs::write(dir.join(name), text).expect("state file");
    }
    let d = dir.display();
    let delay = script.delay;
    let body = format!(
        "#!/bin/sh\n\
         {{ echo \"== $1\"; printf '%s\\n' \"$@\"; echo '-- env'; env | sort; echo '== end'; }} >> '{d}/calls.log'\n\
         case \"$1\" in\n\
         sql)\n\
           case \"$2\" in\n\
             copy*) cat '{d}/copy.out' ;;\n\
             *) cat '{d}/steps.out' ;;\n\
           esac ;;\n\
         backtest)\n\
           if [ \"$TOKENFUSE_DATA_DIR\" = \"$(cat '{d}/trace.path')\" ]; then cat '{d}/trace.out'; else cat '{d}/alone.out'; fi ;;\n\
         esac\n\
         cat '{d}/err.txt' 1>&2\n\
         if [ {delay} -gt 0 ]; then sleep {delay}; fi\n\
         exit 0\n"
    );
    let bin = dir.join("tokenfuse");
    std::fs::write(&bin, body).expect("script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    BudgetConfig {
        tokenfuse_bin: bin,
        trace_dir: Some(trace_dir),
    }
}

struct Call {
    args: Vec<String>,
    env: Vec<String>,
}

fn calls(dir: &Path) -> Vec<Call> {
    let log = std::fs::read_to_string(dir.join("calls.log")).unwrap_or_default();
    log.split("== end\n")
        .filter(|block| !block.trim().is_empty())
        .map(|block| {
            let (mut args, mut env, mut in_env) = (Vec::new(), Vec::new(), false);
            for line in block.lines().skip(1) {
                if line == "-- env" {
                    in_env = true;
                } else if in_env {
                    env.push(line.to_string());
                } else {
                    args.push(line.to_string());
                }
            }
            Call { args, env }
        })
        .collect()
}

fn event(n: u64, run: Option<&str>) -> ConsoleEvent {
    ConsoleEvent {
        event: AgentEvent {
            schema: SCHEMA.to_string(),
            ts: "2026-10-08T10:00:00Z".to_string(),
            source: "tokenfuse".to_string(),
            event_type: "budget_exceeded".to_string(),
            agent_id: "agent://a/one".to_string(),
            severity: Some("high".to_string()),
            run_id: run.map(str::to_string),
            on_behalf_of: Vec::new(),
            data: Some(json!({})),
            prev_hash: None,
            extra: serde_json::Map::new(),
        },
        provenance: Provenance {
            env: "test".to_string(),
            connector: "fixture".to_string(),
            file: Some("tokenfuse.ndjson".to_string()),
            offset: Some(n),
            endpoint: None,
            received_ts: "2026-11-04T00:00:00Z".to_string(),
        },
        raw: json!({ "n": n }).to_string(),
        schema_version: SchemaVersion::from_schema_str(SCHEMA).expect("a known schema"),
    }
}

fn live_bus(dir: &Path) -> AppState {
    AppState {
        events_dir: Some(dir.to_path_buf()),
        source_events_dir: Some(dir.to_path_buf()),
        mode: BusMode::Live {
            env: "test".to_string(),
            dir: dir.display().to_string(),
        },
    }
}

/// A bus whose first event is in `run` and whose second is in no run; returns
/// the bus and the ids of the two rows.
fn bus_with_run(dir: &Path, run: &str) -> (AppState, Vec<i64>) {
    let store = Store::open(&dir.join("console.sqlite")).expect("Store::open");
    store
        .insert_batch(&[event(1, Some(run)), event(2, None)])
        .expect("insert_batch");
    let ids: Vec<i64> = store
        .recent_events(10)
        .expect("recent_events")
        .into_iter()
        .rev()
        .map(|e| e.id)
        .collect();
    drop(store);
    (live_bus(dir), ids)
}

fn bus(dir: &Path) -> (AppState, Vec<i64>) {
    bus_with_run(dir, "r-incident")
}

fn request(id: i64, per_run: Option<&str>) -> BudgetRequest {
    BudgetRequest {
        incident_id: format!("bus:{id}"),
        per_run_usd: per_run.map(str::to_string),
        per_step_usd: None,
        max_steps: None,
    }
}

/// A freshly written script can be refused once as "text file busy" while a
/// sibling test forks; that is the test's race, not the proof's, so it is tried
/// again.
fn prove_again(
    config: &BudgetConfig,
    bus: &AppState,
    request: &BudgetRequest,
    timeout: Duration,
) -> Result<BudgetProof, ProofError> {
    let slots = Slots::new(usize::MAX);
    for _ in 0..5 {
        match prove_budget_in(&slots, config, bus, request, timeout) {
            Err(ProofError::Spawn { detail }) if detail.text.to_lowercase().contains("busy") => {
                std::thread::sleep(Duration::from_millis(100));
            }
            other => return other,
        }
    }
    prove_budget_in(&slots, config, bus, request, timeout)
}

const TEN_SECONDS: Duration = Duration::from_secs(10);

#[test]
fn the_proof_runs_four_children_in_order_over_the_right_directories_and_removes_the_copy() {
    let dir = temp_dir("order");
    let config = fake_tokenfuse(&dir, &Script::ok());
    let (bus, ids) = bus(&dir);
    let trace = config.trace_dir.clone().expect("a trace dir");
    prove_again(&config, &bus, &request(ids[0], Some("0.002")), TEN_SECONDS).expect("a proof");

    let made = calls(&dir);
    assert_eq!(made.len(), 4, "the four children");

    // 1: the copy of the run, out of the gateway's own trace.
    assert_eq!(made[0].args[0], "sql");
    let statement = &made[0].args[1];
    let head =
        "copy (select * from calls where run_id = 'r-incident' order by ts_millis, step) to '";
    assert!(statement.starts_with(head), "{statement}");
    let scratch = statement
        .strip_prefix(head)
        .and_then(|rest| rest.strip_suffix("/part.parquet' stored as parquet"))
        .unwrap_or_else(|| panic!("the statement: {statement}"))
        .to_string();
    assert_eq!(made[0].args.len(), 2);
    assert!(
        made[0]
            .env
            .contains(&format!("TOKENFUSE_DATA_DIR={}", trace.display()))
    );

    // 2: a count of the copy; 3: the backtest of the copy; 4: of the whole trace.
    assert_eq!(
        made[1].args,
        [
            "sql",
            "select count(*) as n, count(distinct step) as steps from calls"
        ]
    );
    assert_eq!(made[2].args[..2], ["backtest", "--budget"]);
    assert_eq!(made[3].args, made[2].args);
    for (child, data) in [(1, scratch.as_str()), (2, scratch.as_str())] {
        assert!(
            made[child]
                .env
                .contains(&format!("TOKENFUSE_DATA_DIR={data}")),
            "child {child}: {:?}",
            made[child].env
        );
    }
    assert!(
        made[3]
            .env
            .contains(&format!("TOKENFUSE_DATA_DIR={}", trace.display()))
    );

    // No environment but a PATH and the directory the child is pointed at.
    for call in &made {
        for line in call.env.iter().filter(|l| !l.is_empty()) {
            let name = line.split('=').next().unwrap_or_default();
            assert!(
                ["PATH", "TOKENFUSE_DATA_DIR", "PWD", "SHLVL", "_", "OLDPWD"].contains(&name),
                "the child saw {line}"
            );
        }
    }

    // The copy is gone when the proof ends.
    assert!(
        !Path::new(&scratch).exists(),
        "the copy of the run was left behind at {scratch}"
    );
}

#[test]
fn a_candidate_is_handed_over_as_the_amount_the_cli_will_read() {
    let dir = temp_dir("amount");
    let one_run = report_text(
        "budget/run $0.0002",
        (1, 1),
        (35, 18),
        ("0.005674", "0.003810", "0.001864"),
    );
    let whole = report_text(
        "budget/run $0.0002",
        (230, 15),
        (832, 70),
        ("0.256379", "0.204439", "0.051940"),
    );
    let config = fake_tokenfuse(
        &dir,
        &Script {
            alone: one_run,
            trace: whole,
            ..Script::ok()
        },
    );
    let (bus, ids) = bus(&dir);
    prove_again(
        &config,
        &bus,
        &request(ids[0], Some("0.000249")),
        TEN_SECONDS,
    )
    .expect("a proof");

    let made = calls(&dir);
    assert_eq!(made[2].args[1], "--budget");
    // The CLI reads the flag as `(v * 1e6) as i64`: 249 micro-dollars, not 248.
    let read = (made[2].args[2].parse::<f64>().expect("a number") * 1e6) as i64;
    assert_eq!(read, 249, "handed over as {}", made[2].args[2]);
}

#[test]
fn a_budget_proof_reports_both_halves_for_an_incident_in_a_real_store() {
    let dir = temp_dir("halves");
    let config = fake_tokenfuse(&dir, &Script::ok());
    let (bus, ids) = bus(&dir);
    let proof =
        prove_again(&config, &bus, &request(ids[0], Some("0.002")), TEN_SECONDS).expect("a proof");

    assert_eq!(proof.run, Data::bounded("r-incident"));
    assert!(proof.stops_this_incident);
    assert_eq!(proof.incident.calls, 35);
    assert_eq!(proof.incident.calls_blocked, 18);
    assert_eq!(proof.incident.saved_micros, 3_810);
    assert_eq!(proof.incident.spent_after_micros, 1_864);
    assert_eq!(proof.elsewhere.runs, 229);
    assert_eq!(proof.elsewhere.runs_affected, 14);
    assert_eq!(proof.elsewhere.calls_blocked, 52);
    assert_eq!(proof.elsewhere.saved_micros, 200_629);
    assert_eq!(proof.trace.runs_affected, 15);
    assert!(proof.elsewhere_exact);
}

#[test]
fn a_candidate_that_blocks_nothing_in_the_run_but_something_elsewhere_stops_nothing() {
    let dir = temp_dir("elsewhere-only");
    let config = fake_tokenfuse(
        &dir,
        &Script {
            alone: report_text(LIMIT, (1, 0), (35, 0), ("0.005674", "0.000000", "0.005674")),
            ..Script::ok()
        },
    );
    let (bus, ids) = bus(&dir);
    let proof =
        prove_again(&config, &bus, &request(ids[0], Some("0.002")), TEN_SECONDS).expect("a proof");
    assert!(!proof.stops_this_incident);
    assert_eq!(proof.incident.calls_blocked, 0);
    assert_eq!(proof.trace.calls_blocked, 70);
}

#[test]
fn calls_at_the_same_step_make_the_split_approximate() {
    let dir = temp_dir("tied");
    let config = fake_tokenfuse(
        &dir,
        &Script {
            steps: steps_table(35, 29),
            ..Script::ok()
        },
    );
    let (bus, ids) = bus(&dir);
    let proof =
        prove_again(&config, &bus, &request(ids[0], Some("0.002")), TEN_SECONDS).expect("a proof");
    assert!(!proof.elsewhere_exact);
    assert_eq!(proof.elsewhere.runs_affected, 14);
}

#[test]
fn a_run_the_trace_does_not_hold_is_not_a_proof_of_nothing() {
    let dir = temp_dir("not-in-trace");
    let config = fake_tokenfuse(
        &dir,
        &Script {
            copy: copy_table(0),
            steps: steps_table(3, 3),
            ..Script::ok()
        },
    );
    let (bus, ids) = bus(&dir);
    let result = prove_again(&config, &bus, &request(ids[0], Some("0.002")), TEN_SECONDS);
    assert_eq!(
        result.err(),
        Some(ProofError::NotInTrace {
            run: Data::bounded("r-incident")
        })
    );
    assert_eq!(calls(&dir).len(), 1, "nothing ran after the empty copy");
}

#[test]
fn a_run_id_that_could_break_the_query_never_reaches_it() {
    let dir = temp_dir("hostile-run");
    let config = fake_tokenfuse(&dir, &Script::ok());
    let (bus, ids) = bus_with_run(&dir, "r1' or run_id <> '");
    let result = prove_again(&config, &bus, &request(ids[0], Some("0.002")), TEN_SECONDS);
    assert!(
        matches!(result, Err(ProofError::RunRefused { .. })),
        "{result:?}"
    );
    assert!(calls(&dir).is_empty(), "a refused run id started a child");
}

#[test]
fn reports_that_disagree_with_each_other_are_refused() {
    let cases: Vec<(&str, Script)> = vec![
        (
            "a count of the copy that is not the copy",
            Script {
                steps: steps_table(34, 34),
                ..Script::ok()
            },
        ),
        (
            "a replay of another number of calls than were copied",
            Script {
                alone: report_text(
                    LIMIT,
                    (1, 1),
                    (36, 18),
                    ("0.005674", "0.003810", "0.001864"),
                ),
                ..Script::ok()
            },
        ),
        (
            "a whole trace that holds no run",
            Script {
                trace: report_text(LIMIT, (0, 0), (0, 0), ("0.000000", "0.000000", "0.000000")),
                ..Script::ok()
            },
        ),
        (
            "a replay that ran another candidate",
            Script {
                alone: report_text(
                    "budget/run $0.0030",
                    (1, 1),
                    (35, 18),
                    ("0.005674", "0.003810", "0.001864"),
                ),
                ..Script::ok()
            },
        ),
        (
            "a report whose projection is not the spend less the saving",
            Script {
                trace: report_text(
                    LIMIT,
                    (230, 15),
                    (832, 70),
                    ("0.256379", "0.204439", "0.100000"),
                ),
                ..Script::ok()
            },
        ),
    ];
    for (what, script) in cases {
        let dir = temp_dir("disagree");
        let config = fake_tokenfuse(&dir, &script);
        let (bus, ids) = bus(&dir);
        let result = prove_again(&config, &bus, &request(ids[0], Some("0.002")), TEN_SECONDS);
        assert!(
            matches!(result, Err(ProofError::Unreadable { .. })),
            "{what}: {result:?}"
        );
    }
}

#[test]
fn a_child_that_exits_zero_with_an_error_of_its_own_is_not_a_proof() {
    let dir = temp_dir("sql-error");
    let config = fake_tokenfuse(
        &dir,
        &Script {
            copy: String::new(),
            stderr: "sql error: no such directory".to_string(),
            ..Script::ok()
        },
    );
    let (bus, ids) = bus(&dir);
    match prove_again(&config, &bus, &request(ids[0], Some("0.002")), TEN_SECONDS) {
        Err(ProofError::ReplayFailed { detail, .. }) => {
            assert_eq!(detail.text, "sql error: no such directory");
        }
        other => panic!("expected the child's own failure, got {other:?}"),
    }
}

#[test]
fn an_input_that_is_missing_is_named_and_nothing_runs() {
    let dir = temp_dir("not-configured");
    let mut config = fake_tokenfuse(&dir, &Script::ok());
    config.trace_dir = None;
    let (bus, ids) = bus(&dir);
    match prove_again(&config, &bus, &request(ids[0], Some("0.002")), TEN_SECONDS) {
        Err(ProofError::NotConfigured { missing }) => {
            assert!(missing.contains("TOKENFUSE_DATA_DIR"), "{missing}");
        }
        other => panic!("expected NotConfigured, got {other:?}"),
    }
    // An incident with no run is its own answer.
    let config = fake_tokenfuse(&dir, &Script::ok());
    let result = prove_again(&config, &bus, &request(ids[1], Some("0.002")), TEN_SECONDS);
    assert_eq!(result.err(), Some(ProofError::NoRun));
    assert!(calls(&dir).is_empty());
}

#[test]
fn a_candidate_that_sets_no_limit_or_a_bad_amount_is_refused_before_anything_runs() {
    let dir = temp_dir("bad-candidate");
    let config = fake_tokenfuse(&dir, &Script::ok());
    let (bus, ids) = bus(&dir);
    let id = format!("bus:{}", ids[0]);
    let ask = |per_run: Option<&str>, per_step: Option<&str>, steps: Option<u32>| BudgetRequest {
        incident_id: id.clone(),
        per_run_usd: per_run.map(str::to_string),
        per_step_usd: per_step.map(str::to_string),
        max_steps: steps,
    };
    for bad in [
        ask(None, None, None),
        ask(Some("abc"), None, None),
        ask(Some("-1"), None, None),
        ask(None, Some("0"), None),
        ask(None, None, Some(0)),
        ask(Some("2"), Some("1e3"), None),
    ] {
        let result = prove_again(&config, &bus, &bad, TEN_SECONDS);
        assert!(
            matches!(result, Err(ProofError::CandidateRefused { .. })),
            "{bad:?}: {result:?}"
        );
    }
    assert!(
        calls(&dir).is_empty(),
        "a refused candidate started a child"
    );
}

#[test]
fn all_the_children_of_one_proof_share_one_deadline() {
    let dir = temp_dir("deadline");
    let config = fake_tokenfuse(
        &dir,
        &Script {
            delay: 1,
            ..Script::ok()
        },
    );
    let (bus, ids) = bus(&dir);
    let started = std::time::Instant::now();
    // Each child takes a second and the proof has two: no single child is late,
    // all of them together are.
    let result = prove_again(
        &config,
        &bus,
        &request(ids[0], Some("0.002")),
        Duration::from_secs(2),
    );
    assert!(
        matches!(result, Err(ProofError::Timeout { .. })),
        "{result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "the proof waited instead of stopping"
    );
}

/// A viewer may ask for a proof, and each one runs children. When every slot is
/// taken the next ask is answered at once with a busy error, not queued and not
/// started; and when the slot comes back the next ask is served. The slots are
/// the policy proof's own.
#[tokio::test]
async fn a_budget_proof_meets_a_busy_slot_as_an_answer_not_a_queue() {
    let slow_dir = temp_dir("busy-slow");
    let slow = fake_tokenfuse(
        &slow_dir,
        &Script {
            delay: 1,
            ..Script::ok()
        },
    );
    let (bus, ids) = bus(&slow_dir);
    let slots = Slots::new(1);

    let first = {
        let (slots, config, bus) = (slots.clone(), slow.clone(), live_bus(&slow_dir));
        let request = request(ids[0], Some("0.002"));
        tokio::spawn(async move {
            budget_proof_in(&slots, config, request, &bus, Duration::from_secs(30)).await
        })
    };
    for _ in 0..250 {
        if slots.in_use() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(slots.in_use(), 1, "the first proof never took its slot");

    let fast_dir = temp_dir("busy-fast");
    let fast = fake_tokenfuse(&fast_dir, &Script::ok());
    let asked = std::time::Instant::now();
    let second = budget_proof_in(
        &slots,
        fast.clone(),
        request(ids[0], Some("0.002")),
        &bus,
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(second.err(), Some(ProofError::Busy { limit: 1 }));
    assert!(
        asked.elapsed() < Duration::from_secs(2),
        "a busy slot waited instead of answering"
    );
    assert!(
        calls(&fast_dir).is_empty(),
        "a refused proof started a child"
    );

    first
        .await
        .expect("the first task")
        .expect("the first proof");
    assert_eq!(slots.in_use(), 0, "the slot was not given back");
    let third = budget_proof_in(
        &slots,
        fast,
        request(ids[0], Some("0.002")),
        &bus,
        Duration::from_secs(30),
    )
    .await;
    assert!(third.is_ok(), "{third:?}");
}
