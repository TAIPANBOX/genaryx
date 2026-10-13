//! Invariant 24: Felyx presents its own gateway client key as `x-fuse-key`.
//!
//! A TokenFuse gateway with client keys on (`TOKENFUSE_CLIENT_KEYS`) refuses
//! every metered call that carries no known `x-fuse-key` with a `401`, checked
//! before anything else about the call (tokenfuse v1.11.0,
//! `crates/gateway/src/proxy.rs`, `resolve_client_key`). Felyx calls that same
//! gateway and sent no such header, so with client keys on every question was
//! refused. These tests drive the real provider clients, and the service the
//! console builds from its configuration, against a hand-rolled stub, the same
//! shape `agent_id_header_test.rs` uses: they prove what GENARYX sends and
//! shows, never a real gateway's behaviour.
//!
//! The key is a secret, so two of them stand in for a hostile or misconfigured
//! gateway that echoes the request's headers into its error body: whatever it
//! sends back, the error Felyx hands the operator (the text `copilot_ask`
//! returns to the browser) carries a redaction marker, never the key.

use genaryx_copilot::provider::{AnthropicMessages, OpenAiCompat};
use genaryx_copilot::{
    ChatRequest, Clients, CopilotConfig, CopilotService, FUSE_KEY_REDACTED, FuseKey, LlmProvider,
    Message, ProviderError, ProviderKind,
};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The key the tests give Felyx. It holds `:` and `~`, both legal in a
/// tokenfuse client secret, so nothing on the way may split or escape it.
const KEY: &str = "sk-felyx-gateway-0123456789abcdef:x~y";
const AGENT: &str = "agent://acme.example/genaryx/felyx";
const ANTHROPIC_OK: &str =
    r#"{"content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":1,"output_tokens":1}}"#;
const OPENAI_OK: &str = r#"{"choices":[{"message":{"content":"ok"}}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;

/// Every header name lowercased, as in `agent_id_header_test.rs`.
type CapturedHeaders = HashMap<String, String>;
type Captured = Arc<Mutex<Vec<CapturedHeaders>>>;

/// What the stub answers.
#[derive(Clone, Copy)]
enum Reply {
    /// `200` with this body.
    Ok(&'static str),
    /// `401` whose body quotes whatever arrived in `x-fuse-key`.
    EchoTheKey,
}

/// Read one HTTP/1.1 request, record its headers BEFORE answering (so a test
/// that reads them right after the client returns never races the stub), then
/// answer as `reply` says.
fn serve_one(stream: TcpStream, reply: Reply, captured: &Captured) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
        return;
    }
    let mut headers = HashMap::new();
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            return;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if name == "content-length" {
                content_length = value.parse().unwrap_or(0);
            }
            headers.insert(name, value);
        }
    }
    let mut raw_body = vec![0u8; content_length];
    if content_length > 0 && reader.read_exact(&mut raw_body).is_err() {
        return;
    }

    let (status, body) = match reply {
        Reply::Ok(body) => ("200 OK", body.to_string()),
        Reply::EchoTheKey => (
            "401 Unauthorized",
            format!(
                r#"{{"error":{{"type":"unauthorized","you_sent":"{}"}}}}"#,
                headers.get("x-fuse-key").cloned().unwrap_or_default()
            ),
        ),
    };
    captured.lock().expect("lock captured").push(headers);

    let mut out = reader.into_inner();
    write!(
        out,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("write response head");
    out.write_all(body.as_bytes()).expect("write response body");
    out.flush().ok();
}

/// A stub on `127.0.0.1:0` answering up to eight connections the same way,
/// so a service that makes a second call (a revision turn) is still answered.
fn spawn_stub(reply: Reply) -> (String, Captured) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub listener");
    let addr = listener.local_addr().expect("stub local addr");
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    std::thread::spawn(move || {
        for stream in listener.incoming().take(8).flatten() {
            serve_one(stream, reply, &sink);
        }
    });
    (format!("http://{addr}"), captured)
}

/// The headers of the first request the stub received.
fn first(captured: &Captured) -> CapturedHeaders {
    captured
        .lock()
        .expect("lock captured")
        .first()
        .cloned()
        .expect("the stub received a request")
}

fn one_message_request() -> ChatRequest {
    ChatRequest {
        system: String::new(),
        messages: vec![Message::user("q")],
        tools: Vec::new(),
        max_tokens: 64,
        temperature: 0.0,
    }
}

fn key() -> FuseKey {
    FuseKey::new(KEY).expect("a printable key")
}

fn anthropic(base_url: String) -> AnthropicMessages {
    AnthropicMessages::new(
        base_url,
        "claude".into(),
        "provider-key".into(),
        false, // the stub is on 127.0.0.1, which the residency gate treats as local
        Vec::new(),
        "genaryx-copilot".into(),
        AGENT.into(),
    )
    .expect("local stub endpoint")
}

fn openai(base_url: String) -> OpenAiCompat {
    OpenAiCompat::new(
        ProviderKind::OpenAiCompat,
        base_url,
        "x".into(),
        Some("provider-key".into()),
        false,
        Vec::new(),
        "genaryx-copilot".into(),
        AGENT.into(),
    )
    .expect("loopback endpoint needs no opt-in")
}

/// A scratch file under a directory this file owns, named per test.
fn scratch_file(name: &str, contents: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("genaryx-it-fuse-key-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join(name);
    std::fs::write(&path, contents).expect("write scratch file");
    path
}

/// @test:anthropic_request_carries_the_configured_x_fuse_key_header
#[tokio::test]
async fn anthropic_request_carries_the_configured_x_fuse_key_header() {
    let (base_url, captured) = spawn_stub(Reply::Ok(ANTHROPIC_OK));
    anthropic(base_url)
        .with_fuse_key(Some(key()))
        .chat(one_message_request())
        .await
        .expect("stub answers 200 with a parseable body");

    let headers = first(&captured);
    assert_eq!(
        headers.get("x-fuse-key").map(String::as_str),
        Some(KEY),
        "the anthropic client must send the configured key as x-fuse-key"
    );
    assert_eq!(
        headers.get("x-api-key").map(String::as_str),
        Some("provider-key"),
        "the provider key stays in its own header"
    );
    assert_eq!(
        headers.get("x-fuse-agent-id").map(String::as_str),
        Some(AGENT)
    );
}

/// @test:openai_compat_request_carries_the_configured_x_fuse_key_header
#[tokio::test]
async fn openai_compat_request_carries_the_configured_x_fuse_key_header() {
    let (base_url, captured) = spawn_stub(Reply::Ok(OPENAI_OK));
    openai(base_url)
        .with_fuse_key(Some(key()))
        .chat(one_message_request())
        .await
        .expect("stub answers 200 with a parseable body");

    let headers = first(&captured);
    assert_eq!(
        headers.get("x-fuse-key").map(String::as_str),
        Some(KEY),
        "the openai-compatible client must send the configured key as x-fuse-key"
    );
    assert_eq!(
        headers.get("authorization").map(String::as_str),
        Some("Bearer provider-key"),
        "the provider key stays the bearer credential"
    );
    assert_eq!(
        headers.get("x-fuse-agent-id").map(String::as_str),
        Some(AGENT)
    );
}

/// With no key, nothing changes: no `x-fuse-key` header at all, and the
/// identity headers Felyx already sent are still sent.
/// @test:no_fuse_key_configured_sends_no_x_fuse_key_header
#[tokio::test]
async fn no_fuse_key_configured_sends_no_x_fuse_key_header() {
    let (url, captured) = spawn_stub(Reply::Ok(ANTHROPIC_OK));
    anthropic(url.clone())
        .chat(one_message_request())
        .await
        .expect("200");
    anthropic(url)
        .with_fuse_key(None)
        .chat(one_message_request())
        .await
        .expect("200");
    let (url, openai_captured) = spawn_stub(Reply::Ok(OPENAI_OK));
    openai(url.clone())
        .chat(one_message_request())
        .await
        .expect("200");
    openai(url)
        .with_fuse_key(None)
        .chat(one_message_request())
        .await
        .expect("200");

    let mut all = captured.lock().expect("lock").clone();
    all.extend(openai_captured.lock().expect("lock").iter().cloned());
    assert_eq!(all.len(), 4, "four calls reached the stubs");
    for headers in all {
        assert!(
            !headers.contains_key("x-fuse-key"),
            "no key configured, no x-fuse-key: {headers:?}"
        );
        assert_eq!(
            headers.get("x-fuse-agent-id").map(String::as_str),
            Some(AGENT)
        );
        assert_eq!(
            headers.get("x-fuse-run-id").map(String::as_str),
            Some("genaryx-copilot")
        );
    }
}

/// A gateway that echoes the key into its `401` body: the error each client
/// returns names the status and carries the redaction marker, never the key.
/// @test:a_gateway_that_echoes_the_fuse_key_never_gets_it_into_felyxs_error
#[tokio::test]
async fn a_gateway_that_echoes_the_fuse_key_never_gets_it_into_felyxs_error() {
    let (url, a_captured) = spawn_stub(Reply::EchoTheKey);
    let from_anthropic = anthropic(url)
        .with_fuse_key(Some(key()))
        .chat(one_message_request())
        .await
        .expect_err("the stub answers 401");
    let (url, o_captured) = spawn_stub(Reply::EchoTheKey);
    let from_openai = openai(url)
        .with_fuse_key(Some(key()))
        .chat(one_message_request())
        .await
        .expect_err("the stub answers 401");

    for (name, captured, err) in [
        ("anthropic", a_captured, from_anthropic),
        ("openai", o_captured, from_openai),
    ] {
        assert_eq!(
            first(&captured).get("x-fuse-key").map(String::as_str),
            Some(KEY),
            "{name}: the stub must have received the key, or its echo proves nothing"
        );
        match &err {
            ProviderError::Http { status, body } => {
                assert_eq!(*status, 401, "{name}");
                assert!(
                    body.contains(FUSE_KEY_REDACTED),
                    "{name}: the echoed key is replaced by the marker"
                );
            }
            _ => panic!("{name}: expected the gateway's 401 as an HTTP error"),
        }
        assert!(
            !err.to_string().contains(KEY),
            "{name}: the key reached the error text"
        );
        assert!(
            !format!("{err:?}").contains(KEY),
            "{name}: the key reached the error's Debug"
        );
    }
}

/// The seam from configuration to the wire: the console builds Felyx from a
/// `CopilotConfig` whose `fuse_key_ref` names a file, exactly as
/// `GENARYX_COPILOT_FUSE_KEY_REF` reaches it, and a question then carries the
/// key, trimmed of the file's newline, for both providers.
/// @test:felyx_reads_its_fuse_key_file_and_sends_it_through_the_service
#[tokio::test]
async fn felyx_reads_its_fuse_key_file_and_sends_it_through_the_service() {
    let key_file = scratch_file("service-key", &format!("  {KEY}\n"));
    let api_file = scratch_file("service-api-key", "provider-key\n");

    let (url, openai_captured) = spawn_stub(Reply::Ok(OPENAI_OK));
    let openai_cfg = CopilotConfig {
        provider: ProviderKind::OpenAiCompat,
        base_url: Some(url),
        model: Some("x".into()),
        agent_id: Some(AGENT.into()),
        fuse_key_ref: Some(format!("file:{}", key_file.display())),
        ..Default::default()
    };
    let (url, anthropic_captured) = spawn_stub(Reply::Ok(ANTHROPIC_OK));
    let anthropic_cfg = CopilotConfig {
        provider: ProviderKind::Anthropic,
        base_url: Some(url),
        model: Some("claude".into()),
        api_key_ref: Some(format!("file:{}", api_file.display())),
        agent_id: Some(AGENT.into()),
        fuse_key_ref: Some(format!("file:{}", key_file.display())),
        ..Default::default()
    };

    for (name, cfg, captured) in [
        ("openai", openai_cfg, openai_captured),
        ("anthropic", anthropic_cfg, anthropic_captured),
    ] {
        let service = CopilotService::from_config_and_clients(&cfg, Clients::default())
            .expect("a local endpoint and a readable key build an enabled service");
        assert!(service.is_enabled(), "{name}");
        service
            .ask("status?")
            .await
            .expect("the stub answers with a final text");
        assert_eq!(
            first(&captured).get("x-fuse-key").map(String::as_str),
            Some(KEY),
            "{name}: the key the file holds is what the gateway receives"
        );
    }
}

/// The same refusal one layer further out, through the service the console
/// calls: the error text `copilot_ask` hands the browser names the gateway's
/// `401` and carries the marker, never the key.
/// @test:a_keyed_gateway_refusal_reaches_the_operator_without_the_fuse_key
#[tokio::test]
async fn a_keyed_gateway_refusal_reaches_the_operator_without_the_fuse_key() {
    let key_file = scratch_file("refusal-key", KEY);
    let (url, captured) = spawn_stub(Reply::EchoTheKey);
    let cfg = CopilotConfig {
        provider: ProviderKind::OpenAiCompat,
        base_url: Some(url),
        model: Some("x".into()),
        agent_id: Some(AGENT.into()),
        fuse_key_ref: Some(format!("file:{}", key_file.display())),
        ..Default::default()
    };
    let service = CopilotService::from_config_and_clients(&cfg, Clients::default())
        .expect("builds against a local stub");
    let shown = service
        .ask("status?")
        .await
        .expect_err("the stub refuses with 401")
        .to_string();

    assert_eq!(
        first(&captured).get("x-fuse-key").map(String::as_str),
        Some(KEY),
        "the stub must have received the key, or its echo proves nothing"
    );
    assert!(
        shown.contains("401"),
        "the operator sees the gateway's status"
    );
    assert!(
        shown.contains(FUSE_KEY_REDACTED),
        "the echoed key is replaced by the marker"
    );
    assert!(
        !shown.contains(KEY),
        "the key reached the operator's screen"
    );
}

/// A key Felyx cannot send disables Felyx at construction, the way every
/// other misconfigured copilot setting does (the console keeps serving its
/// other panels), and the reason it gives names the setting while repeating
/// neither the key nor the reference.
/// @test:a_fuse_key_felyx_cannot_send_disables_it_without_repeating_the_key
#[test]
fn a_fuse_key_felyx_cannot_send_disables_it_without_repeating_the_key() {
    let spaced = "sk-felyx with a space-0123456789";
    let spaced_file = scratch_file("disabled-key", spaced);
    for (case, reference, secret) in [
        ("the key pasted as the reference", KEY.to_string(), KEY),
        (
            "a key with a space",
            format!("file:{}", spaced_file.display()),
            spaced,
        ),
    ] {
        let cfg = CopilotConfig {
            provider: ProviderKind::OpenAiCompat,
            base_url: Some("http://127.0.0.1:9/v1".into()),
            model: Some("x".into()),
            fuse_key_ref: Some(reference),
            ..Default::default()
        };
        let reason = match CopilotService::from_config_and_clients(&cfg, Clients::default()) {
            Ok(_) => panic!("{case}: Felyx was built with a key it cannot send"),
            Err(e) => e.to_string(),
        };
        assert!(
            reason.contains("GENARYX_COPILOT_FUSE_KEY_REF"),
            "{case}: the reason names the setting: {reason}"
        );
        assert!(
            !reason.contains(secret),
            "{case}: the reason repeats the key"
        );
    }
}
