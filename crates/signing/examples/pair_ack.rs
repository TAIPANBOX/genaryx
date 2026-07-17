//! Phase-0 spike #2 live driver: full device-pairing handshake + signed-ack
//! against a real running `tokenfuse-cloud` (07 §4.2).
//!
//! Stand the Cloud up first (from ~/Development/tokenfuse, read-only):
//! ```sh
//! TOKENFUSE_CLOUD_ALLOW_DEVKEY=1 PORT=18080 cargo run -p tokenfuse-cloud
//! ```
//! then:
//! ```sh
//! cargo run -p genaryx-signing --example pair_ack
//! ```
//! Env: `TOKENFUSE_CLOUD_URL` (default `http://127.0.0.1:18080`),
//! `TOKENFUSE_ADMIN_KEY` (default `devkey`), `GENARYX_SIGNER=software` to
//! force the portable signer instead of preferring the Secure Enclave.
//!
//! Exit code 0 only when the Cloud ACCEPTS the two genuine signed mutations
//! (empty-body kill + UTF-8-body budget) AND REJECTS all four tampered
//! variants (wrong-path signature, corrupted signature, replayed nonce,
//! stale timestamp), and the audit trail attributes the mutations to the
//! paired device.

use genaryx_signing::es256::random_hex;
use genaryx_signing::{Es256Signer, SignedMutation, sign_mutation, sign_mutation_at};

fn build_signer() -> Box<dyn Es256Signer> {
    let force_software = std::env::var("GENARYX_SIGNER").is_ok_and(|v| v == "software");
    #[cfg(target_os = "macos")]
    if !force_software {
        let (signer, fallback) =
            genaryx_signing::enclave::SecKeySigner::generate_preferring_enclave()
                .expect("no SecKey signer available at all");
        match &fallback {
            None => println!(
                "signer: {} ({})",
                signer.assurance().label(),
                signer.assurance().detail()
            ),
            Some(reason) => println!(
                "signer: {} ({}); enclave unavailable: {reason}",
                signer.assurance().label(),
                signer.assurance().detail()
            ),
        }
        return Box::new(signer);
    }
    let signer = genaryx_signing::SoftwareSigner::generate().expect("software P-256 keygen");
    println!(
        "signer: {} ({})",
        signer.assurance().label(),
        signer.assurance().detail()
    );
    Box::new(signer)
}

struct Cloud {
    http: reqwest::Client,
    base: String,
}

impl Cloud {
    async fn post(
        &self,
        path: &str,
        bearer: &str,
        body: String,
        signed: Option<(&str, &SignedMutation)>,
    ) -> (u16, String) {
        let mut req = self
            .http
            .post(format!("{}{path}", self.base))
            .header("authorization", format!("Bearer {bearer}"))
            .body(body);
        if let Some((device_id, m)) = signed {
            req = req
                .header("x-fuse-device", device_id)
                .header("x-fuse-ts", &m.ts)
                .header("x-fuse-nonce", &m.nonce)
                .header("x-fuse-sig", &m.sig_b64);
        }
        let resp = req.send().await.expect("cloud unreachable mid-run");
        let status = resp.status().as_u16();
        (status, resp.text().await.unwrap_or_default())
    }

    async fn get(&self, path: &str, bearer: &str) -> (u16, String) {
        let resp = self
            .http
            .get(format!("{}{path}", self.base))
            .header("authorization", format!("Bearer {bearer}"))
            .send()
            .await
            .expect("cloud unreachable mid-run");
        let status = resp.status().as_u16();
        (status, resp.text().await.unwrap_or_default())
    }
}

/// One checked step: print the transcript line, count a failure if the status
/// is not the expected one.
fn check(failures: &mut u32, what: &str, got: u16, want: u16, body: &str) {
    let ok = got == want;
    if !ok {
        *failures += 1;
    }
    println!(
        "  [{}] {what}: HTTP {got} (want {want}) {body}",
        if ok { "PASS" } else { "FAIL" }
    );
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let base =
        std::env::var("TOKENFUSE_CLOUD_URL").unwrap_or_else(|_| "http://127.0.0.1:18080".into());
    let admin_key = std::env::var("TOKENFUSE_ADMIN_KEY").unwrap_or_else(|_| "devkey".into());
    let cloud = Cloud {
        http: reqwest::Client::new(),
        base: base.clone(),
    };
    let mut failures = 0u32;

    println!("== spike #2 signed-ack vs live tokenfuse-cloud at {base} ==");
    let (st, body) = cloud.get("/healthz", &admin_key).await;
    check(&mut failures, "healthz", st, 200, &body);

    let signer = build_signer();
    let pubkey_b64 = signer.public_key_b64().expect("pubkey export");
    println!("device pubkey (X9.63 b64): {pubkey_b64}");

    // 1) Admin issues a one-time pairing code.
    let (st, body) = cloud
        .post("/v1/pair/new", &admin_key, "{}".into(), None)
        .await;
    check(
        &mut failures,
        "POST /v1/pair/new (admin org key)",
        st,
        200,
        &body,
    );
    let v: serde_json::Value = serde_json::from_str(&body).expect("pair/new json");
    let code = v["code"].as_str().expect("pairing code").to_string();

    // 2) The device redeems it with its public key. The code is the credential.
    let pair_req = serde_json::json!({
        "code": code,
        "pubkey_b64": pubkey_b64,
        "platform": "macos",
        "name": format!("genaryx-spike2-rust ({})", signer.assurance().label()),
    });
    let (st, body) = cloud
        .post("/v1/pair", &admin_key, pair_req.to_string(), None)
        .await;
    check(
        &mut failures,
        "POST /v1/pair (redeem code + pubkey)",
        st,
        200,
        &body,
    );
    let v: serde_json::Value = serde_json::from_str(&body).expect("pair json");
    let device_id = v["device_id"].as_str().expect("device_id").to_string();
    let device_token = v["device_token"]
        .as_str()
        .expect("device_token")
        .to_string();
    println!(
        "paired: device_id={device_id} org={} role={}",
        v["org"], v["role"]
    );

    // 3) Genuine signed mutation #1: empty-body kill. THE signed-ack.
    let run = format!("spike2-rust-{}", random_hex(4).expect("nonce"));
    let kill_path = format!("/v1/runs/{run}/kill");
    let m = sign_mutation(signer.as_ref(), "POST", &kill_path, b"").expect("sign kill");
    let (st, body) = cloud
        .post(
            &kill_path,
            &device_token,
            String::new(),
            Some((&device_id, &m)),
        )
        .await;
    check(&mut failures, "signed kill (genuine)", st, 200, &body);

    // 4) Genuine signed mutation #2: non-empty multibyte UTF-8 body - the
    //    exact pinned cross-language vector body - through the body-hash line.
    let budget_path = format!("/v1/runs/{run}/budget");
    let budget_body = "{\"budget_usd\":12.5,\"note\":\"обмеження діє\"}";
    let m = sign_mutation(
        signer.as_ref(),
        "POST",
        &budget_path,
        budget_body.as_bytes(),
    )
    .expect("sign budget");
    let (st, body) = cloud
        .post(
            &budget_path,
            &device_token,
            budget_body.into(),
            Some((&device_id, &m)),
        )
        .await;
    check(
        &mut failures,
        "signed budget (genuine, UTF-8 body)",
        st,
        200,
        &body,
    );

    // 5) Tamper A: signature made for a DIFFERENT path, sent to the real one.
    let m =
        sign_mutation(signer.as_ref(), "POST", "/v1/runs/other-run/kill", b"").expect("sign other");
    let (st, body) = cloud
        .post(
            &kill_path,
            &device_token,
            String::new(),
            Some((&device_id, &m)),
        )
        .await;
    check(
        &mut failures,
        "tampered canonical (wrong path) rejected",
        st,
        403,
        &body,
    );

    // 6) Tamper B: one corrupted base64 signature byte.
    let mut m = sign_mutation(signer.as_ref(), "POST", &kill_path, b"").expect("sign");
    let mut sig = m.sig_b64.into_bytes();
    sig[10] = if sig[10] == b'A' { b'B' } else { b'A' };
    m.sig_b64 = String::from_utf8(sig).expect("still utf-8");
    let (st, body) = cloud
        .post(
            &kill_path,
            &device_token,
            String::new(),
            Some((&device_id, &m)),
        )
        .await;
    check(
        &mut failures,
        "corrupted signature rejected",
        st,
        403,
        &body,
    );

    // 7) Tamper C: replay - a fresh VALID signature reusing a spent nonce.
    let nonce = random_hex(8).expect("nonce");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs()
        .to_string();
    let m = sign_mutation_at(signer.as_ref(), "POST", &kill_path, b"", &ts, &nonce).expect("sign");
    let (st, body) = cloud
        .post(
            &kill_path,
            &device_token,
            String::new(),
            Some((&device_id, &m)),
        )
        .await;
    check(&mut failures, "first use of nonce accepted", st, 200, &body);
    let m = sign_mutation_at(signer.as_ref(), "POST", &kill_path, b"", &ts, &nonce).expect("sign");
    let (st, body) = cloud
        .post(
            &kill_path,
            &device_token,
            String::new(),
            Some((&device_id, &m)),
        )
        .await;
    check(&mut failures, "replayed nonce rejected", st, 403, &body);

    // 8) Tamper D: stale timestamp (> 120s skew), signature otherwise valid.
    let stale = (ts.parse::<i64>().expect("ts") - 1000).to_string();
    let m = sign_mutation_at(
        signer.as_ref(),
        "POST",
        &kill_path,
        b"",
        &stale,
        &random_hex(8).expect("nonce"),
    )
    .expect("sign");
    let (st, body) = cloud
        .post(
            &kill_path,
            &device_token,
            String::new(),
            Some((&device_id, &m)),
        )
        .await;
    check(&mut failures, "stale timestamp rejected", st, 403, &body);

    // 9) Closing evidence: the audit chain attributes the mutations to the
    //    paired device, not to the admin key.
    let (st, body) = cloud.get("/v1/audit", &admin_key).await;
    check(&mut failures, "GET /v1/audit", st, 200, &body);
    let entries: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    let device_actor = format!("device:{device_id}");
    let attributed = entries
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|e| e["actor"] == device_actor.as_str())
                .count()
        })
        .unwrap_or(0);
    println!(
        "  [{}] audit entries attributed to {device_actor}: {attributed} (want >= 3: pair uses the device id itself)",
        if attributed >= 3 { "PASS" } else { "FAIL" }
    );
    if attributed < 3 {
        failures += 1;
    }

    if failures == 0 {
        println!("== SIGNED-ACK PROVEN: all checks passed ==");
    } else {
        println!("== {failures} check(s) FAILED ==");
        std::process::exit(1);
    }
}
