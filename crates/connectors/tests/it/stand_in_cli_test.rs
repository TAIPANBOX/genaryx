//! The clients that shell a plane's CLI (TokenFuse, Qryx, idryx), driven
//! against stand-in scripts that print a fixed answer and exit as told: the
//! arguments and environment each client hands the tool, what it makes of a
//! well-formed answer, and that every failure the tool can produce (a nonzero
//! exit, an exit 0 that still reports an error, output it cannot read) comes
//! back as that failure and never as an empty success. idryx's HTTP reads are
//! held here too, against the loopback stub.

use std::path::{Path, PathBuf};

use genaryx_connectors::{
    CostBreakdownRow, IdryxAlert, IdryxClient, IdryxError, QryxClient, QryxError, TokenfuseClient,
    TokenfuseError, TokenfuseSavings,
};

use crate::loopback::{closed_base, serve};
use crate::stand_in::{recorded_args, retry_busy, scratch_dir, script};

/// What a stand-in wrote beside itself (`"$0.<ext>"`).
fn sidecar(bin: &Path, ext: &str) -> PathBuf {
    PathBuf::from(format!("{}.{ext}", bin.display()))
}

// ---- TokenFuse --------------------------------------------------------------

const FOCUS_EXPORT: &str = r##"printf '%s\n' "$@" > "$0.args"
out=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--out" ]; then out="$a"; fi
  prev="$a"
done
printf 'BilledCost,ChargePeriodStart\n0.5,2026-11-01\n' > "$out"
"##;

#[test]
fn focus_export_hands_back_the_csv_the_tool_wrote_and_removes_its_file() {
    let dir = scratch_dir("focus");
    let bin = script(&dir, "tokenfuse", FOCUS_EXPORT);
    let traces = dir.join("traces");
    let client = TokenfuseClient::new(&bin);

    let csv = retry_busy(|| {
        client.focus_export(
            &traces,
            Some("2026-11-01T00:00:00Z"),
            Some("2026-11-02T00:00:00Z"),
        )
    })
    .expect("focus-export");
    assert_eq!(csv, b"BilledCost,ChargePeriodStart\n0.5,2026-11-01\n");
    let args = recorded_args(&sidecar(&bin, "args"));
    let traces_s = traces.display().to_string();
    assert_eq!(args.len(), 9, "{args:?}");
    assert_eq!(
        args[..4],
        ["focus-export", "--traces", traces_s.as_str(), "--out"]
    );
    assert_eq!(
        args[5..],
        ["--from", "2026-11-01T00:00:00Z", "--to", "2026-11-02T00:00:00Z"]
    );
    assert!(
        !Path::new(&args[4]).exists(),
        "the private output file is removed once read"
    );

    // No window: neither flag is passed at all.
    retry_busy(|| client.focus_export(&traces, None, None)).expect("unwindowed");
    assert_eq!(recorded_args(&sidecar(&bin, "args")).len(), 5);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_focus_export_that_fails_or_writes_nothing_is_an_error() {
    let dir = scratch_dir("focus-fail");
    let failing = script(&dir, "fails", "echo 'unknown flag --frm' >&2\nexit 3\n");
    match retry_busy(|| TokenfuseClient::new(&failing).focus_export(&dir, None, None)) {
        Err(TokenfuseError::Cli {
            command,
            code,
            stderr,
        }) => {
            assert_eq!((command, code), ("focus-export", 3));
            assert_eq!(stderr, "unknown flag --frm");
        }
        other => panic!("expected a Cli error, got {other:?}"),
    }

    let silent = script(&dir, "silent", "exit 0\n");
    match retry_busy(|| TokenfuseClient::new(&silent).focus_export(&dir, None, None)) {
        Err(TokenfuseError::Read { path, .. }) => {
            assert!(path.contains("genaryx-focus-"), "{path}");
        }
        other => panic!("an export that wrote no file must not read as empty, got {other:?}"),
    }
    std::fs::remove_dir_all(&dir).ok();
}

const SAVINGS: &str = r##"printf '%s\n' "$@" > "$0.args"
printf '%s\n' "$TOKENFUSE_DATA_DIR" > "$0.env"
cat <<'EOF'
TokenFuse savings - from the trace
  runaway spend stopped:   $3.500000   (3 blocked call(s) across 2 budget break(s))
  cache saved:             $0.750000
  router saved:            $0.100000
    budget_exceeded  $1.500000
    loop_detected    $2.000000
EOF
"##;

#[test]
fn savings_reads_the_trace_named_in_the_environment_and_parses_every_line() {
    let dir = scratch_dir("savings");
    let bin = script(&dir, "tokenfuse", SAVINGS);
    let traces = dir.join("gateway-traces");
    let got = retry_busy(|| TokenfuseClient::new(&bin).savings(&traces)).expect("savings");
    let mut by_reason = std::collections::BTreeMap::new();
    by_reason.insert("budget_exceeded".to_string(), 1_500_000);
    by_reason.insert("loop_detected".to_string(), 2_000_000);
    assert_eq!(
        got,
        TokenfuseSavings {
            trace_data_found: true,
            blocked_spend_microusd: 3_500_000,
            blocked_calls: 3,
            budget_breaks_prevented: 2,
            cache_saved_microusd: 750_000,
            router_saved_microusd: 100_000,
            by_reason_microusd: by_reason,
        }
    );
    assert_eq!(recorded_args(&sidecar(&bin, "args")), ["savings"]);
    assert_eq!(
        recorded_args(&sidecar(&bin, "env")),
        [traces.display().to_string()],
        "the trace directory travels as TOKENFUSE_DATA_DIR, not a flag"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn savings_that_fail_or_print_something_else_are_errors() {
    let dir = scratch_dir("savings-fail");
    let failing = script(&dir, "fails", "echo 'no data dir' >&2\nexit 2\n");
    match retry_busy(|| TokenfuseClient::new(&failing).savings(&dir)) {
        Err(TokenfuseError::Cli {
            command,
            code,
            stderr,
        }) => assert_eq!((command, code, stderr.as_str()), ("savings", 2, "no data dir")),
        other => panic!("expected a Cli error, got {other:?}"),
    }
    let chatty = script(&dir, "chatty", "echo 'Welcome to TokenFuse!'\n");
    assert!(matches!(
        retry_busy(|| TokenfuseClient::new(&chatty).savings(&dir)),
        Err(TokenfuseError::Parse {
            command: "savings",
            ..
        })
    ));
    std::fs::remove_dir_all(&dir).ok();
}

const COST_TABLES: &str = r##"printf '%s\n' "$TOKENFUSE_DATA_DIR" > "$0.env"
case "$2" in
  *"group by model"*) cat <<'EOF'
+-------+-------+
| model | calls | total_cost_microusd | total_tool_calls | tool_calls_known_rows |
+-------+-------+
| m-a   | 4     | 100                 | 5                | 4                     |
+-------+-------+
EOF
  ;;
  *) cat <<'EOF'
| agent_id                 | calls | total_cost_microusd | total_tool_calls | tool_calls_known_rows |
| agent://acme.example/bot | 4     | 100                 | 0                | 0                     |
EOF
  ;;
esac
"##;

#[test]
fn cost_per_action_runs_both_fixed_queries_and_keys_each_table_by_its_label() {
    let dir = scratch_dir("cost");
    let bin = script(&dir, "tokenfuse", COST_TABLES);
    let report = retry_busy(|| TokenfuseClient::new(&bin).cost_per_action(&dir)).expect("cost");
    assert_eq!(
        report.by_model,
        [CostBreakdownRow {
            label: "m-a".into(),
            calls: 4,
            total_cost_microusd: 100,
            total_tool_calls: 5,
            tool_calls_known_rows: 4,
            cost_per_tool_call_microusd: Some(20),
        }]
    );
    assert_eq!(
        report.by_agent,
        [CostBreakdownRow {
            label: "agent://acme.example/bot".into(),
            calls: 4,
            total_cost_microusd: 100,
            total_tool_calls: 0,
            tool_calls_known_rows: 0,
            cost_per_tool_call_microusd: None,
        }],
        "no known tool calls is no rate, never a rate of zero"
    );
    assert_eq!(
        recorded_args(&sidecar(&bin, "env")),
        [dir.display().to_string()]
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_sql_error_on_stderr_is_a_failure_even_when_the_tool_exits_zero() {
    let dir = scratch_dir("cost-fail");
    let quirk = script(&dir, "quirk", "echo 'sql error: no table calls' >&2\nexit 0\n");
    match retry_busy(|| TokenfuseClient::new(&quirk).cost_per_action(&dir)) {
        Err(TokenfuseError::Cli {
            command,
            code,
            stderr,
        }) => assert_eq!(
            (command, code, stderr.as_str()),
            ("sql", 0, "sql error: no table calls")
        ),
        other => panic!("an exit-0 sql error must not read as zero rows, got {other:?}"),
    }
    let crashed = script(&dir, "crashed", "echo partial\necho crash >&2\nexit 101\n");
    match retry_busy(|| TokenfuseClient::new(&crashed).cost_per_action(&dir)) {
        Err(TokenfuseError::Cli {
            command,
            code,
            stderr,
        }) => assert_eq!((command, code, stderr.as_str()), ("sql", 101, "crash")),
        other => panic!("expected a Cli error, got {other:?}"),
    }
    // A readable but empty trace prints an empty table: zero rows, no error.
    let empty = script(&dir, "empty", "printf '++\\n++\\n'\n");
    let report = retry_busy(|| TokenfuseClient::new(&empty).cost_per_action(&dir)).expect("empty");
    assert!(report.by_model.is_empty() && report.by_agent.is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

// ---- Qryx -------------------------------------------------------------------

const QRYX: &str = r##"printf '%s\n' "$@" > "$0.args"
case "$3" in
  ncsc) cat <<'EOF'
{"standard":"NCSC PQC migration timeline (2028/2031/2035)","generatedAt":"2026-11-01T00:00:00Z","root":"/src","discovery2028":{"verdict":"at-risk","coverageBySource":{"code":3},"totalInventoried":3,"quantumVulnerableCount":2,"migrationPlanExists":false,"quantumVulnerableFindings":null},"highestPriority2031":{"verdict":"not-started","count":1,"migratedCount":0,"remainingCount":1,"findings":[{"algorithm":"RSA-2048","type":"public-key","severity":"high","occurrences":2,"locations":["a.go:1"],"externallyFacing":true,"longLivedData":false,"planned":false}]},"fullMigration2035":{"verdict":"not-started","count":2,"findings":null}}
EOF
  ;;
  cbom) cat <<'EOF'
{"bomFormat":"CycloneDX","specVersion":"1.6","components":[]}
EOF
  ;;
  evidence) cat <<'EOF'
{"tool":"qryx","version":"1.2.3","standard":"CNSA 2.0","generatedAt":"2026-11-01T00:00:00Z","root":"/src","summary":{"compliant":3,"nonCompliant":1,"issues":1,"total":4,"scorePct":75,"bySeverity":{"high":1}},"assets":[{"name":"a"}],"digest":"sha256:00ff","signature":{"alg":"ml-dsa-65","value":"c2ln","publicKey":"cGs="}}
EOF
  ;;
esac
"##;

#[test]
fn qryx_puts_its_flags_before_the_target_and_parses_every_format() {
    let dir = scratch_dir("qryx");
    let bin = script(&dir, "qryx", QRYX);
    let q = QryxClient::new(&bin);
    let target = Path::new("/src");
    let key = Path::new("/keys/mldsa.pem");
    let args = || recorded_args(&sidecar(&bin, "args"));

    let ncsc = retry_busy(|| q.scan_ncsc(target)).expect("ncsc");
    assert_eq!(args(), ["scan", "--format", "ncsc", "/src"]);
    assert_eq!(ncsc.discovery_2028.verdict, "at-risk");
    assert!(
        ncsc.discovery_2028.quantum_vulnerable_findings.is_empty(),
        "a Go null list reads as empty"
    );
    assert_eq!(ncsc.highest_priority_2031.findings[0].asset_type, "public-key");
    assert!(ncsc.full_migration_2035.findings.is_empty());

    let cbom = retry_busy(|| q.scan_cbom(target)).expect("cbom");
    assert_eq!(cbom["specVersion"], "1.6");
    let raw = retry_busy(|| q.scan_cbom_raw(target)).expect("cbom raw");
    assert_eq!(
        raw,
        b"{\"bomFormat\":\"CycloneDX\",\"specVersion\":\"1.6\",\"components\":[]}\n",
        "the raw form is the tool's bytes, unparsed"
    );

    let signed = retry_busy(|| q.scan_evidence(target, Some(key))).expect("evidence");
    assert_eq!(
        args(),
        ["scan", "--format", "evidence", "--sign-key", "/keys/mldsa.pem", "/src"]
    );
    assert_eq!(signed.summary.score_pct, 75);
    assert_eq!(
        signed.signature.as_ref().map(|s| s.alg.as_str()),
        Some("ml-dsa-65")
    );
    retry_busy(|| q.scan_evidence(target, None)).expect("unsigned evidence");
    assert_eq!(args(), ["scan", "--format", "evidence", "/src"]);

    let raw = retry_busy(|| q.scan_evidence_raw(target, Some(key))).expect("raw evidence");
    assert!(raw.starts_with(b"{\"tool\":\"qryx\""));
    assert_eq!(args()[3], "--sign-key");
    retry_busy(|| q.scan_evidence_raw(target, None)).expect("raw unsigned");
    assert_eq!(args(), ["scan", "--format", "evidence", "/src"]);

    let agents = retry_busy(|| q.agents_evidence(target)).expect("agents");
    assert_eq!(args(), ["agents", "--format", "evidence", "/src"]);
    assert_eq!(agents.digest, "sha256:00ff");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_qryx_failure_or_unreadable_output_is_an_error_never_an_empty_report() {
    let dir = scratch_dir("qryx-fail");
    let failing = script(&dir, "fails", "echo 'scan root missing' >&2\nexit 1\n");
    match retry_busy(|| QryxClient::new(&failing).scan_ncsc(Path::new("/nope"))) {
        Err(QryxError::Cli { code, stderr }) => {
            assert_eq!((code, stderr.as_str()), (1, "scan root missing"));
        }
        other => panic!("expected a Cli error, got {other:?}"),
    }
    assert!(matches!(
        retry_busy(|| QryxClient::new(&failing).scan_cbom_raw(Path::new("/nope"))),
        Err(QryxError::Cli { code: 1, .. })
    ));
    let garbage = script(&dir, "garbage", "echo 'not json'\n");
    assert!(matches!(
        retry_busy(|| QryxClient::new(&garbage).scan_evidence(Path::new("/src"), None)),
        Err(QryxError::Json(_))
    ));
    assert!(matches!(
        QryxClient::new(dir.join("absent")).scan_cbom(Path::new("/src")),
        Err(QryxError::Spawn { .. })
    ));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn verify_evidence_reads_the_exit_code_as_the_verdict() {
    let dir = scratch_dir("qryx-verify");
    let bin = script(
        &dir,
        "qryx",
        "if [ \"$2\" = \"/good.json\" ]; then echo 'evidence: VERIFIED (ml-dsa-65)'; exit 0; fi\n\
         echo 'evidence: digest mismatch' >&2\nexit 1\n",
    );
    let q = QryxClient::new(&bin);
    let good = retry_busy(|| q.verify_evidence(Path::new("/good.json"))).expect("verdict");
    assert!(good.verified);
    assert_eq!(good.message, "evidence: VERIFIED (ml-dsa-65)");
    let bad = retry_busy(|| q.verify_evidence(Path::new("/bad.json"))).expect("verdict");
    assert!(!bad.verified, "a nonzero exit is a 'not verified' answer");
    assert_eq!(
        bad.message, "evidence: digest mismatch",
        "the reason comes from stderr when stdout is empty"
    );
    std::fs::remove_dir_all(&dir).ok();
}

// ---- idryx --------------------------------------------------------------------

const IDRYX: &str = r##"printf '%s\n' "$@" > "$0.args"
if [ "$1" = "detect" ]; then cat <<'EOF'
[{"detector":"privileged_unused","identity":"agent://acme.example/bot","severity":"high","time":"2026-11-01T00:00:00Z","summary":"admin never used"}]
EOF
exit 0
fi
printf '{"bomFormat":"CycloneDX"}'
"##;

#[test]
fn idryx_detect_and_bom_pass_every_load_and_read_the_answer() {
    let dir = scratch_dir("idryx");
    let bin = script(&dir, "idryx", IDRYX);
    let loads = [
        ("tokenfuse", "/bus/tokenfuse.ndjson"),
        ("wardryx", "/bus/wardryx.ndjson"),
    ];
    let alerts = retry_busy(|| IdryxClient::rescan(&bin, &loads, "medium")).expect("detect");
    assert_eq!(
        alerts,
        [IdryxAlert {
            detector: "privileged_unused".into(),
            identity: "agent://acme.example/bot".into(),
            severity: "high".into(),
            time: "2026-11-01T00:00:00Z".into(),
            summary: "admin never used".into(),
        }]
    );
    assert_eq!(
        recorded_args(&sidecar(&bin, "args")),
        [
            "detect",
            "--format",
            "json",
            "--min-severity",
            "medium",
            "--load",
            "tokenfuse:/bus/tokenfuse.ndjson",
            "--load",
            "wardryx:/bus/wardryx.ndjson",
        ]
    );

    let bom = retry_busy(|| IdryxClient::agent_bom(&bin, &loads[..1])).expect("bom");
    assert_eq!(bom, b"{\"bomFormat\":\"CycloneDX\"}");
    assert_eq!(
        recorded_args(&sidecar(&bin, "args")),
        ["bom", "--format", "json", "--load", "tokenfuse:/bus/tokenfuse.ndjson"]
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_idryx_cli_failure_names_the_subcommand_and_its_stderr() {
    let dir = scratch_dir("idryx-fail");
    let failing = script(&dir, "fails", "echo 'no bus' >&2\nexit 4\n");
    for (r, sub) in [
        (
            retry_busy(|| IdryxClient::rescan(&failing, &[], "low").map(|_| ())),
            "`idryx detect` exited",
        ),
        (
            retry_busy(|| IdryxClient::agent_bom(&failing, &[]).map(|_| ())),
            "`idryx bom` exited",
        ),
    ] {
        match r {
            Err(IdryxError::Cli(m)) => {
                assert!(m.starts_with(sub), "{m}");
                assert!(m.ends_with("no bus"), "{m}");
            }
            other => panic!("expected a Cli error, got {other:?}"),
        }
    }
    let garbage = script(&dir, "garbage", "echo '{not json'\n");
    assert!(matches!(
        retry_busy(|| IdryxClient::rescan(&garbage, &[], "low")),
        Err(IdryxError::Json(_))
    ));
    match IdryxClient::agent_bom(&dir.join("absent"), &[]) {
        Err(IdryxError::Cli(m)) => assert!(m.starts_with("spawn "), "{m}"),
        other => panic!("expected a spawn failure, got {other:?}"),
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn idryx_http_reads_parse_the_server_shapes_and_refuse_everything_else() {
    let stub = serve(vec![
        (
            "GET /api/identities",
            "200 OK",
            r#"[{"id":"agent://acme.example/bot","type":"agent","privileged":true,"source":"tokenfuse","owner":"user://acme.example/pat","events":3,"alerts":1,"permissions":[{"name":"s3:*","admin":true,"used":false}],"remediation":{"kind":"scope_down","explanation":"unused admin","code":"deny s3:*"}}]"#
                .to_string(),
        ),
        (
            "GET /api/alerts",
            "200 OK",
            r#"[{"detector":"d","identity":"i","severity":"low","time":"t","summary":"s"}]"#
                .to_string(),
        ),
        (
            "GET /api/remediations",
            "200 OK",
            r#"[{"identity":"i","kind":"rotate","explanation":"old key","code":"rotate it"}]"#
                .to_string(),
        ),
        ("GET /healthz", "200 OK", "ok".to_string()),
    ]);
    let c = IdryxClient::new(format!("{}/", stub.base)).expect("client");
    let ids = c.list_identities().await.expect("identities");
    assert_eq!(ids[0].identity_type, "agent");
    assert!(ids[0].permissions[0].admin && !ids[0].permissions[0].used);
    assert_eq!(
        ids[0].remediation.as_ref().map(|r| r.kind.as_str()),
        Some("scope_down")
    );
    assert!(ids[0].rotation.is_none() && ids[0].on_behalf_of.is_empty());
    assert_eq!(c.list_alerts().await.expect("alerts")[0].severity, "low");
    assert_eq!(
        c.list_remediations().await.expect("remediations")[0].created_at,
        "",
        "an absent created_at defaults to empty"
    );
    assert!(c.healthz().await.expect("healthz"));
    let targets: Vec<String> = stub.requests().into_iter().map(|r| r.target).collect();
    assert_eq!(
        targets,
        ["/api/identities", "/api/alerts", "/api/remediations", "/healthz"]
    );

    let sick = serve(vec![
        ("GET /healthz", "503 Service Unavailable", String::new()),
        (
            "GET /api/identities",
            "500 Internal Server Error",
            "db locked".to_string(),
        ),
        ("GET /api/alerts", "200 OK", "not json".to_string()),
    ]);
    let c = IdryxClient::new(sick.base.clone()).expect("client");
    assert!(!c.healthz().await.expect("a 503 is an answer: unhealthy"));
    match c.list_identities().await {
        Err(IdryxError::Api { status, body }) => {
            assert_eq!((status, body.as_str()), (500, "db locked"));
        }
        other => panic!("expected Api 500, got {other:?}"),
    }
    assert!(matches!(c.list_alerts().await, Err(IdryxError::Json(_))));
    assert!(matches!(
        IdryxClient::new(closed_base())
            .expect("client")
            .list_remediations()
            .await,
        Err(IdryxError::Transport(_))
    ));
}
