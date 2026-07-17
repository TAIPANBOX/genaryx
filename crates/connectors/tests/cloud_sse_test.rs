//! Integration proof for `CloudSse` (Phase-0 spike #6, 06 §7 / 07 §4.2)
//! against a real local TCP socket: a tiny hand-rolled HTTP/1.1 SSE server
//! (`std::net` only, no hyper/axum) that
//!
//!   1. splits one event's JSON across two separate socket writes, proving
//!      chunk-split reassembly over a genuine transport (not just the
//!      decoder unit tests in `sse_decoder.rs`), and
//!   2. delivers two more events in a single write, then closes the
//!      connection (simulating a server-side drop).
//!
//! `CloudSse` must reconnect on its own and read one more event from a
//! second accepted connection. This is a real socket end to end
//! (`127.0.0.1`, an ephemeral port, a real `reqwest` HTTP client) -- not a
//! scripted decoder replay.

use genaryx_connectors::{CloudSse, CloudSseConfig};
use genaryx_core::EventSource;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// Read and discard whatever the client has already sent (its HTTP
/// request). Not needed for correctness of the response we send, but avoids
/// a BSD-socket gotcha found while building this test: closing a socket
/// while the peer's bytes are still sitting unread in our receive buffer
/// sends a TCP RST instead of a clean FIN, and `reqwest`'s hyper-based
/// client fails the whole in-flight request on an RST (`hyper::Error(..,
/// UnexpectedMessage)`) rather than surfacing it as a stream-read error
/// after delivering the body -- draining first turns the deliberate
/// mid-body close below into a graceful FIN, which hyper handles as a
/// normal (if incomplete) body end. A short best-effort read is enough for
/// the tiny GET `reqwest` sends here; a read timeout guards against ever
/// blocking indefinitely if, for some reason, nothing arrives.
fn drain_request(stream: &mut TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let mut buf = [0u8; 8192];
    let _ = stream.read(&mut buf);
}

/// Wrap `data` as one HTTP/1.1 chunked-transfer-encoding chunk
/// (`<hex-size>\r\n<data>\r\n`).
fn http_chunk(data: &[u8]) -> Vec<u8> {
    let mut out = format!("{:x}\r\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\r\n");
    out
}

/// Write a minimal `text/event-stream` HTTP/1.1 response: the header
/// (`Transfer-Encoding: chunked`, no `Content-Length`), then each `frames`
/// element as its own HTTP chunk, in its own `write` (with a short delay
/// first), then drop the connection *without* a terminating `0\r\n\r\n`
/// chunk -- an abrupt mid-body disconnect, which is our "server drops the
/// connection" trigger for the reconnect half of this test.
///
/// Chunked encoding (rather than a `Connection: close` / no-framing body) is
/// deliberate: a bare close-delimited response with neither `Content-Length`
/// nor `Transfer-Encoding` was tried first and hyper's client (which
/// `reqwest` uses) tore the connection down before the server finished
/// writing it, so this uses well-defined HTTP/1.1 framing instead. It also
/// gives precise control over `.bytes_stream()` boundaries: each `frames`
/// element becomes exactly one HTTP chunk, so splitting one SSE `data:` line
/// across two `frames` elements reliably yields two separate `Bytes` items
/// on the client, proving `SseDecoder` reassembly through the real
/// `reqwest`/hyper pipeline (not just the direct unit tests).
///
/// Drains the incoming request first (see [`drain_request`]) so the
/// deliberate mid-body close below is a clean TCP FIN, not an RST.
fn write_sse_response(mut stream: TcpStream, frames: &[&[u8]]) {
    drain_request(&mut stream);
    let header = b"HTTP/1.1 200 OK\r\n\
Content-Type: text/event-stream\r\n\
Cache-Control: no-cache\r\n\
Transfer-Encoding: chunked\r\n\
\r\n";
    stream.write_all(header).expect("write response header");
    stream.flush().expect("flush header");
    for frame in frames {
        std::thread::sleep(Duration::from_millis(20));
        let chunk = http_chunk(frame);
        stream.write_all(&chunk).expect("write sse frame");
        stream.flush().expect("flush sse frame");
    }
    // `stream` drops here, still mid-body (no terminating chunk): the
    // socket closes, simulating a server-side disconnect.
}

/// Serves exactly two connections then returns: the initial connect, and
/// the one reconnect `CloudSse` is expected to make after the first drops.
fn run_mock_server(listener: TcpListener) {
    let (first, _) = listener.accept().expect("accept first connection");
    write_sse_response(
        first,
        &[
            b"data: {\"seq\":1".as_slice(),
            b"}\n\n".as_slice(),
            b"data: {\"seq\":2}\n\ndata: {\"seq\":3}\n\n".as_slice(),
        ],
    );

    let (second, _) = listener.accept().expect("accept reconnect");
    write_sse_response(second, &[b"data: {\"seq\":4}\n\n".as_slice()]);
}

#[test]
fn reconnects_after_drop_and_reassembles_a_chunk_split_frame() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("local_addr");
    let server = std::thread::spawn(move || run_mock_server(listener));

    let config = CloudSseConfig {
        url: format!("http://{addr}/v1/stream"),
        bearer_token: "test-token".to_string(),
        initial_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(100),
        max_attempts: Some(20),
    };

    let mut source = CloudSse::spawn("cloud-sse:test", config).expect("spawn CloudSse");

    let mut received: Vec<String> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while received.len() < 4 && Instant::now() < deadline {
        match source.poll() {
            Ok(records) => received.extend(records.into_iter().map(|r| r.raw)),
            Err(e) => panic!("unexpected poll error: {e}"),
        }
        if received.len() < 4 {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    source.shutdown();
    server.join().expect("mock server thread panicked");

    assert_eq!(
        received,
        vec![
            r#"{"seq":1}"#.to_string(),
            r#"{"seq":2}"#.to_string(),
            r#"{"seq":3}"#.to_string(),
            r#"{"seq":4}"#.to_string(),
        ],
        "all 4 events arrive in order: event 1's JSON was reassembled from \
         two separate socket writes, and event 4 arrived over a second, \
         reconnected TCP connection after the first was dropped"
    );
}
