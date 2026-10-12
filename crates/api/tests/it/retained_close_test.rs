//! D8, the console side: an operator lists the gateway's retained
//! reservations and closes one with evidence, by minting a short-lived token
//! the gateway verifies (TokenFuse `crates/gateway/src/closure.rs`).
//!
//! Every gateway here is a HAND-ROLLED stub on an ephemeral port, the same
//! shape `delegation_revoke_test.rs` uses: this proves GENARYX's own client
//! (the token it mints, the headers and body it sends, how it reads each
//! answer, what it journals), not the gateway's correctness, which is
//! TokenFuse's own suite's job.
//!
//! Nothing in this file writes the process environment: the secret is handed
//! in through `ClosureConfig::from_secret_file`, a path being a value.

use genaryx_api::credentials::env::EnvSource;
use genaryx_api::credentials::state::{CredentialsClient, CredentialsInner, CredentialsState};
use genaryx_api::money::closure::{
    ClosureConfig, MAX_TOKEN_TTL_MILLIS, MIN_SECRET_BYTES, TOKEN_TTL_MILLIS, mint_token,
};
use genaryx_api::money::retained::{
    CloseRetainedArgs, EvidenceKind, RetainedError, money_close_retained, money_list_retained,
};
use genaryx_api::money::state::BusHandle;
use genaryx_connectors::GatewayClient;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The vector TokenFuse's `closure.rs` and this console must both mint.
const VECTOR_SECRET: &str = "0123456789abcdef0123456789abcdef";
const VECTOR_TOKEN: &str = "v1.7.0011223344556677.1760000000000.ea4d5ef61542d974dc20dbd83b439baad4f781863d7e48bfc0e08480d29beced";

const SECRET: &str = "closure-test-secret-0123456789abcdef-unmistakable";
const ADMIN_KEY: &str = "sk-gateway-admin-key-for-closure-tests";

// ---------------------------------------------------------------------------
// stub gateway
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Captured {
    method: String,
    path: String,
    authorization: Option<String>,
    closure_token: Option<String>,
    body: Value,
}

fn serve_one(stream: TcpStream, status: u16, body: &[u8]) -> Captured {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut request_line = String::new();
    reader.read_line(&mut request_line).expect("request line");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    let mut content_length = 0usize;
    let mut authorization = None;
    let mut closure_token = None;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("header line");
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            match name.as_str() {
                "content-length" => content_length = value.parse().unwrap_or(0),
                "authorization" => authorization = Some(value),
                "x-fuse-closure-token" => closure_token = Some(value),
                _ => {}
            }
        }
    }
    let mut raw = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut raw).expect("body");
    }
    let parsed = serde_json::from_slice(&raw).unwrap_or(Value::Null);

    let mut out = reader.into_inner();
    write!(
        out,
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("head");
    out.write_all(body).expect("body");
    out.flush().ok();
    Captured {
        method,
        path,
        authorization,
        closure_token,
        body: parsed,
    }
}

/// A stub that answers every connection it receives with `status`/`body`,
/// recording each request and counting connections.
struct Stub {
    url: String,
    captured: Arc<Mutex<Vec<Captured>>>,
    connections: Arc<AtomicUsize>,
}

fn spawn_stub(status: u16, body: Value) -> Stub {
    spawn_stub_raw(status, serde_json::to_vec(&body).unwrap())
}

fn spawn_stub_raw(status: u16, body: Vec<u8>) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}", listener.local_addr().unwrap());
    let captured = Arc::new(Mutex::new(Vec::new()));
    let connections = Arc::new(AtomicUsize::new(0));
    let (c, n) = (Arc::clone(&captured), Arc::clone(&connections));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            n.fetch_add(1, Ordering::SeqCst);
            let req = serve_one(stream, status, &body);
            c.lock().unwrap().push(req);
        }
    });
    Stub {
        url,
        captured,
        connections,
    }
}

fn unreachable_url() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    format!("http://{addr}")
}

fn unique_dir(tag: &str) -> std::path::PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!(
        "genaryx-retained-itest-{tag}-{}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("scratch dir");
    d
}

fn scratch_bus(tag: &str) -> BusHandle {
    let d = unique_dir(&format!("bus-{tag}"));
    BusHandle::from_dirs(&d, &d)
}

fn gateway_at(url: &str) -> CredentialsState {
    let client = GatewayClient::new(url)
        .expect("client")
        .with_admin_key(Some(ADMIN_KEY.to_string()));
    let state = CredentialsState::pending();
    *state.inner.try_lock().expect("fresh state") = CredentialsInner::Ready(CredentialsClient {
        client: Arc::new(client),
        source: EnvSource::Taipan {
            name: "test".to_string(),
        },
        gateway_url: url.to_string(),
    });
    state
}

fn config_with(secret: &str) -> ClosureConfig {
    let d = unique_dir("secret");
    let p = d.join("closure.secret");
    std::fs::write(&p, format!("{secret}\n")).unwrap();
    ClosureConfig::from_secret_file(Some(p.to_str().unwrap())).expect("a 32-byte+ secret resolves")
}

fn observed_args() -> CloseRetainedArgs {
    CloseRetainedArgs {
        reservation_id: 7,
        expected_revision: "0011223344556677".to_string(),
        evidence_kind: EvidenceKind::Observed,
        observed_microusd: Some(4_210),
        evidence_ref: "invoice INV-2026-10 line 14".to_string(),
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn journaled_line(bus: &BusHandle) -> Value {
    let body = std::fs::read_to_string(&bus.console_events_path).expect("a journaled line");
    let line = body.lines().last().expect("at least one line");
    serde_json::from_str(line).expect("line parses")
}

fn journaled_row(bus: &BusHandle) -> (String, String, String, Value, i64, String) {
    let conn = rusqlite::Connection::open(&bus.store_db_path).expect("store");
    conn.query_row(
        "SELECT operator, action, target, params, http_status, verify_result \
         FROM commands_journal ORDER BY id DESC LIMIT 1",
        [],
        |r| {
            let params: String = r.get(3)?;
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                serde_json::from_str(&params).unwrap_or(Value::Null),
                r.get(4)?,
                r.get(5)?,
            ))
        },
    )
    .expect("a journal row")
}

fn closed_body(op: &str) -> Value {
    json!({
        "closed": true, "replayed": false, "reservation_id": 7,
        "revision": "0011223344556677", "operation_id": op,
        "evidence_kind": "observed", "evidence_ref": "invoice INV-2026-10 line 14",
        "settled_microusd": 4210, "run_id": "run-a", "unit": null,
        "closed_at_millis": 1_760_000_000_000_i64
    })
}

// ---------------------------------------------------------------------------
// the token
// ---------------------------------------------------------------------------

#[test]
fn the_closure_token_matches_the_gateway_test_vector() {
    assert_eq!(
        mint_token(
            VECTOR_SECRET.as_bytes(),
            7,
            "0011223344556677",
            1_760_000_000_000
        ),
        VECTOR_TOKEN
    );
}

#[test]
fn the_token_lifetime_is_two_minutes_and_never_over_five() {
    assert_eq!(TOKEN_TTL_MILLIS, 120_000);
    assert_eq!(MAX_TOKEN_TTL_MILLIS, 300_000);
    const { assert!(TOKEN_TTL_MILLIS <= MAX_TOKEN_TTL_MILLIS) };
    assert_eq!(MIN_SECRET_BYTES, 32);
}

// ---------------------------------------------------------------------------
// configuration
// ---------------------------------------------------------------------------

#[test]
fn an_unset_secret_file_is_not_configured() {
    assert!(matches!(
        ClosureConfig::from_secret_file(None),
        Ok(ClosureConfig::NotConfigured)
    ));
    assert!(matches!(
        ClosureConfig::from_secret_file(Some("  ")),
        Ok(ClosureConfig::NotConfigured)
    ));
}

#[test]
fn a_secret_shorter_than_32_bytes_refuses_to_start() {
    let d = unique_dir("short");
    let p = d.join("s");
    std::fs::write(&p, "0123456789abcdef0123456789abcde\n").unwrap(); // 31 bytes
    let err = ClosureConfig::from_secret_file(Some(p.to_str().unwrap()))
        .expect_err("31 bytes must refuse");
    assert!(
        err.contains("GENARYX_CLOSURE_SECRET_FILE") && err.contains("32"),
        "the refusal names the variable and the minimum: {err}"
    );
    assert!(
        !err.contains("0123456789abcdef0123456789abcde"),
        "the refusal never prints the secret: {err}"
    );

    std::fs::write(&p, VECTOR_SECRET).unwrap(); // exactly 32
    assert!(matches!(
        ClosureConfig::from_secret_file(Some(p.to_str().unwrap())),
        Ok(ClosureConfig::Configured { .. })
    ));
}

#[test]
fn an_unreadable_or_empty_secret_file_refuses_to_start() {
    let d = unique_dir("unreadable");
    let missing = d.join("nope");
    let err = ClosureConfig::from_secret_file(Some(missing.to_str().unwrap()))
        .expect_err("a missing file must refuse");
    assert!(err.contains("GENARYX_CLOSURE_SECRET_FILE"), "{err}");

    let empty = d.join("empty");
    std::fs::write(&empty, "\n").unwrap();
    assert!(ClosureConfig::from_secret_file(Some(empty.to_str().unwrap())).is_err());
}

#[test]
fn the_secret_never_appears_in_debug_output() {
    let cfg = config_with(SECRET);
    let dbg = format!("{cfg:?}");
    assert!(!dbg.contains(SECRET), "{dbg}");
}

// ---------------------------------------------------------------------------
// money_list_retained
// ---------------------------------------------------------------------------

#[tokio::test]
async fn listing_reads_get_v1_retained_with_the_admin_key() {
    let stub = spawn_stub(
        200,
        json!([{
            "reservation_id": 7, "revision": "0011223344556677", "run_id": "run-a",
            "chain": ["run-a"], "amount_microusd": 8657, "unit": "finance",
            "model": "gpt-x", "retained_at_millis": 1_760_000_000_000_i64, "future": 1
        }]),
    );
    let rows = money_list_retained(&gateway_at(&stub.url))
        .await
        .expect("a 200 list parses");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].reservation_id, 7);
    assert_eq!(rows[0].revision, "0011223344556677");
    assert_eq!(rows[0].amount_microusd, 8657);
    assert_eq!(rows[0].unit.as_deref(), Some("finance"));

    let req = stub.captured.lock().unwrap()[0].clone();
    assert_eq!(req.method, "GET");
    assert_eq!(req.path, "/v1/retained");
    assert_eq!(
        req.authorization.as_deref(),
        Some(format!("Bearer {ADMIN_KEY}").as_str())
    );
}

#[tokio::test]
async fn a_gateway_with_the_closure_route_off_says_so_when_listing() {
    // TokenFuse registers GET /v1/retained only when TOKENFUSE_CLOSURE_SECRET
    // is set; otherwise the route does not exist and axum answers a bare 404.
    let stub = spawn_stub_raw(404, Vec::new());
    let err = money_list_retained(&gateway_at(&stub.url))
        .await
        .expect_err("a 404 is not an empty list");
    assert!(matches!(err, RetainedError::ClosureOff), "{err:?}");
}

#[tokio::test]
async fn listing_with_no_gateway_says_so_rather_than_an_empty_list() {
    let err = money_list_retained(&CredentialsState::pending())
        .await
        .expect_err("a bootstrapping plane is not an empty list");
    assert!(matches!(err, RetainedError::Bootstrapping), "{err:?}");
}

// ---------------------------------------------------------------------------
// money_close_retained: what reaches the gateway
// ---------------------------------------------------------------------------

#[tokio::test]
async fn closing_posts_the_evidence_with_a_fresh_token_bound_to_the_reservation() {
    let stub = spawn_stub(200, closed_body("whatever"));
    let bus = scratch_bus("happy");
    let before = now_millis();
    let out = money_close_retained(
        observed_args(),
        &config_with(SECRET),
        &gateway_at(&stub.url),
        Some(&bus),
    )
    .await
    .expect("a confirmed 200 is success");
    let after = now_millis();

    assert_eq!(out.http_status, 200);
    assert!(!out.replayed);
    assert_eq!(out.settled_microusd, Some(4210));

    let req = stub.captured.lock().unwrap()[0].clone();
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/v1/retained/close");
    assert_eq!(
        req.authorization.as_deref(),
        Some(format!("Bearer {ADMIN_KEY}").as_str()),
        "the admin key rides as the bearer, the token in its own header"
    );

    // The body is exactly the gateway's CloseRequest (deny_unknown_fields
    // there, so a stray member would be a 400).
    let body = req.body.as_object().expect("a JSON object");
    let mut keys: Vec<&str> = body.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "evidence_kind",
            "evidence_ref",
            "expected_revision",
            "observed_microusd",
            "operation_id",
            "reservation_id"
        ]
    );
    assert_eq!(body["reservation_id"], 7);
    assert_eq!(body["expected_revision"], "0011223344556677");
    assert_eq!(body["evidence_kind"], "observed");
    assert_eq!(body["observed_microusd"], 4210);
    assert_eq!(body["evidence_ref"], "invoice INV-2026-10 line 14");
    let op = body["operation_id"].as_str().expect("an operation id");
    assert_eq!(op, out.operation_id, "the console generates and returns it");
    assert!(
        !op.is_empty() && op.len() <= 128 && op.bytes().all(|b| b.is_ascii_graphic()),
        "within the gateway's operation_id bounds: {op}"
    );

    // The token: five parts, bound to this id and revision, expiring two
    // minutes from when it was minted, and signed with the shared secret.
    let token = req.closure_token.expect("x-fuse-closure-token is sent");
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 5, "{token}");
    assert_eq!(parts[0], "v1");
    assert_eq!(parts[1], "7");
    assert_eq!(parts[2], "0011223344556677");
    let exp: i64 = parts[3].parse().expect("decimal expiry");
    assert!(
        exp >= before + TOKEN_TTL_MILLIS && exp <= after + TOKEN_TTL_MILLIS,
        "expiry {exp} is two minutes after minting ({before}..{after})"
    );
    assert_eq!(
        token,
        mint_token(SECRET.as_bytes(), 7, "0011223344556677", exp),
        "signed with the configured secret"
    );
}

#[tokio::test]
async fn a_no_charge_closure_sends_no_observed_amount() {
    let stub = spawn_stub(200, closed_body("x"));
    let args = CloseRetainedArgs {
        evidence_kind: EvidenceKind::NoCharge,
        observed_microusd: None,
        ..observed_args()
    };
    money_close_retained(
        args,
        &config_with(SECRET),
        &gateway_at(&stub.url),
        Some(&scratch_bus("nocharge")),
    )
    .await
    .expect("success");
    let req = stub.captured.lock().unwrap()[0].clone();
    assert_eq!(req.body["evidence_kind"], "no_charge");
    assert!(req.body["observed_microusd"].is_null(), "{}", req.body);
}

// ---------------------------------------------------------------------------
// money_close_retained: what the operator and the journal are told
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_closure_is_journaled_with_the_acting_principal_and_its_result() {
    let stub = spawn_stub(200, closed_body("x"));
    let bus = scratch_bus("journal");
    let out = genaryx_api::console_actor::with_actor(Some("alice".to_string()), async {
        money_close_retained(
            observed_args(),
            &config_with(SECRET),
            &gateway_at(&stub.url),
            Some(&bus),
        )
        .await
    })
    .await
    .expect("success");
    assert!(out.bus_recorded, "{:?}", out.bus_error);

    let line = journaled_line(&bus);
    assert_eq!(line["type"], "console_command");
    assert_eq!(line["data"]["action"], "console.close_retained");
    assert_eq!(line["data"]["target"], "reservation:7");
    assert_eq!(line["data"]["http_status"], 200);
    assert_eq!(line["data"]["verify_result"], "closed");
    let who = line["on_behalf_of"][0].as_str().expect("a principal");
    assert!(who.ends_with("/alice"), "the chain names who closed it: {who}");

    let (operator, action, target, params, status, verify) = journaled_row(&bus);
    assert!(operator.ends_with("/alice"), "{operator}");
    assert_eq!(action, "console.close_retained");
    assert_eq!(target, "reservation:7");
    assert_eq!(status, 200);
    assert_eq!(verify, "closed");
    assert_eq!(params["operation_id"], out.operation_id.as_str());
    assert_eq!(params["expected_revision"], "0011223344556677");
    assert_eq!(params["evidence_kind"], "observed");
    assert_eq!(params["observed_microusd"], 4210);
    assert_eq!(params["evidence_ref"], "invoice INV-2026-10 line 14");
}

#[tokio::test]
async fn a_stale_revision_is_refused_and_journaled_as_the_gateways_real_409() {
    let stub = spawn_stub(
        409,
        json!({"type":"error","error":{"type":"stale_revision","message":"reservation 7 is at revision 1111111111111111, not 0011223344556677"}}),
    );
    let bus = scratch_bus("stale");
    let err = money_close_retained(
        observed_args(),
        &config_with(SECRET),
        &gateway_at(&stub.url),
        Some(&bus),
    )
    .await
    .expect_err("a 409 is never success");
    match err {
        RetainedError::Refused { status, error, .. } => {
            assert_eq!(status, 409);
            assert_eq!(error, "stale_revision");
        }
        other => panic!("expected Refused, got {other:?}"),
    }
    let line = journaled_line(&bus);
    assert_eq!(line["data"]["http_status"], 409);
    assert_eq!(line["data"]["verify_result"], "refused:stale_revision");
}

#[tokio::test]
async fn a_200_that_does_not_confirm_the_closure_is_never_success() {
    for body in [json!({}), json!({"closed": false}), json!("<html>ok</html>")] {
        let stub = spawn_stub(200, body.clone());
        let bus = scratch_bus("unconfirmed");
        let err = money_close_retained(
            observed_args(),
            &config_with(SECRET),
            &gateway_at(&stub.url),
            Some(&bus),
        )
        .await
        .expect_err("an unconfirmed 200 is not success");
        assert!(
            matches!(err, RetainedError::Unconfirmed { status: 200, .. }),
            "{body}: {err:?}"
        );
        let line = journaled_line(&bus);
        assert_ne!(line["data"]["verify_result"], "closed", "{body}");
    }
}

#[tokio::test]
async fn a_gateway_with_the_closure_route_off_is_reported_as_such() {
    let stub = spawn_stub_raw(404, Vec::new());
    let err = money_close_retained(
        observed_args(),
        &config_with(SECRET),
        &gateway_at(&stub.url),
        Some(&scratch_bus("off")),
    )
    .await
    .expect_err("404");
    assert!(matches!(err, RetainedError::ClosureOff), "{err:?}");
}

#[tokio::test]
async fn an_unreachable_gateway_is_journaled_with_status_zero() {
    let bus = scratch_bus("unreachable");
    let err = money_close_retained(
        observed_args(),
        &config_with(SECRET),
        &gateway_at(&unreachable_url()),
        Some(&bus),
    )
    .await
    .expect_err("unreachable");
    assert!(matches!(err, RetainedError::Unreachable { .. }), "{err:?}");
    assert_eq!(journaled_line(&bus)["data"]["http_status"], 0);
}

#[tokio::test]
async fn an_unconfigured_console_answers_not_configured_and_calls_nobody() {
    let stub = spawn_stub(200, closed_body("x"));
    let bus = scratch_bus("notconfigured");
    let err = money_close_retained(
        observed_args(),
        &ClosureConfig::NotConfigured,
        &gateway_at(&stub.url),
        Some(&bus),
    )
    .await
    .expect_err("not configured");
    assert!(matches!(err, RetainedError::NotConfigured), "{err:?}");
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(stub.connections.load(Ordering::SeqCst), 0);
    let line = journaled_line(&bus);
    assert_eq!(line["data"]["http_status"], 0);
    assert_eq!(line["data"]["verify_result"], "refused:not_configured");
}

#[tokio::test]
async fn evidence_that_does_not_fit_its_kind_is_refused_before_any_call() {
    let stub = spawn_stub(200, closed_body("x"));
    let bad = [
        CloseRetainedArgs {
            observed_microusd: None,
            ..observed_args()
        },
        CloseRetainedArgs {
            observed_microusd: Some(-1),
            ..observed_args()
        },
        CloseRetainedArgs {
            evidence_kind: EvidenceKind::NoCharge,
            observed_microusd: Some(5),
            ..observed_args()
        },
        CloseRetainedArgs {
            evidence_ref: "   ".to_string(),
            ..observed_args()
        },
        CloseRetainedArgs {
            evidence_ref: "line\none".to_string(),
            ..observed_args()
        },
        CloseRetainedArgs {
            evidence_ref: "x".repeat(513),
            ..observed_args()
        },
        CloseRetainedArgs {
            expected_revision: "00112233445566AB".to_string(),
            ..observed_args()
        },
        CloseRetainedArgs {
            expected_revision: "001122".to_string(),
            ..observed_args()
        },
    ];
    for args in bad {
        let shown = format!("{args:?}");
        let err = money_close_retained(
            args,
            &config_with(SECRET),
            &gateway_at(&stub.url),
            Some(&scratch_bus("invalid")),
        )
        .await
        .expect_err("invalid evidence must be refused");
        assert!(
            matches!(err, RetainedError::InvalidArgs { .. }),
            "{shown}: {err:?}"
        );
    }
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(stub.connections.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_secret_and_the_token_never_appear_in_the_journal_or_the_answer() {
    for (status, body) in [
        (200u16, closed_body("x")),
        (
            401,
            json!({"type":"error","error":{"type":"closure_token_invalid","message":"no"}}),
        ),
        (
            409,
            json!({"type":"error","error":{"type":"already_closed","message":"no"}}),
        ),
    ] {
        let stub = spawn_stub(status, body);
        let bus = scratch_bus("leak");
        let result = money_close_retained(
            observed_args(),
            &config_with(SECRET),
            &gateway_at(&stub.url),
            Some(&bus),
        )
        .await;
        let token = stub.captured.lock().unwrap()[0]
            .closure_token
            .clone()
            .expect("token sent");
        let sig = token.rsplit('.').next().unwrap().to_string();

        let shown = format!("{result:?}");
        let line = std::fs::read_to_string(&bus.console_events_path).unwrap_or_default();
        let (_, _, _, params, _, verify) = journaled_row(&bus);
        let row = format!("{params} {verify}");
        for (what, needle) in [("secret", SECRET), ("token signature", sig.as_str())] {
            assert!(!shown.contains(needle), "{status}: {what} in the answer");
            assert!(!line.contains(needle), "{status}: {what} in the bus line");
            assert!(!row.contains(needle), "{status}: {what} in the journal row");
        }
    }
}

// ---------------------------------------------------------------------------
// Felyx stays unable to sign
// ---------------------------------------------------------------------------

#[test]
fn the_copilot_crate_cannot_reach_the_closure_secret_or_mint_a_token() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../copilot");
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("copilot manifest");
    for dep in ["genaryx-api", "hmac"] {
        assert!(
            !manifest
                .lines()
                .any(|l| l.trim_start().starts_with(&format!("{dep} "))
                    || l.trim_start().starts_with(&format!("{dep}="))),
            "genaryx-copilot must not depend on {dep}: the closure mint lives in genaryx-api"
        );
    }
    let mut stack = vec![root.join("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read copilot src") {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let src = std::fs::read_to_string(&path).unwrap();
                for needle in [
                    "money_close_retained",
                    "x-fuse-closure-token",
                    "GENARYX_CLOSURE_SECRET_FILE",
                    "retained/close",
                ] {
                    assert!(
                        !src.contains(needle),
                        "{} names {needle}: Felyx proposes, it never closes",
                        path.display()
                    );
                }
            }
        }
    }
}
