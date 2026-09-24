//! Integration proof for `delegation::commands::delegation_revoke` against a
//! HAND-ROLLED stub vouchryx, never a real Go process: this module tests
//! GENARYX's own HTTP client (the header, the body shape, how each status
//! code is interpreted), not vouchryx's own correctness, which is vouchryx's
//! own T3 suite's job. The brief that asked for this file names the shape
//! directly: "a stub vouchryx records it".
//!
//! Every test spawns its own stub on an ephemeral port and tears it down at
//! the end of the function (the accept thread exits after serving the one
//! request each scenario needs, or never receives one at all for the
//! validation-refused / not-configured / unreachable cases).

use genaryx_api::delegation::commands::delegation_revoke;
use genaryx_api::delegation::env::RevokeConfig;
use genaryx_api::money::state::BusHandle;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Guards every `GENARYX_VOUCHRYX_*` env mutation in this file. `cargo test`
/// runs one binary's tests on parallel threads by default, and the pair of
/// variables `resolve_from_env()` reads is process-wide state: without this,
/// two tests calling `configured()` (or touching the vars directly)
/// concurrently can interleave their set/read/clear cycles, so one test
/// resolves a URL or key file that belongs to another. Measured, not
/// assumed: `cargo test -p genaryx-api --test delegation_revoke_test` alone
/// (this binary only, less contention) passed all 17 tests; the same suite
/// inside `cargo test --workspace` (every binary's threads contending at
/// once) failed three of them until this lock was added.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// One request the stub actually received.
#[derive(Debug, Clone)]
struct CapturedRequest {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Value,
}

/// Read one HTTP/1.1 request off `stream` (request line, headers up to the
/// blank line, then exactly `Content-Length` body bytes - every request this
/// module ever sends is a short `POST` with a JSON body, so nothing more is
/// needed) and answer it with `status`/`body`.
fn serve_one(stream: TcpStream, status: u16, body: &Value) -> CapturedRequest {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));

    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .expect("read request line");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    let mut content_length: usize = 0;
    let mut authorization = None;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read header line");
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if name == "content-length" {
                content_length = value.parse().unwrap_or(0);
            } else if name == "authorization" {
                authorization = Some(value);
            }
        }
    }

    let mut raw_body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut raw_body).expect("read body");
    }
    let parsed_body: Value = if raw_body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&raw_body).unwrap_or(Value::Null)
    };

    let body_bytes = serde_json::to_vec(body).expect("serialize canned response");
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        503 => "Service Unavailable",
        _ => "Unknown",
    };
    let mut out = reader.into_inner();
    write!(
        out,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body_bytes.len()
    )
    .expect("write response head");
    out.write_all(&body_bytes).expect("write response body");
    out.flush().ok();

    CapturedRequest {
        method,
        path,
        authorization,
        body: parsed_body,
    }
}

/// Spawn a stub bound to `127.0.0.1:0` that answers exactly one connection
/// with `status`/`body`, and hand back its base URL plus a slot the
/// captured request lands in once the exchange completes.
fn spawn_stub(status: u16, body: Value) -> (String, Arc<Mutex<Option<CapturedRequest>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub listener");
    let addr = listener.local_addr().expect("stub local addr");
    let captured = Arc::new(Mutex::new(None));
    let captured_thread = Arc::clone(&captured);
    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            let req = serve_one(stream, status, &body);
            *captured_thread.lock().expect("captured lock") = Some(req);
        }
    });
    (format!("http://{addr}"), captured)
}

/// A stub that must NEVER be connected to: used by every "refused before any
/// call" test. Counts connection attempts instead of asserting from inside
/// the accept thread, so a violation fails the actual test assertion rather
/// than only printing from a background thread nobody reads.
fn spawn_must_not_be_called() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub listener");
    let addr = listener.local_addr().expect("stub local addr");
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_thread = Arc::clone(&hits);
    std::thread::spawn(move || {
        listener.set_nonblocking(false).expect("blocking listener");
        for stream in listener.incoming().take(1).flatten() {
            hits_thread.fetch_add(1, Ordering::SeqCst);
            // Answer something so a caller that DID connect (a bug) does not
            // hang waiting for a response and blow the test timeout.
            let _ = serve_one(
                stream,
                500,
                &json!({"error":"should never have been called"}),
            );
        }
    });
    (format!("http://{addr}"), hits)
}

/// A URL nothing listens on: bind, read the assigned port back, then drop
/// the listener immediately, so a connection attempt gets ECONNREFUSED at
/// once rather than waiting out the real 5s timeout.
fn unreachable_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind to free a port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    format!("http://{addr}")
}

/// A scratch directory unique across every thread this binary's tests run
/// on, EVEN within the same process id and the same clock tick: `pid` +
/// `SystemTime::now()` alone collided under `cargo test --workspace`'s
/// heavier thread contention (measured: two tests' `configured()` calls
/// landed on the same nanosecond, so the second one's key file was deleted
/// by the first one's cleanup mid-flight, "No such file or directory").
/// `env.rs`'s own `unique_path` and `money/env.rs`'s `unique_dir` already
/// add a per-process `AtomicU64` for exactly this reason; this mirrors them.
fn unique_dir(tag: &str) -> std::path::PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "genaryx-delegation-itest-{tag}-{}-{}-{n}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ))
}

fn configured(url: &str) -> RevokeConfig {
    // `RevokeConfig` has no public constructor beyond `resolve_from_env`
    // (deliberately - see its module doc), so tests build it the same way
    // every other plane's tests build config: through a real temp key file
    // and the env-resolution function itself.
    let dir = unique_dir("cfg");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let key_path = dir.join("revoke.key");
    std::fs::write(&key_path, "test-vouchryx-bearer-key-0123456789").expect("write key file");
    // `env.rs`'s own unit tests avoid the process env entirely by testing the
    // pure core directly; this integration file has to go through the real
    // env because it is proving `main.rs`'s actual startup path end to end,
    // which means the set/resolve/clear cycle below has to be one atomic
    // section - see ENV_LOCK's own doc for why (measured, not assumed).
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::set_var("GENARYX_VOUCHRYX_URL", url);
        std::env::set_var(
            "GENARYX_VOUCHRYX_REVOKE_KEY_FILE",
            key_path.to_str().unwrap(),
        );
    }
    let cfg = genaryx_api::delegation::env::resolve_from_env().expect("must resolve Configured");
    unsafe {
        std::env::remove_var("GENARYX_VOUCHRYX_URL");
        std::env::remove_var("GENARYX_VOUCHRYX_REVOKE_KEY_FILE");
    }
    let _ = std::fs::remove_dir_all(&dir);
    cfg
}

fn scratch_bus(tag: &str) -> BusHandle {
    let dir = unique_dir(&format!("bus-{tag}"));
    std::fs::create_dir_all(&dir).expect("create bus scratch dir");
    BusHandle::from_dirs(&dir, &dir)
}

// ---------------------------------------------------------------------------
// the command posts exactly the body and the bearer key vouchryx expects
// ---------------------------------------------------------------------------

#[tokio::test]
async fn posts_exactly_the_body_and_bearer_key_for_a_subject_revocation() {
    let (url, captured) = spawn_stub(200, json!({"revoked": true, "expires": 1_800_000_000}));
    let cfg = configured(&url);
    let bus = scratch_bus("subject-body");

    let outcome = delegation_revoke(
        Some("agent://acme.example/bot/compromised".to_string()),
        None,
        "credentials leaked in a paste".to_string(),
        &cfg,
        Some(&bus),
    )
    .await
    .expect("a 200 from vouchryx must be Ok");
    assert_eq!(outcome.http_status, 200);

    let req = captured
        .lock()
        .unwrap()
        .take()
        .expect("stub must have been called exactly once");
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/v1/revoke");
    assert_eq!(
        req.authorization.as_deref(),
        Some("Bearer test-vouchryx-bearer-key-0123456789")
    );
    assert_eq!(
        req.body,
        json!({
            "jti": "",
            "subject": "agent://acme.example/bot/compromised",
            "actor": req.body["actor"],
            "reason": "credentials leaked in a paste",
        }),
        "the body must carry exactly jti/subject/actor/reason, subject populated and jti empty"
    );
    assert!(
        req.body["actor"].as_str().is_some_and(|a| !a.is_empty()),
        "actor must be a real, non-empty principal: {:?}",
        req.body["actor"]
    );
}

#[tokio::test]
async fn posts_exactly_the_body_for_a_jti_revocation() {
    let (url, captured) = spawn_stub(200, json!({"revoked": true, "expires": 1_800_000_000}));
    let cfg = configured(&url);
    let bus = scratch_bus("jti-body");

    delegation_revoke(
        None,
        Some("jti-abc-123".to_string()),
        "one leaked token".to_string(),
        &cfg,
        Some(&bus),
    )
    .await
    .expect("a 200 from vouchryx must be Ok");

    let req = captured
        .lock()
        .unwrap()
        .take()
        .expect("stub must have been called");
    assert_eq!(req.body["jti"], "jti-abc-123");
    assert_eq!(req.body["subject"], "");
    assert_eq!(req.body["reason"], "one leaked token");
}

// ---------------------------------------------------------------------------
// every vouchryx outcome reaches the operator and the journal as itself
// ---------------------------------------------------------------------------

#[tokio::test]
async fn vouchryx_200_is_success_and_journals_it_as_200() {
    let (url, _captured) = spawn_stub(200, json!({"revoked": true, "expires": 42}));
    let cfg = configured(&url);
    let bus = scratch_bus("200");

    let outcome = delegation_revoke(
        Some("agent://acme.example/bot/a".to_string()),
        None,
        "why".to_string(),
        &cfg,
        Some(&bus),
    )
    .await
    .expect("200 must be Ok");
    assert_eq!(outcome.http_status, 200);
    assert_eq!(outcome.verify_result, "revoked:true");
    assert_eq!(
        outcome.vouchryx_response,
        json!({"revoked": true, "expires": 42})
    );
    assert!(outcome.bus_recorded, "bus_error: {:?}", outcome.bus_error);

    assert_journaled_status(&bus, 200);
}

#[tokio::test]
async fn vouchryx_401_is_refused_never_success() {
    let (url, _c) = spawn_stub(401, json!({"error": "invalid_client"}));
    let cfg = configured(&url);
    let bus = scratch_bus("401");

    let err = delegation_revoke(
        Some("agent://acme.example/bot/a".to_string()),
        None,
        "why".to_string(),
        &cfg,
        Some(&bus),
    )
    .await
    .expect_err("401 must be Err, never Ok/success");
    assert_eq!(
        err,
        genaryx_api::delegation::commands::DelegationError::Refused {
            status: 401,
            error: "invalid_client".to_string()
        }
    );
    assert_journaled_status(&bus, 401);
}

#[tokio::test]
async fn vouchryx_403_is_refused_never_success() {
    let (url, _c) = spawn_stub(403, json!({"error": "access_denied"}));
    let cfg = configured(&url);
    let bus = scratch_bus("403");

    let err = delegation_revoke(
        Some("agent://acme.example/bot/a".to_string()),
        None,
        "why".to_string(),
        &cfg,
        Some(&bus),
    )
    .await
    .expect_err("403 must be Err, never Ok/success");
    assert_eq!(
        err,
        genaryx_api::delegation::commands::DelegationError::Refused {
            status: 403,
            error: "access_denied".to_string()
        }
    );
    assert_journaled_status(&bus, 403);
}

#[tokio::test]
async fn vouchryx_400_is_refused_never_success() {
    let (url, _c) = spawn_stub(400, json!({"error": "invalid_request"}));
    let cfg = configured(&url);
    let bus = scratch_bus("400");

    let err = delegation_revoke(
        None,
        Some("jti-x".to_string()),
        "why".to_string(),
        &cfg,
        Some(&bus),
    )
    .await
    .expect_err("400 must be Err, never Ok/success");
    assert_eq!(
        err,
        genaryx_api::delegation::commands::DelegationError::Refused {
            status: 400,
            error: "invalid_request".to_string()
        }
    );
    assert_journaled_status(&bus, 400);
}

#[tokio::test]
async fn vouchryx_503_is_reported_as_not_durable_never_as_success() {
    // vouchryx's own `refuse()` sends the identical body whether its list is
    // full or the write to disk failed after accepting the revocation in
    // memory (internal/api/api.go, read at origin/main 19b5211) - this
    // console cannot tell which happened and must not claim it can.
    let (url, _c) = spawn_stub(503, json!({"error": "temporarily_unavailable"}));
    let cfg = configured(&url);
    let bus = scratch_bus("503");

    let err = delegation_revoke(
        Some("agent://acme.example/bot/a".to_string()),
        None,
        "why".to_string(),
        &cfg,
        Some(&bus),
    )
    .await
    .expect_err("503 must be Err, never Ok/success");
    assert_eq!(
        err,
        genaryx_api::delegation::commands::DelegationError::NotDurable
    );
    assert_journaled_status(&bus, 503);

    let body = std::fs::read_to_string(&bus.console_events_path).unwrap();
    let line = body.lines().next_back().unwrap();
    let v: Value = serde_json::from_str(line).unwrap();
    assert_eq!(
        v.pointer("/data/verify_result").and_then(|s| s.as_str()),
        Some("not_durable"),
        "the journal must say not_durable, never a success shape: {line}"
    );
}

#[tokio::test]
async fn vouchryx_unreachable_is_reported_and_never_success() {
    let cfg = configured(&unreachable_url());
    let bus = scratch_bus("unreachable");

    let err = delegation_revoke(
        Some("agent://acme.example/bot/a".to_string()),
        None,
        "why".to_string(),
        &cfg,
        Some(&bus),
    )
    .await
    .expect_err("no answer must be Err, never Ok/success");
    assert!(
        matches!(
            err,
            genaryx_api::delegation::commands::DelegationError::Unreachable { .. }
        ),
        "{err:?}"
    );
    assert_journaled_status(&bus, 0);
}

/// The journaled line's `data.http_status` is exactly `expected` - the real
/// status vouchryx answered with (or 0 for unreachable), never a fabricated
/// 200. Reads the LAST line so callers that already appended one (the 503
/// test) still find their own.
fn assert_journaled_status(bus: &BusHandle, expected: u64) {
    let body = std::fs::read_to_string(&bus.console_events_path).expect("read journaled events");
    let line = body
        .lines()
        .next_back()
        .expect("at least one journaled line");
    let v: Value = serde_json::from_str(line).expect("journaled line parses");
    assert_eq!(
        v.pointer("/data/http_status").and_then(|s| s.as_u64()),
        Some(expected),
        "line: {line}"
    );
    let conformer = genaryx_core::Conformer::new().expect("embedded schemas compile");
    let report = conformer.check_line(line);
    assert!(report.valid, "{:?}\n{line}", report.errors);
}

// ---------------------------------------------------------------------------
// configuration: not configured, and one variable without the other
// ---------------------------------------------------------------------------

#[tokio::test]
async fn not_configured_answers_the_named_refusal_and_calls_nobody() {
    let bus = scratch_bus("not-configured");
    let (_url, hits) = spawn_must_not_be_called();

    let result = tokio::time::timeout(
        Duration::from_secs(2),
        delegation_revoke(
            Some("agent://acme.example/bot/a".to_string()),
            None,
            "why".to_string(),
            &RevokeConfig::NotConfigured,
            Some(&bus),
        ),
    )
    .await
    .expect("must answer fast: no network call means no reason to wait");

    assert_eq!(
        result.unwrap_err(),
        genaryx_api::delegation::commands::DelegationError::NotConfigured
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "vouchryx must never be called"
    );
}

#[test]
fn one_variable_without_the_other_refuses_to_start() {
    // The two-variable interaction is `env.rs`'s own unit-tested core
    // (`url_without_the_key_file_refuses_to_start` and its mirror); this
    // proves the SAME real `resolve_from_env()` this process's `main.rs`
    // calls at boot sees it too, end to end through the process environment.
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::set_var("GENARYX_VOUCHRYX_URL", "http://127.0.0.1:1");
        std::env::remove_var("GENARYX_VOUCHRYX_REVOKE_KEY_FILE");
    }
    let err = genaryx_api::delegation::env::resolve_from_env().unwrap_err();
    unsafe {
        std::env::remove_var("GENARYX_VOUCHRYX_URL");
    }
    assert!(err.contains("GENARYX_VOUCHRYX_URL"));
    assert!(err.contains("GENARYX_VOUCHRYX_REVOKE_KEY_FILE"));
}

// ---------------------------------------------------------------------------
// argument validation is refused before any call
// ---------------------------------------------------------------------------

#[tokio::test]
async fn both_target_forms_together_are_refused_before_any_call() {
    let bus = scratch_bus("both-targets");
    let (url, hits) = spawn_must_not_be_called();
    let cfg = configured(&url);

    let err = delegation_revoke(
        Some("agent://acme.example/bot/a".to_string()),
        Some("jti-1".to_string()),
        "why".to_string(),
        &cfg,
        Some(&bus),
    )
    .await
    .unwrap_err();
    assert_eq!(
        err,
        genaryx_api::delegation::commands::DelegationError::ExactlyOneTarget
    );
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn neither_target_form_is_refused_before_any_call() {
    let bus = scratch_bus("neither-target");
    let (url, hits) = spawn_must_not_be_called();
    let cfg = configured(&url);

    let err = delegation_revoke(None, None, "why".to_string(), &cfg, Some(&bus))
        .await
        .unwrap_err();
    assert_eq!(
        err,
        genaryx_api::delegation::commands::DelegationError::ExactlyOneTarget
    );
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_subject_with_the_wrong_scheme_is_refused_before_any_call() {
    let bus = scratch_bus("bad-scheme");
    let (url, hits) = spawn_must_not_be_called();
    let cfg = configured(&url);

    let err = delegation_revoke(
        Some("bot://acme.example/a".to_string()),
        None,
        "why".to_string(),
        &cfg,
        Some(&bus),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        genaryx_api::delegation::commands::DelegationError::InvalidSubject { .. }
    ));
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_empty_reason_is_refused_before_any_call() {
    let bus = scratch_bus("empty-reason");
    let (url, hits) = spawn_must_not_be_called();
    let cfg = configured(&url);

    let err = delegation_revoke(
        Some("agent://acme.example/bot/a".to_string()),
        None,
        "   ".to_string(),
        &cfg,
        Some(&bus),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        genaryx_api::delegation::commands::DelegationError::InvalidReason { .. }
    ));
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_oversized_reason_is_refused_before_any_call() {
    let bus = scratch_bus("oversized-reason");
    let (url, hits) = spawn_must_not_be_called();
    let cfg = configured(&url);

    let err = delegation_revoke(
        Some("agent://acme.example/bot/a".to_string()),
        None,
        "x".repeat(501),
        &cfg,
        Some(&bus),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        genaryx_api::delegation::commands::DelegationError::InvalidReason { .. }
    ));
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

// ---------------------------------------------------------------------------
// the key never appears anywhere observable
// ---------------------------------------------------------------------------

/// The bearer key touches source code in exactly one place outside `env.rs`
/// itself: the `.bearer_auth(revoke_key.as_str())` call in `commands.rs`'s
/// `call_vouchryx`. A structural check on the source, the same
/// `include_str!` idiom `roles.rs`'s own completeness test uses on
/// `dispatch.rs` - a string that is never read out of `RevokeKey` cannot
/// leak into a log line, an error, or a response, by construction.
#[test]
fn the_key_is_read_out_of_revoke_key_in_exactly_one_place() {
    const SOURCE: &str = include_str!("../src/delegation/commands.rs");
    let call_sites = SOURCE.matches("revoke_key.as_str()").count();
    assert_eq!(
        call_sites, 1,
        "revoke_key.as_str() must appear exactly once (the Authorization header); \
         found {call_sites} in commands.rs"
    );
    // And nothing in commands.rs ever formats a RevokeKey with `{:?}` either
    // (which would still be redacted by `env.rs`'s own Debug impl, but the
    // absence of any attempt is the stronger claim).
    assert!(
        !SOURCE.contains("revoke_key:?") && !SOURCE.contains("{revoke_key}"),
        "commands.rs must never format revoke_key directly"
    );
}

#[tokio::test]
async fn the_key_never_appears_in_the_journaled_line_or_the_returned_error() {
    const SECRET: &str = "unmistakable-secret-bytes-0123456789";
    let dir = unique_dir("key-leak");
    std::fs::create_dir_all(&dir).expect("create dir");
    let key_path = dir.join("revoke.key");
    std::fs::write(&key_path, SECRET).unwrap();

    // Run every outcome (success, each refusal, unreachable) and check each
    // one's journaled line and returned error for the secret's bytes.
    for (status, body) in [
        (200u16, json!({"revoked": true, "expires": 1})),
        (401, json!({"error": "invalid_client"})),
        (400, json!({"error": "invalid_request"})),
        (503, json!({"error": "temporarily_unavailable"})),
    ] {
        let (url, _c) = spawn_stub(status, body);
        let cfg = {
            let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            unsafe {
                std::env::set_var("GENARYX_VOUCHRYX_URL", &url);
                std::env::set_var(
                    "GENARYX_VOUCHRYX_REVOKE_KEY_FILE",
                    key_path.to_str().unwrap(),
                );
            }
            let cfg = genaryx_api::delegation::env::resolve_from_env().unwrap();
            unsafe {
                std::env::remove_var("GENARYX_VOUCHRYX_URL");
                std::env::remove_var("GENARYX_VOUCHRYX_REVOKE_KEY_FILE");
            }
            cfg
        };
        let bus = scratch_bus(&format!("key-leak-{status}"));

        let result = delegation_revoke(
            Some("agent://acme.example/bot/a".to_string()),
            None,
            "checking for leaks".to_string(),
            &cfg,
            Some(&bus),
        )
        .await;

        let err_repr = format!("{result:?}");
        assert!(
            !err_repr.contains(SECRET),
            "status {status}: the key leaked into the returned value: {err_repr}"
        );

        let journaled = std::fs::read_to_string(&bus.console_events_path).unwrap_or_default();
        assert!(
            !journaled.contains(SECRET),
            "status {status}: the key leaked into the journaled bus line: {journaled}"
        );
    }
    // The unreachable path too, with the SAME secret key file (so this is a
    // real check on SECRET, not on `configured()`'s own unrelated key).
    let cfg = {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var("GENARYX_VOUCHRYX_URL", unreachable_url());
            std::env::set_var(
                "GENARYX_VOUCHRYX_REVOKE_KEY_FILE",
                key_path.to_str().unwrap(),
            );
        }
        let cfg = genaryx_api::delegation::env::resolve_from_env().unwrap();
        unsafe {
            std::env::remove_var("GENARYX_VOUCHRYX_URL");
            std::env::remove_var("GENARYX_VOUCHRYX_REVOKE_KEY_FILE");
        }
        cfg
    };
    let bus = scratch_bus("key-leak-unreachable");
    let result = delegation_revoke(
        Some("agent://acme.example/bot/a".to_string()),
        None,
        "checking for leaks".to_string(),
        &cfg,
        Some(&bus),
    )
    .await;
    let err_repr = format!("{result:?}");
    assert!(
        !err_repr.contains(SECRET),
        "unreachable case: the key leaked into the returned value: {err_repr}"
    );
    let journaled = std::fs::read_to_string(&bus.console_events_path).unwrap_or_default();
    assert!(
        !journaled.contains(SECRET),
        "unreachable case: the key leaked into the journaled bus line: {journaled}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
