//! Invariant 14: a residency-gated client ignores `HTTP_PROXY`/`ALL_PROXY`.
//! reqwest reads the system proxy from the environment by default, and a
//! proxy is a second destination the gate never checked: the turn would be
//! handed to whatever the variable names. Its own test binary, because it
//! sets a process-wide environment variable and must not race any other test.

use genaryx_copilot::provider::OpenAiCompat;
use genaryx_copilot::{ChatRequest, LlmProvider, Message, ProviderKind};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

fn spawn(hits: Arc<AtomicUsize>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            hits.fetch_add(1, Ordering::SeqCst);
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let mut r = BufReader::new(s.try_clone().expect("clone"));
            let mut l = String::new();
            while r.read_line(&mut l).unwrap_or(0) > 0 && l != "\r\n" {
                l.clear();
            }
            let body = r#"{"choices":[{"message":{"content":"ok"}}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;
            let mut o = r.into_inner();
            let _ = write!(
                o,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    port
}

/// @test:a_gated_client_ignores_the_proxy_the_environment_names
#[tokio::test]
async fn a_gated_client_ignores_the_proxy_the_environment_names() {
    let proxy_hits = Arc::new(AtomicUsize::new(0));
    let proxy = spawn(proxy_hits.clone());
    let endpoint_hits = Arc::new(AtomicUsize::new(0));
    let endpoint = spawn(endpoint_hits.clone());
    // SAFETY: this binary holds exactly one test, so nothing else reads the
    // environment concurrently.
    unsafe {
        std::env::set_var("HTTP_PROXY", format!("http://127.0.0.1:{proxy}"));
        std::env::set_var("http_proxy", format!("http://127.0.0.1:{proxy}"));
        std::env::set_var("ALL_PROXY", format!("http://127.0.0.1:{proxy}"));
        std::env::remove_var("NO_PROXY");
        std::env::remove_var("no_proxy");
    }
    let provider = OpenAiCompat::new(
        ProviderKind::OpenAiCompat,
        format!("http://127.0.0.1:{endpoint}"),
        "x".into(),
        None,
        false,
        Vec::new(),
        "genaryx-copilot".into(),
        "agent://local/genaryx/felyx".into(),
    )
    .expect("a literal local address builds");
    let _ = provider
        .chat(ChatRequest {
            system: String::new(),
            messages: vec![Message::user("q")],
            tools: Vec::new(),
            max_tokens: 64,
            temperature: 0.0,
        })
        .await;
    assert_eq!(
        proxy_hits.load(Ordering::SeqCst),
        0,
        "the turn went through the proxy the environment named"
    );
    assert_eq!(
        endpoint_hits.load(Ordering::SeqCst),
        1,
        "the turn must reach the endpoint itself"
    );
}
