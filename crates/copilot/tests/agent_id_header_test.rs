//! Integration proof for the 2026-09-27 defect fix: Felyx (this crate's own
//! provider clients) must send `x-fuse-agent-id` on every model call, beside
//! the existing `x-fuse-run-id`, so a TokenFuse gateway running its Wardryx
//! hook in enforce mode can identify the console's own copilot instead of
//! refusing it with `identity_required` for carrying no agent identity at
//! all (measured on a live cluster, 2026-09-27: `400
//! {"error":{"reason":"policy enforcement is on and this request carries no
//! agent identity; send one in \`x-fuse-agent-id\`",...}}`).
//!
//! HAND-ROLLED stub server, same shape `crates/api/tests/
//! delegation_revoke_test.rs` already uses for vouchryx: this proves GENARYX's
//! own HTTP client sends the header, never a real provider's correctness.

use genaryx_copilot::provider::{AnthropicMessages, OpenAiCompat};
use genaryx_copilot::{ChatRequest, LlmProvider, Message, ProviderKind};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Every header name lowercased (HTTP header names are case-insensitive), so
/// a test asserting `headers.get("x-fuse-agent-id")` is stable regardless of
/// how a client happened to case it.
type CapturedHeaders = HashMap<String, String>;

/// Read one HTTP/1.1 request off `stream` and answer it with a fixed 200 and
/// `body`, handing back every header the request carried.
fn serve_one(stream: TcpStream, body: &'static str) -> CapturedHeaders {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));

    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .expect("read request line");

    let mut headers = HashMap::new();
    let mut content_length: usize = 0;
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
            }
            headers.insert(name, value);
        }
    }

    let mut raw_body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut raw_body).expect("read body");
    }

    let body_bytes = body.as_bytes();
    let mut out = reader.into_inner();
    write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body_bytes.len()
    )
    .expect("write response head");
    out.write_all(body_bytes).expect("write response body");
    out.flush().ok();

    headers
}

/// Spawn a stub bound to `127.0.0.1:0` that answers exactly one connection
/// with a fixed 200/`body`, and hand back its base URL plus a slot the
/// captured headers land in once the exchange completes.
fn spawn_stub(body: &'static str) -> (String, Arc<Mutex<Option<CapturedHeaders>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub listener");
    let addr = listener.local_addr().expect("stub local addr");
    let captured = Arc::new(Mutex::new(None));
    let captured_thread = Arc::clone(&captured);
    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            let headers = serve_one(stream, body);
            *captured_thread.lock().expect("lock captured slot") = Some(headers);
        }
    });
    (format!("http://{addr}"), captured)
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

/// @test:anthropic_request_carries_the_configured_x_fuse_agent_id_header
#[tokio::test]
async fn anthropic_request_carries_the_configured_x_fuse_agent_id_header() {
    let (base_url, captured) = spawn_stub(
        r#"{"content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":1,"output_tokens":1}}"#,
    );
    let provider = AnthropicMessages::new(
        base_url,
        "claude".into(),
        "k".into(),
        false, // the stub is bound to 127.0.0.1, which the residency gate treats as local
        Vec::new(),
        "genaryx-copilot".into(),
        "agent://acme.example/genaryx/felyx".into(),
    )
    .expect("local stub endpoint");

    provider
        .chat(one_message_request())
        .await
        .expect("stub answers 200 with a parseable body");

    let headers = captured
        .lock()
        .expect("lock captured slot")
        .take()
        .expect("stub received exactly one request");
    assert_eq!(
        headers.get("x-fuse-agent-id").map(String::as_str),
        Some("agent://acme.example/genaryx/felyx"),
        "the anthropic client must send x-fuse-agent-id: {headers:?}"
    );
    assert_eq!(
        headers.get("x-fuse-run-id").map(String::as_str),
        Some("genaryx-copilot"),
        "x-fuse-run-id must still be sent beside x-fuse-agent-id: {headers:?}"
    );
}

/// @test:openai_compat_request_carries_the_configured_x_fuse_agent_id_header
#[tokio::test]
async fn openai_compat_request_carries_the_configured_x_fuse_agent_id_header() {
    let (base_url, captured) = spawn_stub(
        r#"{"choices":[{"message":{"content":"ok"}}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#,
    );
    let provider = OpenAiCompat::new(
        ProviderKind::OpenAiCompat,
        base_url,
        "x".into(),
        None,
        false,
        Vec::new(),
        "genaryx-copilot".into(),
        "agent://acme.example/genaryx/felyx".into(),
    )
    .expect("loopback endpoint needs no opt-in");

    provider
        .chat(one_message_request())
        .await
        .expect("stub answers 200 with a parseable body");

    let headers = captured
        .lock()
        .expect("lock captured slot")
        .take()
        .expect("stub received exactly one request");
    assert_eq!(
        headers.get("x-fuse-agent-id").map(String::as_str),
        Some("agent://acme.example/genaryx/felyx"),
        "the openai-compatible client must send x-fuse-agent-id: {headers:?}"
    );
    assert_eq!(
        headers.get("x-fuse-run-id").map(String::as_str),
        Some("genaryx-copilot"),
        "x-fuse-run-id must still be sent beside x-fuse-agent-id: {headers:?}"
    );
}
