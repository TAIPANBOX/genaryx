//! Invariant 14, the other half of "the address checked is the address
//! dialed": a residency-gated client never follows a redirect. reqwest
//! follows up to ten by default, and a `Location` naming a literal public
//! address never passes through the DNS resolver that re-checks every
//! connection, so a local endpoint that answered 307 could send a turn (and
//! its prompt) anywhere. Hand-rolled stubs, same shape as
//! `residency_hostname_test.rs`.

use genaryx_copilot::provider::{AnthropicMessages, OpenAiCompat};
use genaryx_copilot::resolver::HostnameLookup;
use genaryx_copilot::{ChatRequest, LlmProvider, Message, ProviderKind};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Reads one request, then writes `head` + `body` as the whole response.
fn answer(stream: TcpStream, head: &str, body: &str) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut content_length = 0usize;
    let mut first = true;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let t = line.trim_end_matches(['\r', '\n']);
        if t.is_empty() && !first {
            break;
        }
        first = false;
        if let Some((n, v)) = t.split_once(':')
            && n.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut b = vec![0u8; content_length];
    let _ = reader.read_exact(&mut b);
    let mut out = reader.into_inner();
    let _ = write!(
        out,
        "{head}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = out.flush();
}

/// A stub that counts what reaches it and answers a valid turn for either wire.
fn spawn_target(hits: Arc<AtomicUsize>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind target");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            hits.fetch_add(1, Ordering::SeqCst);
            answer(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n",
                r#"{"choices":[{"message":{"content":"ok"}}],"usage":{"prompt_tokens":1,"completion_tokens":1},"content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn"}"#,
            );
        }
    });
    port
}

/// A stub that redirects every request to `target_port`, path kept.
fn spawn_redirector(target_port: u16) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind redirector");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let head = format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://127.0.0.1:{target_port}/elsewhere\r\n"
            );
            answer(s, &head, "");
        }
    });
    port
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

struct Loopback;
impl HostnameLookup for Loopback {
    fn lookup(&self, _host: &str) -> std::io::Result<Vec<IpAddr>> {
        Ok(vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))])
    }
}

/// @test:a_gated_client_on_a_literal_local_address_does_not_follow_a_redirect
#[tokio::test]
async fn a_gated_client_on_a_literal_local_address_does_not_follow_a_redirect() {
    let hits = Arc::new(AtomicUsize::new(0));
    let target = spawn_target(hits.clone());
    let redirector = spawn_redirector(target);
    let provider = OpenAiCompat::new(
        ProviderKind::OpenAiCompat,
        format!("http://127.0.0.1:{redirector}"),
        "x".into(),
        None,
        false,
        Vec::new(),
        "genaryx-copilot".into(),
        "agent://local/genaryx/felyx".into(),
    )
    .expect("a literal local address builds");
    let result = provider.chat(one_message_request()).await;
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "the redirect was followed: the turn reached an address the gate never checked"
    );
    assert!(result.is_err(), "a 307 is not a turn");
}

/// @test:a_gated_client_on_an_allow_listed_hostname_does_not_follow_a_redirect
#[tokio::test]
async fn a_gated_client_on_an_allow_listed_hostname_does_not_follow_a_redirect() {
    let hits = Arc::new(AtomicUsize::new(0));
    let target = spawn_target(hits.clone());
    let redirector = spawn_redirector(target);
    let provider = AnthropicMessages::new_with_lookup(
        format!("http://tokenfuse-gateway:{redirector}"),
        "x".into(),
        "k".into(),
        false,
        vec!["tokenfuse-gateway".to_string()],
        "genaryx-copilot".into(),
        "agent://local/genaryx/felyx".into(),
        Arc::new(Loopback),
    )
    .expect("an allow-listed hostname resolving to loopback builds");
    let result = provider.chat(one_message_request()).await;
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "the redirect was followed: the turn reached an address the gate never checked"
    );
    assert!(result.is_err(), "a 307 is not a turn");
}
