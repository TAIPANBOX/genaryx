//! Integration proof for `CloudClient` (Phase-1 wave 1, docs/PHASE1.md)
//! against a real running `tokenfuse-cloud`, standing it up locally exactly
//! as Phase-0 spike #2 did (`crates/signing/examples/pair_ack.rs`): built +
//! run from `~/Development/tokenfuse` with `TOKENFUSE_CLOUD_ALLOW_DEVKEY=1`
//! on a fresh ephemeral port, torn down after.
//!
//! Gated: if `~/Development/tokenfuse` isn't present, doesn't build, or never
//! answers `/healthz` within the timeout, this degrades to an `eprintln!`
//! skip message and an early return - a missing sibling checkout must never
//! turn `cargo test -p genaryx-connectors` red. The DTO-shape,
//! error-classification, and fail-closed-without-a-device checks in
//! `src/cloud_rest.rs`'s own unit tests cover the rest of the contract with
//! no network at all, so this file only proves what genuinely needs a live
//! server: pairing, reads, a signed mutation accepted, and a tampered one
//! rejected.
//!
//! Single test function by design (06 §... "keep any real-socket test
//! single-threaded and hermetic"): one ephemeral port, one server process,
//! one sequential story, mirroring `cloud_sse_test.rs`'s shape.

use genaryx_connectors::CloudClient;
use genaryx_signing::{SoftwareSigner, sign_mutation};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long to wait for `tokenfuse-cloud` to bind and answer `/healthz` once
/// spawned (the build itself, below, has already completed by this point).
const HEALTHZ_TIMEOUT: Duration = Duration::from_secs(30);

/// Kills and reaps the child on drop, including on a mid-test panic, so a
/// failed assertion never leaks a `tokenfuse-cloud` process holding the port.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// An ephemeral, currently-free TCP port: bind to port 0, read back what the
/// OS assigned, then release it immediately so `tokenfuse-cloud` can bind it.
/// A small TOCTOU race is inherent to this trick, but it is the standard one
/// and is what keeps this test hermetic (a fresh port every run) without a
/// hardcoded number that could collide with a concurrently running instance.
fn free_port() -> Option<u16> {
    TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
}

/// Best-effort locate `~/Development/tokenfuse`, the ground-truth repo this
/// task reads from. Read-only in spirit everywhere else in this task; here it
/// is only ever built and run (`cargo build` / the resulting binary), never
/// edited.
fn tokenfuse_repo() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(home).join("Development/tokenfuse");
    dir.join("Cargo.toml").is_file().then_some(dir)
}

/// Build `tokenfuse-cloud` (a no-op if the target is already current, as it
/// is on this box), then spawn the resulting binary directly rather than
/// `cargo run`, so [`ChildGuard`] holds the actual server process and killing
/// it is reliable, instead of potentially leaving an orphaned server behind a
/// killed `cargo` wrapper process.
fn build_and_spawn(repo: &Path, port: u16) -> Option<Child> {
    let build = Command::new("cargo")
        .args(["build", "--quiet", "-p", "tokenfuse-cloud"])
        .current_dir(repo)
        .status();
    match build {
        Ok(status) if status.success() => {}
        Ok(status) => {
            eprintln!(
                "cloud_rest_test: SKIPPING live-cloud checks: `cargo build -p tokenfuse-cloud` failed ({status})"
            );
            return None;
        }
        Err(e) => {
            eprintln!("cloud_rest_test: SKIPPING live-cloud checks: could not run cargo: {e}");
            return None;
        }
    }

    let binary = repo.join("target/debug/tokenfuse-cloud");
    if !binary.is_file() {
        eprintln!(
            "cloud_rest_test: SKIPPING live-cloud checks: build succeeded but {} is missing \
             (unexpected target layout?)",
            binary.display()
        );
        return None;
    }

    match Command::new(&binary)
        .env("TOKENFUSE_CLOUD_ALLOW_DEVKEY", "1")
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => Some(child),
        Err(e) => {
            eprintln!(
                "cloud_rest_test: SKIPPING live-cloud checks: failed to spawn {}: {e}",
                binary.display()
            );
            None
        }
    }
}

/// Stand up a fresh `tokenfuse-cloud` on an ephemeral port and wait for it to
/// answer `/healthz`. `None` (after an explanatory `eprintln!`) on any
/// failure along the way, so the caller can degrade gracefully instead of
/// failing the suite.
async fn try_start_cloud() -> Option<(ChildGuard, String)> {
    let Some(repo) = tokenfuse_repo() else {
        eprintln!("cloud_rest_test: SKIPPING live-cloud checks: ~/Development/tokenfuse not found");
        return None;
    };
    let Some(port) = free_port() else {
        eprintln!("cloud_rest_test: SKIPPING live-cloud checks: could not reserve a test port");
        return None;
    };
    let Some(mut child) = build_and_spawn(&repo, port) else {
        return None; // build_and_spawn already explained why.
    };

    let base = format!("http://127.0.0.1:{port}");
    let http = reqwest::Client::new();
    let deadline = Instant::now() + HEALTHZ_TIMEOUT;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            eprintln!(
                "cloud_rest_test: SKIPPING live-cloud checks: tokenfuse-cloud exited early ({status})"
            );
            return None;
        }
        if let Ok(resp) = http.get(format!("{base}/healthz")).send().await
            && resp.status().is_success()
        {
            return Some((ChildGuard(child), base));
        }
        if Instant::now() >= deadline {
            eprintln!(
                "cloud_rest_test: SKIPPING live-cloud checks: tokenfuse-cloud never answered \
                 /healthz within {HEALTHZ_TIMEOUT:?}"
            );
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn pair_read_signed_mutations_and_tamper_reject_against_live_cloud() {
    let Some((_guard, base)) = try_start_cloud().await else {
        return; // Already explained why via eprintln! above.
    };

    // ---- pair_new alone (Phase 5 W2, D12.2a step 1): the desktop Pocket
    // panel's own call shape - mint a code and stop, no redeem, distinct from
    // pair()'s single-shot mint-then-redeem below. ----
    let mut client = CloudClient::new(&base, "devkey").expect("build CloudClient");
    let minted = client
        .pair_new("devkey")
        .await
        .expect("pair_new against the live cloud");
    assert_eq!(
        minted.code.len(),
        8,
        "devices::pairing_code() mints an 8-char code: {:?}",
        minted.code
    );
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("post-epoch clock")
        .as_secs() as i64;
    assert!(
        minted.expires_unix > now_unix,
        "a freshly minted code must expire in the future: {} vs now {now_unix}",
        minted.expires_unix
    );

    // ---- pair a portable SoftwareSigner (CI-safe: no Secure Enclave needed) ----
    let signer = SoftwareSigner::generate().expect("generate a software P-256 key");
    let paired = client
        .pair("devkey", &signer)
        .await
        .expect("pair a device against the live cloud");
    assert_eq!(
        paired.org, "default",
        "TOKENFUSE_CLOUD_ALLOW_DEVKEY's devkey fallback resolves org=default"
    );
    assert_eq!(paired.role, "admin");
    assert!(!paired.device_id.is_empty());
    assert!(!paired.device_token.is_empty());

    // Precompute a genuine signature to then corrupt (mirrors pair_ack.rs's
    // Tamper B), while `signer` is still borrowed rather than moved into the
    // client below - `SoftwareSigner` isn't `Clone`, so this has to happen
    // before `attach_device` takes ownership of it.
    let run_id = format!("connectors-test-{}", std::process::id());
    let kill_path = format!("/v1/runs/{run_id}/kill");
    let mut tampered = sign_mutation(&signer, "POST", &kill_path, b"").expect("sign for tamper");
    let mut sig_bytes = tampered.sig_b64.into_bytes();
    let flip_at = sig_bytes.len() / 2;
    sig_bytes[flip_at] = if sig_bytes[flip_at] == b'A' {
        b'B'
    } else {
        b'A'
    };
    tampered.sig_b64 = String::from_utf8(sig_bytes).expect("byte flip stays valid utf8 base64");

    client.attach_device(
        paired.device_id.clone(),
        paired.device_token.clone(),
        Box::new(signer),
    );
    assert!(client.has_device());

    // ---- reads: a freshly spawned process has an empty, well-shaped org view ----
    let summary = client.summary().await.expect("GET /v1/summary");
    assert_eq!(summary.runs, 0);
    assert_eq!(summary.calls, 0);
    assert_eq!(summary.spent_microusd, 0);

    let runs = client.runs().await.expect("GET /v1/runs");
    assert!(runs.is_empty());

    let savings = client.savings().await.expect("GET /v1/savings");
    assert_eq!(savings.total_saved_microusd, 0);

    let incidents = client.incidents().await.expect("GET /v1/incidents");
    assert!(incidents.is_empty());

    // ---- signed kill: 200, response echoes the run id ----
    let killed = client
        .kill_run(&run_id)
        .await
        .expect("signed kill_run must be accepted (200)");
    assert_eq!(killed.killed, run_id);

    // ---- signed budget: 200, proving the JSON-body signing path too (the
    // exact body bytes signed must be the exact body bytes sent) ----
    let budget = client
        .set_budget(&run_id, 12.5)
        .await
        .expect("signed set_budget must be accepted (200)");
    assert_eq!(budget.run, run_id);
    assert_eq!(budget.budget_micros, 12_500_000);

    // ---- task #21: an id carrying reserved characters survives the round
    // trip. This is the regression proof for the desktop twin of mobile #15:
    // the client percent-encodes the id into ONE path segment, signs THAT
    // path, and sends exactly those bytes, while the Cloud verifies the
    // signature over `uri.path()` (the raw encoded path, `http.rs::kill`'s
    // `authorize_mutation("POST", uri.path(), ...)`) and only then decodes the
    // segment back via axum's `Path(run)`. Signing the raw id and letting
    // `url` encode it afterwards - the old shape - desyncs the two and this
    // call comes back 403 signature_invalid, i.e. a kill that does not kill.
    //
    // The echoed `killed` is the DECODED id, so asserting it equals the
    // original proves both halves at once: the signature verified over the
    // encoded path, and the server recovered the id byte for byte.
    //
    // Three ids on purpose: the old code broke differently for each, and only
    // the first is the signature desync this task is named after. Each failure
    // below was OBSERVED against this live Cloud by reintroducing the raw
    // interpolation, not reasoned about:
    //   - space + non-ASCII: the path still matched the route, so the request
    //     reached signature verification and was rejected 403
    //     signature_invalid. For `kill` that is a kill that does not kill.
    //   - a `#`: `url` reads it as the start of the fragment, so everything
    //     after it - including the `/kill` verb itself - falls off the path.
    //     404.
    //   - a `/`: the raw id opens an extra path segment, so the request misses
    //     the route entirely. 404, a mutation that quietly hit nothing.
    for nasty_run in [
        format!("connectors-test-{} зупинка 7", std::process::id()),
        format!("connectors-test-{}-зупинка #7 a b", std::process::id()),
        format!("connectors-test-{}/зупинка #7 a b", std::process::id()),
    ] {
        let killed = client
            .kill_run(&nasty_run)
            .await
            .unwrap_or_else(|e| panic!("a run id with reserved characters must verify: {e:?}"));
        assert_eq!(
            killed.killed, nasty_run,
            "the Cloud must recover exactly the id that was sent"
        );

        let budget = client
            .set_budget(&nasty_run, 3.25)
            .await
            .unwrap_or_else(|e| panic!("the same id must verify with a JSON body too: {e:?}"));
        assert_eq!(budget.run, nasty_run);
        assert_eq!(budget.budget_micros, 3_250_000);
    }

    // An id that cannot be a single path segment at all fails closed in the
    // client, before any I/O - never a request against a different resource.
    let err = client.kill_run("..").await.unwrap_err();
    assert!(
        matches!(
            err,
            genaryx_connectors::ConnectorError::InvalidPathSegment(_)
        ),
        "a `..` run id must fail closed, got: {err:?}"
    );

    // ---- tamper: a genuine signature with one corrupted byte must be
    // rejected 403 signature_invalid. Sent with raw reqwest (bypassing
    // CloudClient on purpose): CloudClient's own public API has no way to
    // send a bad signature in the first place - that IS the fail-closed
    // guarantee under test - so proving the SERVER independently rejects one
    // requires going around it, exactly like spike #2's `pair_ack.rs` did.
    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{base}{kill_path}"))
        .bearer_auth(&paired.device_token)
        .header("X-Fuse-Device", paired.device_id.as_str())
        .header("X-Fuse-TS", tampered.ts.as_str())
        .header("X-Fuse-Nonce", tampered.nonce.as_str())
        .header("X-Fuse-Sig", tampered.sig_b64.as_str())
        .body("")
        .send()
        .await
        .expect("tamper request reaches the live cloud");
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    assert_eq!(
        status.as_u16(),
        403,
        "tampered signature must be rejected, got: {body}"
    );
    assert!(
        body.contains("signature_invalid"),
        "expected a signature_invalid body, got: {body}"
    );
}
