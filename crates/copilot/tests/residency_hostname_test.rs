//! Integration proof for invariant 14 (2026-09-27): the residency gate can
//! accept a HOSTNAME - a Kubernetes Service name or a Compose service name,
//! neither a literal address nor `localhost` - when, and only when, it can
//! prove every address that name resolves to is local, and it must hold that
//! at CONNECTION time, not only when the provider is built (DNS rebinding).
//!
//! HAND-ROLLED stub server, same shape `agent_id_header_test.rs` and
//! `crates/api/tests/delegation_revoke_test.rs` already use: this proves
//! GENARYX's own client behaviour, never a real provider's or a real DNS
//! server's correctness. Every lookup here is injected (`FixedLookup`/
//! `ErrLookup`/`SequencedLookup`/`CountingLookup`), so nothing depends on
//! real DNS or network reachability.

use genaryx_copilot::provider::{AnthropicMessages, OpenAiCompat};
use genaryx_copilot::resolver::HostnameLookup;
use genaryx_copilot::{ChatRequest, LlmProvider, Message, ProviderError, ProviderKind};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Answers every connection with a fixed 200/`body` (never closes early), so
/// a test that only needs the request to SUCCEED doesn't have to count
/// connections.
fn spawn_stub(body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub listener");
    let addr = listener.local_addr().expect("stub local addr");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            serve_one(stream, body);
        }
    });
    format!("http://{addr}")
}

fn serve_one(stream: TcpStream, body: &'static str) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut raw_body = vec![0u8; content_length];
    if content_length > 0 {
        let _ = reader.read_exact(&mut raw_body);
    }
    let body_bytes = body.as_bytes();
    let mut out = reader.into_inner();
    let _ = write!(
        out,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body_bytes.len()
    );
    let _ = out.write_all(body_bytes);
    let _ = out.flush();
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

const OPENAI_BODY: &str = r#"{"choices":[{"message":{"content":"ok"}}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;

/// A fixed answer for every call.
struct FixedLookup(Vec<IpAddr>);
impl HostnameLookup for FixedLookup {
    fn lookup(&self, _host: &str) -> std::io::Result<Vec<IpAddr>> {
        Ok(self.0.clone())
    }
}

/// Always errors, as an unresolvable name would.
struct ErrLookup;
impl HostnameLookup for ErrLookup {
    fn lookup(&self, _host: &str) -> std::io::Result<Vec<IpAddr>> {
        Err(std::io::Error::other("nxdomain: no such host"))
    }
}

/// Returns each of `answers` in turn (one per call), then repeats the last -
/// used to prove the check re-runs at connection time: the FIRST call (the
/// provider constructor's own build-time check) sees one answer, and the
/// SECOND call (the real HTTP client actually connecting) sees another.
struct SequencedLookup {
    answers: Vec<Vec<IpAddr>>,
    calls: AtomicUsize,
}
impl SequencedLookup {
    fn new(answers: Vec<Vec<IpAddr>>) -> Self {
        Self {
            answers,
            calls: AtomicUsize::new(0),
        }
    }
}
impl HostnameLookup for SequencedLookup {
    fn lookup(&self, _host: &str) -> std::io::Result<Vec<IpAddr>> {
        let i = self.calls.fetch_add(1, Ordering::SeqCst);
        let idx = i.min(self.answers.len() - 1);
        Ok(self.answers[idx].clone())
    }
}

/// Counts calls, so a test can prove the eligibility (allow-list) check
/// short-circuits BEFORE any DNS lookup for a hostname the operator never
/// named.
#[derive(Default)]
struct CountingLookup {
    calls: AtomicUsize,
}
impl HostnameLookup for CountingLookup {
    fn lookup(&self, _host: &str) -> std::io::Result<Vec<IpAddr>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))])
    }
}

/// @test:a_hostname_resolving_only_to_a_private_address_is_accepted_end_to_end
#[tokio::test]
async fn a_hostname_resolving_only_to_a_private_address_is_accepted_end_to_end() {
    let base_url = spawn_stub(OPENAI_BODY);
    // The stub is bound to 127.0.0.1, but the provider is pointed at it
    // through a NAME (the injected lookup, not real DNS, answers it), to
    // exercise the hostname path rather than the literal-IP one.
    let port: u16 = base_url
        .rsplit_once(':')
        .expect("stub url carries a port")
        .1
        .parse()
        .expect("port is numeric");
    let hostname_url = format!("http://tokenfuse-gateway:{port}");

    // The resolved address must be where the stub is REALLY listening
    // (127.0.0.1) for the connection to actually succeed - this test proves
    // the resolver's answer is what reqwest dials, not merely that some
    // private-looking address was accepted.
    let lookup: Arc<dyn HostnameLookup> =
        Arc::new(FixedLookup(vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))]));
    let provider = OpenAiCompat::new_with_lookup(
        ProviderKind::OpenAiCompat,
        hostname_url,
        "x".into(),
        None,
        false,
        vec!["tokenfuse-gateway".to_string()],
        "genaryx-copilot".into(),
        "agent://local/genaryx/felyx".into(),
        lookup,
    )
    .expect("an allow-listed hostname resolving only privately must build");
    assert!(provider.descriptor().local, "must report itself local");

    provider
        .chat(one_message_request())
        .await
        .expect("the resolver must translate the hostname to the stub's real port");
}

/// @test:a_hostname_not_on_the_allow_list_is_refused_with_no_dns_lookup_at_all
#[test]
fn a_hostname_not_on_the_allow_list_is_refused_with_no_dns_lookup_at_all() {
    let counting: Arc<CountingLookup> = Arc::new(CountingLookup::default());
    let err = AnthropicMessages::new_with_lookup(
        "http://tokenfuse-gateway:4100".into(),
        "claude".into(),
        "k".into(),
        false,
        Vec::new(), // nothing allow-listed
        "genaryx-copilot".into(),
        "agent://local/genaryx/felyx".into(),
        counting.clone(),
    )
    .unwrap_err();
    assert!(matches!(err, ProviderError::NonLocalEndpointRefused { .. }));
    assert_eq!(
        counting.calls.load(Ordering::SeqCst),
        0,
        "an ineligible hostname must never be resolved at all"
    );
}

/// @test:a_hostname_resolving_to_one_private_and_one_public_address_is_refused_at_construction
#[test]
fn a_hostname_resolving_to_one_private_and_one_public_address_is_refused_at_construction() {
    let lookup: Arc<dyn HostnameLookup> = Arc::new(FixedLookup(vec![
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
        IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
    ]));
    let err = OpenAiCompat::new_with_lookup(
        ProviderKind::OpenAiCompat,
        "http://tokenfuse-gateway:4100".into(),
        "x".into(),
        None,
        false,
        vec!["tokenfuse-gateway".to_string()],
        "genaryx-copilot".into(),
        "agent://local/genaryx/felyx".into(),
        lookup,
    )
    .unwrap_err();
    match err {
        ProviderError::NonLocalEndpointRefused { url } => {
            assert!(url.contains("8.8.8.8"), "{url}");
        }
        other => panic!("expected NonLocalEndpointRefused, got {other:?}"),
    }
}

/// @test:an_unresolvable_hostname_is_refused_at_construction
#[test]
fn an_unresolvable_hostname_is_refused_at_construction() {
    let err = AnthropicMessages::new_with_lookup(
        "http://tokenfuse-gateway:4100".into(),
        "claude".into(),
        "k".into(),
        false,
        vec!["tokenfuse-gateway".to_string()],
        "genaryx-copilot".into(),
        "agent://local/genaryx/felyx".into(),
        Arc::new(ErrLookup),
    )
    .unwrap_err();
    match err {
        ProviderError::NonLocalEndpointRefused { url } => {
            assert!(url.contains("nxdomain"), "{url}");
        }
        other => panic!("expected NonLocalEndpointRefused, got {other:?}"),
    }
}

/// @test:a_hostname_that_resolves_privately_at_build_and_publicly_at_request_is_refused_at_request_time
///
/// The rebinding case: the SAME hostname answers PRIVATELY the first time it
/// is resolved (the provider constructor's own build-time check) and
/// PUBLICLY the second time (the real HTTP client actually connecting to
/// send the chat request). Building must succeed; the chat call must not.
#[tokio::test]
async fn a_hostname_that_resolves_privately_at_build_and_publicly_at_request_is_refused_at_request_time()
 {
    let lookup: Arc<dyn HostnameLookup> = Arc::new(SequencedLookup::new(vec![
        vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))], // build time: private
        vec![IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))],  // request time: public
    ]));
    let provider = OpenAiCompat::new_with_lookup(
        ProviderKind::OpenAiCompat,
        "http://rebinding-svc:4100".into(),
        "x".into(),
        None,
        false,
        vec!["rebinding-svc".to_string()],
        "genaryx-copilot".into(),
        "agent://local/genaryx/felyx".into(),
        lookup,
    )
    .expect("build time saw a private address and must succeed");
    assert!(provider.descriptor().local);

    let err = provider
        .chat(one_message_request())
        .await
        .expect_err("request time must re-check and see the public address");
    let msg = err.to_string();
    assert!(
        msg.contains("8.8.8.8") || msg.contains("non-local") || msg.contains("residency"),
        "the connection-time refusal must be readable, not a bare transport error: {msg}"
    );
}

/// @test:anthropic_hostname_path_also_rechecks_at_request_time
///
/// The Anthropic client is a separate `reqwest::Client` builder call from
/// `OpenAiCompat`'s; prove the same rebinding protection independently
/// rather than assuming it from one provider's test alone.
#[tokio::test]
async fn anthropic_hostname_path_also_rechecks_at_request_time() {
    let lookup: Arc<dyn HostnameLookup> = Arc::new(SequencedLookup::new(vec![
        vec![IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20))],
        vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))],
    ]));
    let provider = AnthropicMessages::new_with_lookup(
        "http://rebinding-svc:4100".into(),
        "claude".into(),
        "k".into(),
        false,
        vec!["rebinding-svc".to_string()],
        "genaryx-copilot".into(),
        "agent://local/genaryx/felyx".into(),
        lookup,
    )
    .expect("build time saw a private address and must succeed");

    let err = provider
        .chat(one_message_request())
        .await
        .expect_err("request time must re-check and see the public address");
    let msg = err.to_string();
    assert!(
        msg.contains("1.1.1.1") || msg.contains("non-local") || msg.contains("residency"),
        "{msg}"
    );
}
