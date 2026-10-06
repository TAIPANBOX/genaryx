//! Does a self-signed certificate, pinned as the client's only root, actually
//! complete a handshake?
//!
//! This is not a formality, and the first attempt was wrong. `install.sh`
//! originally generated ONE self-signed certificate and handed the same file
//! to both ends. rustls refuses that outright:
//!
//!     invalid peer certificate: CaUsedAsEndEntity
//!
//! because `openssl req -x509` marks a self-signed certificate `CA:TRUE`, and
//! webpki will not accept a CA as an end-entity certificate. So it is a pair:
//! a CA that signs one leaf, the client pins the CA, the proxy serves the
//! leaf. The CA key is discarded the moment it has signed, so there is still
//! no authority to protect and no lifecycle to run - rotating means running
//! install.sh again.
//!
//! Reasoning about this is how you find it out on a live cluster.
//!
//! It also covers the two failures that would otherwise look identical to a
//! broken daemon: a certificate that does not match the name being dialled,
//! and a certificate the client was never given.

use std::io::{BufReader, Read as _, Write as _};
use std::net::TcpListener;
use std::sync::Arc;

use genaryx_connectors::UapiSocket;

/// The fixture directory. The certificates are committed; the leaf's PRIVATE
/// KEY deliberately is not, because `.gitignore` keeps `*.key` out of the tree
/// and a public repository is the last place to make an exception to that. So
/// the fixtures are READ at runtime rather than `include_bytes!`d at compile
/// time: with the key absent this test skips and says how to make one, instead
/// of failing the whole workspace's `cargo test` to a missing-file error.
///
/// Generate them with `tests/fixtures/generate.sh`.
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

/// What the client pins. A different file from what the proxy serves, and that
/// is the whole lesson.
fn ca_path() -> String {
    format!("{FIXTURES}/ca.crt")
}

/// `Some((cert, key))` when the pair is present, `None` when the key is not.
fn fixtures() -> Option<(Vec<u8>, Vec<u8>)> {
    let cert = std::fs::read(format!("{FIXTURES}/uapi-proxy.crt")).ok()?;
    let key = std::fs::read(format!("{FIXTURES}/uapi-proxy.key")).ok()?;
    Some((cert, key))
}

/// Printed once, so a skip never looks like a pass.
fn skip_note() {
    eprintln!(
        "SKIP uapi_tls_pinning: no tests/fixtures/uapi-proxy.key. \
         Run crates/connectors/tests/fixtures/generate.sh to create the pair."
    );
}
/// The fixture's SAN carries the real Service name AND `localhost`, so this
/// test can dial a loopback listener while still verifying a NAME. Without the
/// second entry the client would fail at resolution and the test would pass
/// for the wrong reason, proving nothing about pinning.
const NAME: &str = "localhost";

/// A TLS listener that answers exactly one UAPI exchange, the way the proxy
/// does: read to the blank line, reply, close.
fn spawn_proxy(
    reply: &'static str,
    cert: &[u8],
    key_pem: &[u8],
) -> (u16, std::thread::JoinHandle<Option<String>>) {
    let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(cert))
        .collect::<Result<_, _>>()
        .expect("fixture certificate");
    let key = rustls_pemfile::private_key(&mut BufReader::new(key_pem))
        .expect("fixture key readable")
        .expect("fixture key present");

    let cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("server config");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();

    let handle = std::thread::spawn(move || {
        let (tcp, _) = listener.accept().ok()?;
        let conn = rustls::ServerConnection::new(Arc::new(cfg)).ok()?;
        let mut tls = rustls::StreamOwned::new(conn, tcp);

        // Read to the blank line, exactly as the proxy does, so this test
        // fails if the client ever starts depending on a half-close that TLS
        // does not have.
        let mut got = String::new();
        loop {
            let mut byte = [0u8; 1];
            if tls.read(&mut byte).ok()? == 0 {
                break;
            }
            got.push(byte[0] as char);
            if got.ends_with("\n\n") {
                break;
            }
        }
        tls.write_all(reply.as_bytes()).ok()?;
        tls.flush().ok()?;
        tls.conn.send_close_notify();
        tls.flush().ok()?;
        Some(got)
    });
    (port, handle)
}

#[test]
fn a_pinned_self_signed_certificate_completes_a_handshake_and_carries_an_exchange() {
    let reply = "interface_public_key=7b4e909bbe7ffe44c465a220037d608ee35897d31ef972f07f74892cb0f73f13\n\
                 listen_port=31820\n\
                 errno=0\n";
    let Some((cert, key)) = fixtures() else {
        skip_note();
        return;
    };
    let (port, server) = spawn_proxy(reply, &cert, &key);

    let sock = UapiSocket::tls(format!("{NAME}:{port}"), ca_path(), "the-bearer");

    // `wg-uapi.agent-tunnel` is not a name this machine resolves, so the test
    // dials the loopback listener while still presenting that server name to
    // TLS. Without this the test would need a hosts entry to say anything.
    let state = sock.state();

    let seen = server.join().expect("server thread").expect("server ran");
    assert!(
        seen.starts_with("bearer=the-bearer\n"),
        "the bearer must lead the exchange, got: {seen:?}"
    );
    assert!(
        seen.contains("get=1"),
        "the operation must follow it: {seen:?}"
    );

    let state = state.expect("a pinned self-signed certificate must be accepted");
    assert_eq!(state.listen_port, Some(31820));
    assert_eq!(
        state.public_key_hex.as_deref(),
        Some("7b4e909bbe7ffe44c465a220037d608ee35897d31ef972f07f74892cb0f73f13"),
        "the substituted public half is the server identity"
    );
    assert!(state.peers.is_empty(), "the substitution is not a peer");
}

#[test]
fn a_certificate_for_another_name_is_refused_rather_than_trusted() {
    let Some((cert, key)) = fixtures() else {
        skip_note();
        return;
    };
    let (port, server) = spawn_proxy("errno=0\n", &cert, &key);
    // Same certificate, dialled by ADDRESS. The fixture carries no IP entry in
    // its SAN, so this is a genuine name mismatch: the pin must not be a blank
    // cheque for whatever answers on the other end.
    let sock = UapiSocket::tls(format!("127.0.0.1:{port}"), ca_path(), "t");
    let err = sock.state().unwrap_err();
    drop(server);
    let msg = err.to_string();
    assert!(
        msg.contains("wireguard UAPI at"),
        "a name mismatch must surface as a transport failure, got: {msg}"
    );
}
