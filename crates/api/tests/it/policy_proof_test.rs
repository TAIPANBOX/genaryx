//! The policy proof, with a stand-in `wardryx` that prints a report and records
//! how it was run.
//!
//! The parser and the two halves are held in `crates/api/src/proof/tests.rs`.
//! What is held here is the process around them: the exact arguments, the
//! empty environment, the temporary candidate and its removal, the deadline,
//! and each way a proof is refused rather than invented.

use genaryx_api::bus::{AppState, BusMode};
use genaryx_api::proof::commands::{
    CANDIDATE_MAX_BYTES, ProofConfig, ProofError, ProofRequest, prove,
};
use genaryx_core::store::Store;
use genaryx_core::{AgentEvent, ConsoleEvent, Provenance, SchemaVersion};
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SCHEMA: &str = "taipanbox.dev/agent-event/v0.2";
const REPORT: &str = "Replayed 3 decision(s) from /e/wardryx.ndjson

  reproduced             3
  approval-decided       0
  approval-spent         0
  approval-refused       0
  not archived           0
  unreadable             0
  diverged               0

Against /tmp/candidate.yaml, 2 of 3 replayable decision(s) change:

  allow -> deny   agent://a/one  run r-incident
        was: allowed: request satisfies all matched policy rules
        now: tool \"shell\" is denied by policy \"p1\"

  deny -> allow   agent://a/two  run r-elsewhere
        was: tool \"x\" is denied
        now: allowed: request satisfies all matched policy rules
";
const DIVERGED_REPORT: &str = "Replayed 2 decision(s) from /e/wardryx.ndjson

  reproduced             1
  approval-decided       0
  approval-spent         0
  approval-refused       0
  not archived           0
  unreadable             0
  diverged               1   replaying their OWN version disagrees with the record

DIVERGED rows mean the record and this build no longer agree about the
past. Nothing below is trustworthy until that is explained:
  agent://a/one run r-incident: replayed deny, recorded allow

Against /tmp/candidate.yaml, 0 of 1 replayable decision(s) change:
  none.
";

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "genaryx-proof-test-{tag}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create_dir_all");
    dir
}

/// A `wardryx` that records its arguments, the environment it was given and a
/// copy of the candidate it was pointed at, then prints `stdout` and `stderr`
/// and exits with `code` after `sleep` seconds.
fn fake_wardryx(dir: &Path, stdout: &str, stderr: &str, code: i32, sleep: u32) -> PathBuf {
    std::fs::write(dir.join("out.txt"), stdout).expect("out");
    std::fs::write(dir.join("err.txt"), stderr).expect("err");
    let d = dir.display();
    let script = format!(
        "#!/bin/sh\n\
         printf '%s\\n' \"$@\" > '{d}/args.txt'\n\
         env | sort > '{d}/env.txt'\n\
         prev=''\n\
         for a in \"$@\"; do\n\
           if [ \"$prev\" = '-policy' ]; then cp \"$a\" '{d}/candidate.copy'; echo \"$a\" > '{d}/candidate.path'; fi\n\
           prev=\"$a\"\n\
         done\n\
         cat '{d}/out.txt'\n\
         cat '{d}/err.txt' 1>&2\n\
         if [ {sleep} -gt 0 ]; then sleep {sleep}; fi\n\
         exit {code}\n"
    );
    let bin = dir.join("wardryx");
    std::fs::write(&bin, script).expect("script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    bin
}

fn event(n: u64, run: Option<&str>) -> ConsoleEvent {
    ConsoleEvent {
        event: AgentEvent {
            schema: SCHEMA.to_string(),
            ts: "2026-10-08T10:00:00Z".to_string(),
            source: "wardryx".to_string(),
            event_type: "policy_allow".to_string(),
            agent_id: "agent://a/one".to_string(),
            severity: Some("info".to_string()),
            run_id: run.map(str::to_string),
            on_behalf_of: Vec::new(),
            data: Some(json!({})),
            prev_hash: None,
            extra: serde_json::Map::new(),
        },
        provenance: Provenance {
            env: "test".to_string(),
            connector: "fixture".to_string(),
            file: Some("wardryx.ndjson".to_string()),
            offset: Some(n),
            endpoint: None,
            received_ts: "2026-11-04T00:00:00Z".to_string(),
        },
        raw: json!({ "n": n }).to_string(),
        schema_version: SchemaVersion::from_schema_str(SCHEMA).expect("a known schema"),
    }
}

/// A bus with two events, the first in `r-incident` and the second in no run;
/// returns the bus and the ids of those two rows.
fn bus(dir: &Path) -> (AppState, Vec<i64>) {
    let store = Store::open(&dir.join("console.sqlite")).expect("Store::open");
    store
        .insert_batch(&[event(1, Some("r-incident")), event(2, None)])
        .expect("insert_batch");
    let ids: Vec<i64> = store
        .recent_events(10)
        .expect("recent_events")
        .into_iter()
        .rev()
        .map(|e| e.id)
        .collect();
    drop(store);
    let bus = AppState {
        events_dir: Some(dir.to_path_buf()),
        source_events_dir: Some(dir.to_path_buf()),
        mode: BusMode::Live {
            env: "test".to_string(),
            dir: dir.display().to_string(),
        },
    };
    (bus, ids)
}

fn config(dir: &Path, bin: PathBuf) -> ProofConfig {
    ProofConfig {
        wardryx_bin: bin,
        archive: Some(dir.join("archive")),
        events: Some(dir.join("wardryx.ndjson")),
    }
}

/// A freshly written script can be refused once as "text file busy" while a
/// sibling test forks; that is the test's race, not the proof's, so it is
/// tried again.
fn prove_again(
    config: &ProofConfig,
    bus: &AppState,
    request: &ProofRequest,
    timeout: Duration,
) -> Result<genaryx_api::proof::PolicyProof, ProofError> {
    for _ in 0..5 {
        match prove(config, bus, request, timeout) {
            Err(ProofError::Spawn { detail }) if detail.text.to_lowercase().contains("busy") => {
                std::thread::sleep(Duration::from_millis(100));
            }
            other => return other,
        }
    }
    prove(config, bus, request, timeout)
}

fn request(id: i64, candidate: &str) -> ProofRequest {
    ProofRequest {
        incident_id: format!("bus:{id}"),
        candidate: candidate.to_string(),
    }
}

const CANDIDATE: &str = "- name: deny-shell\n  target: agent://a/*\n  deny_tool: [shell]\n";

#[test]
fn the_proof_runs_wardryx_replay_with_exactly_its_arguments_and_removes_the_candidate() {
    let dir = temp_dir("args");
    let bin = fake_wardryx(&dir, REPORT, "", 0, 0);
    let (bus, ids) = bus(&dir);
    let config = config(&dir, bin);
    prove_again(
        &config,
        &bus,
        &request(ids[0], CANDIDATE),
        Duration::from_secs(10),
    )
    .expect("a proof");

    let args = std::fs::read_to_string(dir.join("args.txt")).expect("the arguments");
    let args: Vec<&str> = args.lines().collect();
    let candidate_path = std::fs::read_to_string(dir.join("candidate.path"))
        .expect("the candidate path")
        .trim()
        .to_string();
    assert_eq!(
        args,
        vec![
            "replay",
            "-events",
            dir.join("wardryx.ndjson").to_str().expect("utf8"),
            "-archive",
            dir.join("archive").to_str().expect("utf8"),
            "-policy",
            candidate_path.as_str(),
        ],
        "no shell, no other flag, the files the console chose"
    );
    assert!(candidate_path.ends_with("candidate.yaml"));
    assert_eq!(
        std::fs::read_to_string(dir.join("candidate.copy")).expect("a copy"),
        CANDIDATE,
        "wardryx read the operator's text"
    );
    assert!(
        !Path::new(&candidate_path).exists(),
        "the candidate is removed when the run ends"
    );
    assert!(
        !Path::new(&candidate_path)
            .parent()
            .is_some_and(Path::exists),
        "and so is its directory"
    );
}

#[test]
fn the_child_gets_no_environment_but_a_path() {
    let dir = temp_dir("env");
    let bin = fake_wardryx(&dir, REPORT, "", 0, 0);
    let (bus, ids) = bus(&dir);
    prove_again(
        &config(&dir, bin),
        &bus,
        &request(ids[0], CANDIDATE),
        Duration::from_secs(10),
    )
    .expect("a proof");
    let env = std::fs::read_to_string(dir.join("env.txt")).expect("the environment");
    assert!(env.contains("PATH="), "{env}");
    for leaked in ["HOME=", "USER=", "CARGO", "WARDRYX", "GENARYX", "TOKENFUSE"] {
        assert!(!env.contains(leaked), "the child saw {leaked}: {env}");
    }
}

#[test]
fn a_proof_reports_both_halves_for_an_incident_in_a_real_store() {
    let dir = temp_dir("halves");
    let bin = fake_wardryx(&dir, REPORT, "", 0, 0);
    let (bus, ids) = bus(&dir);
    let proof = prove_again(
        &config(&dir, bin),
        &bus,
        &request(ids[0], CANDIDATE),
        Duration::from_secs(10),
    )
    .expect("a proof");

    assert!(proof.trustworthy);
    assert_eq!(proof.stops_this_incident, Some(true));
    assert_eq!(proof.incident_changes.len(), 1);
    assert_eq!(proof.incident_changes[0].run.text, "r-incident");
    assert_eq!(proof.elsewhere.loosened, 1);
    assert_eq!(proof.elsewhere.tightened, 0);
    assert_eq!(proof.elsewhere_changes[0].run.text, "r-elsewhere");
    assert_eq!(proof.outcomes.reproduced, 3);
}

#[test]
fn a_diverged_replay_exits_non_zero_and_is_still_reported_as_untrustworthy() {
    let dir = temp_dir("diverged");
    let bin = fake_wardryx(&dir, DIVERGED_REPORT, "", 1, 0);
    let (bus, ids) = bus(&dir);
    let proof = prove_again(
        &config(&dir, bin),
        &bus,
        &request(ids[0], CANDIDATE),
        Duration::from_secs(10),
    )
    .expect("a proof that says it cannot be relied on");
    assert!(!proof.trustworthy);
    assert_eq!(proof.stops_this_incident, None);
    assert_eq!(proof.outcomes.diverged, 1);
}

#[test]
fn an_unconfigured_proof_names_what_is_missing_and_is_never_a_proof() {
    let dir = temp_dir("unconfigured");
    let bin = fake_wardryx(&dir, REPORT, "", 0, 0);
    let (bus, ids) = bus(&dir);
    let mut no_archive = config(&dir, bin.clone());
    no_archive.archive = None;
    assert!(matches!(
        prove(&no_archive, &bus, &request(ids[0], CANDIDATE), Duration::from_secs(5)),
        Err(ProofError::NotConfigured { missing }) if missing.contains("WARDRYX_POLICY_ARCHIVE")
    ));
    let mut no_events = config(&dir, bin);
    no_events.events = None;
    assert!(matches!(
        prove(
            &no_events,
            &bus,
            &request(ids[0], CANDIDATE),
            Duration::from_secs(5)
        ),
        Err(ProofError::NotConfigured { .. })
    ));
    assert!(
        !dir.join("args.txt").exists(),
        "nothing was run for a proof that could not be made"
    );
}

#[test]
fn a_candidate_that_cannot_be_given_is_refused_before_anything_runs() {
    let dir = temp_dir("refused");
    let bin = fake_wardryx(&dir, REPORT, "", 0, 0);
    let (bus, ids) = bus(&dir);
    let config = config(&dir, bin);
    for candidate in [
        String::new(),
        "   \n".to_string(),
        "name: x\0".to_string(),
        "x".repeat(CANDIDATE_MAX_BYTES + 1),
    ] {
        assert!(
            matches!(
                prove(
                    &config,
                    &bus,
                    &request(ids[0], &candidate),
                    Duration::from_secs(5)
                ),
                Err(ProofError::CandidateRefused { .. })
            ),
            "{} bytes",
            candidate.len()
        );
    }
    assert!(!dir.join("args.txt").exists(), "wardryx was started");
}

#[test]
fn an_incident_with_no_run_or_no_row_is_refused() {
    let dir = temp_dir("incident");
    let bin = fake_wardryx(&dir, REPORT, "", 0, 0);
    let (bus, ids) = bus(&dir);
    let config = config(&dir, bin);
    assert_eq!(
        prove(
            &config,
            &bus,
            &request(ids[1], CANDIDATE),
            Duration::from_secs(5)
        )
        .err(),
        Some(ProofError::NoRun),
        "an event with no run has no run to ask about"
    );
    assert!(matches!(
        prove(
            &config,
            &bus,
            &request(ids[1] + 1000, CANDIDATE),
            Duration::from_secs(5)
        ),
        Err(ProofError::Incident { .. })
    ));
    assert!(matches!(
        prove(
            &config,
            &bus,
            &ProofRequest {
                incident_id: "money:inc-1".to_string(),
                candidate: CANDIDATE.to_string()
            },
            Duration::from_secs(5)
        ),
        Err(ProofError::Incident { .. })
    ));
}

#[test]
fn a_replay_that_hangs_is_killed_at_its_deadline() {
    let dir = temp_dir("hang");
    let bin = fake_wardryx(&dir, REPORT, "", 0, 20);
    let (bus, ids) = bus(&dir);
    let started = std::time::Instant::now();
    let result = prove_again(
        &config(&dir, bin),
        &bus,
        &request(ids[0], CANDIDATE),
        Duration::from_millis(400),
    );
    assert!(
        matches!(result, Err(ProofError::Timeout { .. })),
        "{result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the proof waited for the replay instead of stopping it"
    );
}

#[test]
fn a_replay_that_fails_without_a_report_says_so() {
    let dir = temp_dir("failed");
    let bin = fake_wardryx(
        &dir,
        "usage: wardryx replay\n",
        "policy archive unreadable",
        3,
        0,
    );
    let (bus, ids) = bus(&dir);
    let result = prove_again(
        &config(&dir, bin),
        &bus,
        &request(ids[0], CANDIDATE),
        Duration::from_secs(10),
    );
    match result {
        Err(ProofError::ReplayFailed { exit, detail }) => {
            assert_eq!(exit, Some(3));
            assert_eq!(detail.text, "policy archive unreadable");
        }
        other => panic!("expected the replay's own failure, got {other:?}"),
    }
}

#[test]
fn a_missing_binary_is_a_spawn_error() {
    let dir = temp_dir("missing");
    let (bus, ids) = bus(&dir);
    let config = config(&dir, dir.join("no-such-wardryx"));
    assert!(matches!(
        prove(
            &config,
            &bus,
            &request(ids[0], CANDIDATE),
            Duration::from_secs(5)
        ),
        Err(ProofError::Spawn { .. })
    ));
}
