//! Cross-language checker for spike #2: read a `{pubkey_b64, message_b64,
//! sig_b64}` JSON blob on stdin (the Swift harness's `--emit-json` output) and
//! verify it through [`genaryx_signing::verify_es256_b64`] - the exact
//! operation sequence `tokenfuse-cloud`'s `devices.rs` runs. A CryptoKit
//! Secure-Enclave signature passing here is proof the two paths are
//! wire-compatible without ever touching the network.
//!
//! ```sh
//! (cd crates/signing/enclave-smoke && swift run enclave-smoke --emit-json) \
//!   | cargo run -p genaryx-signing --example verify_es256_blob
//! ```

use std::io::Read;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;

fn main() {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .expect("read stdin");
    let v: serde_json::Value = serde_json::from_str(input.trim()).expect("json blob on stdin");
    let pubkey_b64 = v["pubkey_b64"].as_str().expect("pubkey_b64");
    let message_b64 = v["message_b64"].as_str().expect("message_b64");
    let sig_b64 = v["sig_b64"].as_str().expect("sig_b64");
    let assurance = v["assurance"].as_str().unwrap_or("unknown");

    let message = B64.decode(message_b64).expect("message_b64 decodes");
    println!(
        "message ({} bytes):\n---\n{}\n---",
        message.len(),
        String::from_utf8_lossy(&message)
    );

    match genaryx_signing::verify_es256_b64(pubkey_b64, &message, sig_b64) {
        Ok(()) => {
            println!(
                "VERIFIED: {assurance} signature accepted by the devices.rs verify path (p256 0.13)"
            );
        }
        Err(e) => {
            println!("REJECTED: {e}");
            std::process::exit(1);
        }
    }

    // Tamper check: the same signature must NOT verify a modified message.
    let mut tampered = message.clone();
    match tampered.first_mut() {
        Some(b) => *b ^= 0x01,
        None => tampered.push(b'x'),
    }
    match genaryx_signing::verify_es256_b64(pubkey_b64, &tampered, sig_b64) {
        Err(_) => println!("TAMPER-REJECTED: modified message correctly refused"),
        Ok(()) => {
            println!("DANGER: tampered message verified - broken verify path");
            std::process::exit(1);
        }
    }
}
