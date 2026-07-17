//! Integration proof for `WardryxClient` (Phase-2 wave 1) against a real
//! running `wardryx serve`, mirroring `cloud_rest_test.rs`'s shape: built +
//! run from `~/Development/wardryx` (a Go repo, sibling of `tokenfuse` and
//! this repo under `~/Development`), on a fresh ephemeral port, torn down
//! after.
//!
//! Two differences from `cloud_rest_test.rs`, both forced by wardryx being
//! a separate Go binary rather than another crate in this workspace:
//!  - the "build" step shells out to `go build -o <bin> ./cmd/wardryx`
//!    instead of `cargo build -p ...`, and the resulting binary is written
//!    to a scratch temp path (not `target/debug`, which belongs to this
//!    Rust workspace) - cleaned up on drop by [`ChildGuard`] alongside the
//!    process itself;
//!  - the bearer this test authenticates with, `"tk_test"`, is deliberately
//!    the BARE token half of `WARDRYX_KEYS="tk_test:test-org:admin"`, never
//!    the full `token:org:role` spec - see `WardryxClient`'s own doc
//!    comment and this repo's `killer_demo_test.rs` module doc for why that
//!    distinction is exactly the bug (issue #20) this project has already
//!    been bitten by once, against this same server's auth code.
//!
//! Gated exactly like `cloud_rest_test.rs`: if `~/Development/wardryx`
//! isn't present, `go` isn't on `PATH`, the build fails, or the spawned
//! server never answers `/healthz` within the timeout, this degrades to an
//! `eprintln!` skip message and an early return - a missing sibling
//! checkout or missing Go toolchain must never turn `cargo test -p
//! genaryx-connectors` red. The DTO-shape, error-classification, and
//! `ApprovalTokenClaims` decode/expiry checks in `src/wardryx.rs`'s own
//! unit tests cover the rest of the contract with no network and no Go
//! toolchain at all; this file only proves what genuinely needs a live
//! server: a policy write reaching the live PDP, a full
//! hold -> grant -> token -> allow cycle, the token's cost-ceiling and
//! already-decided edges, and the deny path.
//!
//! Single test function by design, same rationale as `cloud_rest_test.rs`:
//! one ephemeral port, one server process, one sequential story.

use genaryx_connectors::{ApprovalTokenClaims, ApprovalVerdict, DecideRequest, WardryxError};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

/// How long to wait for `wardryx serve` to bind and answer `/healthz` once
/// spawned (the `go build`, above, has already completed by this point).
const HEALTHZ_TIMEOUT: Duration = Duration::from_secs(30);

/// The BARE half of `WARDRYX_KEYS`'s `"tk_test:test-org:admin"` entry - see
/// this file's module doc comment for why it must be bare, not the full
/// spec.
const BEARER: &str = "tk_test";
const WARDRYX_KEYS: &str = "tk_test:test-org:admin";
const APPROVAL_SECRET: &str = "genaryx-wardryx-test-secret-0123456789";

/// Kills and reaps the child on drop, including on a mid-test panic, so a
/// failed assertion never leaks a `wardryx` process holding the port; also
/// removes the scratch binary and events file this test wrote to
/// `std::env::temp_dir()` (unlike `cloud_rest_test.rs`'s sibling-repo
/// `target/debug` binary, these are ours to clean up, not a normal build
/// artifact).
struct ChildGuard {
    child: Child,
    bin_path: PathBuf,
    events_path: PathBuf,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.bin_path);
        let _ = std::fs::remove_file(&self.events_path);
    }
}

/// An ephemeral, currently-free TCP port: bind to port 0, read back what the
/// OS assigned, then release it immediately so `wardryx` can bind it. Same
/// trick (and the same small inherent TOCTOU race) as `cloud_rest_test.rs`'s
/// `free_port` - and it doubles as this test's answer to the "use an
/// uncommon port so a real 8090 stack is never touched" requirement: an
/// OS-assigned ephemeral port is never 8090 (wardryx's documented default)
/// and, unlike a single hardcoded alternate port, cannot collide with a
/// concurrently running instance of this same test either.
fn free_port() -> Option<u16> {
    TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
}

/// Best-effort locate `~/Development/wardryx`, the ground-truth repo this
/// task reads from - a sibling of `~/Development/tokenfuse`
/// (`cloud_rest_test.rs`'s own ground-truth repo) and of this repo,
/// discovered the same way: `$HOME/Development/<name>`, confirmed present
/// by checking for that repo's own manifest file (`go.mod` here, in place
/// of `cloud_rest_test.rs`'s `Cargo.toml`, since wardryx is a Go module,
/// not a Rust crate). Read-only in spirit everywhere else in this task;
/// here it is only ever built and run, never edited.
fn wardryx_repo() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(home).join("Development/wardryx");
    dir.join("go.mod").is_file().then_some(dir)
}

/// `go build -o bin_path ./cmd/wardryx`, run inside `repo`. `Err` carries a
/// human-readable reason, never a panic - the caller turns it into an
/// `eprintln!` skip.
fn build_wardryx(repo: &Path, bin_path: &Path) -> Result<(), String> {
    match Command::new("go")
        .arg("build")
        .arg("-o")
        .arg(bin_path)
        .arg("./cmd/wardryx")
        .current_dir(repo)
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!(
            "`go build -o {} ./cmd/wardryx` failed ({status})",
            bin_path.display()
        )),
        Err(e) => Err(format!("could not run `go`: {e}")),
    }
}

/// Spawn the already-built `wardryx` binary directly (never `go run`), so
/// [`ChildGuard`] holds the actual server process and killing it is
/// reliable - mirrors `cloud_rest_test.rs::build_and_spawn`'s identical
/// rationale for spawning `tokenfuse-cloud`'s binary rather than wrapping it
/// in `cargo run`.
fn spawn_wardryx(bin_path: &Path, addr: &str, events_path: &Path) -> Option<Child> {
    Command::new(bin_path)
        .arg("serve")
        .arg("-addr")
        .arg(addr)
        .arg("-events")
        .arg(events_path)
        .env("WARDRYX_KEYS", WARDRYX_KEYS)
        .env("WARDRYX_APPROVAL_SECRET", APPROVAL_SECRET)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

/// Stand up a fresh `wardryx serve` on an ephemeral port and wait for it to
/// answer `/healthz`. `None` (after an explanatory `eprintln!`) on any
/// failure along the way - missing repo, missing `go`, a failed build, a
/// failed spawn, or a health-check timeout - so the caller can degrade
/// gracefully instead of failing the suite.
async fn try_start_wardryx() -> Option<(ChildGuard, String)> {
    let Some(repo) = wardryx_repo() else {
        eprintln!("wardryx_test: SKIPPING live-wardryx checks: ~/Development/wardryx not found");
        return None;
    };
    let Some(port) = free_port() else {
        eprintln!("wardryx_test: SKIPPING live-wardryx checks: could not reserve a test port");
        return None;
    };

    let tmp = std::env::temp_dir();
    let unique = format!("genaryx-wardryx-test-{}-{port}", std::process::id());
    let bin_path = tmp.join(&unique);
    let events_path = tmp.join(format!("{unique}.ndjson"));

    if let Err(reason) = build_wardryx(&repo, &bin_path) {
        eprintln!("wardryx_test: SKIPPING live-wardryx checks: {reason}");
        return None;
    }
    if !bin_path.is_file() {
        eprintln!(
            "wardryx_test: SKIPPING live-wardryx checks: build succeeded but {} is missing",
            bin_path.display()
        );
        return None;
    }

    let addr = format!("127.0.0.1:{port}");
    let Some(mut child) = spawn_wardryx(&bin_path, &addr, &events_path) else {
        eprintln!(
            "wardryx_test: SKIPPING live-wardryx checks: failed to spawn {}",
            bin_path.display()
        );
        let _ = std::fs::remove_file(&bin_path);
        return None;
    };

    let base = format!("http://{addr}");
    let http = reqwest::Client::new();
    let deadline = Instant::now() + HEALTHZ_TIMEOUT;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            eprintln!(
                "wardryx_test: SKIPPING live-wardryx checks: wardryx exited early ({status})"
            );
            let _ = std::fs::remove_file(&bin_path);
            let _ = std::fs::remove_file(&events_path);
            return None;
        }
        if let Ok(resp) = http.get(format!("{base}/healthz")).send().await
            && resp.status().is_success()
        {
            return Some((
                ChildGuard {
                    child,
                    bin_path,
                    events_path,
                },
                base,
            ));
        }
        if Instant::now() >= deadline {
            eprintln!(
                "wardryx_test: SKIPPING live-wardryx checks: wardryx never answered /healthz within {HEALTHZ_TIMEOUT:?}"
            );
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&bin_path);
            let _ = std::fs::remove_file(&events_path);
            return None;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn hold_grant_token_replay_and_deny_cycle_against_live_wardryx() {
    let Some((_guard, base)) = try_start_wardryx().await else {
        return; // Already explained why via eprintln! above.
    };

    let client =
        genaryx_connectors::WardryxClient::new(&base, BEARER).expect("build WardryxClient");

    // ---- 1. healthz -----------------------------------------------------
    client
        .healthz()
        .await
        .expect("GET /healthz must succeed against a live wardryx");

    // ---- 2. put_policy, list_policies, get_policy ------------------------
    let policy = genaryx_connectors::Policy {
        target: "agent://test-org/*".to_string(),
        require_human_above_usd: 1.0,
        deny_tool: vec!["shell_exec".to_string()],
        deny_above_usd: 1000.0,
        ..Default::default()
    };
    let put = client
        .put_policy("demo", &policy)
        .await
        .expect("PUT /v1/policies/demo");
    assert_eq!(put.id, "demo");
    assert_eq!(put.policy.target, "agent://test-org/*");
    assert_eq!(put.policy.deny_tool, vec!["shell_exec".to_string()]);

    let listed = client.list_policies().await.expect("GET /v1/policies");
    assert!(
        listed.iter().any(|p| p.id == "demo"),
        "the freshly PUT policy must appear in the list"
    );

    let fetched = client
        .get_policy("demo")
        .await
        .expect("GET /v1/policies/demo");
    assert_eq!(fetched.policy.target, "agent://test-org/*");

    // ---- 3. decide -> hold ------------------------------------------------
    // The server was started with no -policy file (see spawn_wardryx), so
    // "demo" is the ONLY policy the live PDP knows about, and it can only
    // know about it because put_policy above swapped the live engine
    // (WardryxClient::put_policy's own doc comment). If that swap hadn't
    // happened, this decide would fall through to the "no policy targets
    // this agent" allow, not hold - so this assertion is itself the proof
    // the spec asked for that PUT reaches the live PDP, not just the store.
    let decide_req = DecideRequest {
        agent_id: "agent://test-org/payments".to_string(),
        run_id: "run-1".to_string(),
        tool_names: vec!["charge".to_string()],
        est_cost_usd: 50.0,
        ..Default::default()
    };
    let hold = client
        .decide(&decide_req)
        .await
        .expect("POST /v1/decide over threshold");
    assert_eq!(hold.decision, "hold");
    assert!(hold.approval_token_required);
    assert!(!hold.approval_id.is_empty());
    assert!(!hold.policy_version.is_empty());

    // ---- 4. list_approvals -----------------------------------------------
    let approvals = client.list_approvals().await.expect("GET /v1/approvals");
    let pending = approvals
        .iter()
        .find(|a| a.approval_id == hold.approval_id)
        .expect("the just-created hold must appear in list_approvals");
    assert!(pending.pending);
    let est = pending
        .est_cost_usd()
        .expect("context[\"est_cost_usd\"] must be present");
    assert!((est - 50.0).abs() < 0.01);
    assert!(
        pending
            .tool_names()
            .unwrap_or_default()
            .iter()
            .any(|t| t == "charge")
    );
    assert!(!pending.reason().unwrap_or_default().is_empty());

    // ---- 5. decide_approval grant ------------------------------------------
    let granted = client
        .decide_approval(
            &hold.approval_id,
            ApprovalVerdict::Grant,
            "user://test-org/alice",
        )
        .await
        .expect("grant the pending approval");
    assert_eq!(granted.decision, "grant");
    let token = granted
        .approval_token
        .clone()
        .expect("a grant must return an approval_token");

    // ---- 6. decode claims ---------------------------------------------------
    let claims = ApprovalTokenClaims::decode(&token).expect("decode approval_token claims");
    assert_eq!(claims.agent_id, "agent://test-org/payments");
    assert_eq!(claims.run_id, "run-1");
    assert_eq!(claims.tools, vec!["charge".to_string()]);
    assert!((claims.cost_ceiling_usd() - 50.0).abs() < 0.01);
    let now = SystemTime::now();
    assert!(
        !claims.is_expired(now),
        "a freshly minted token must not be expired"
    );
    let ttl = claims.ttl_remaining(now);
    assert!(
        ttl > Duration::ZERO && ttl <= Duration::from_secs(600),
        "ttl_remaining must be in (0, 10min] for a token just minted with the 10-minute default TTL, got {ttl:?}"
    );

    // ---- 7. decide with the valid token -> allow ---------------------------
    let allow_req = DecideRequest {
        approval_token: token.clone(),
        ..decide_req.clone()
    };
    let allow = client
        .decide(&allow_req)
        .await
        .expect("decide with a valid approval_token");
    assert_eq!(allow.decision, "allow");
    assert_eq!(
        allow.policy_version, hold.policy_version,
        "the same live policy set must report the same policy_version across calls"
    );

    // ---- 8. same token, cost over its ceiling -> deny ----------------------
    let over_ceiling_req = DecideRequest {
        est_cost_usd: 500.0,
        approval_token: token.clone(),
        ..decide_req.clone()
    };
    let over_ceiling = client
        .decide(&over_ceiling_req)
        .await
        .expect("decide over the token's cost ceiling");
    assert_eq!(over_ceiling.decision, "deny");

    // ---- 9. deny_tool -> deny, reason mentions the tool --------------------
    let deny_tool_req = DecideRequest {
        agent_id: "agent://test-org/ops".to_string(),
        run_id: "run-x".to_string(),
        tool_names: vec!["shell_exec".to_string()],
        est_cost_usd: 0.5,
        ..Default::default()
    };
    let deny_tool = client
        .decide(&deny_tool_req)
        .await
        .expect("decide a deny_tool-listed tool");
    assert_eq!(deny_tool.decision, "deny");
    assert!(
        deny_tool.reason.contains("shell_exec"),
        "reason should name the denied tool, got: {}",
        deny_tool.reason
    );

    // ---- 10. decide_approval again -> ApprovalAlreadyDecided ---------------
    let err = client
        .decide_approval(
            &hold.approval_id,
            ApprovalVerdict::Grant,
            "user://test-org/alice",
        )
        .await
        .expect_err("deciding an already-decided approval a second time must fail");
    assert!(
        matches!(err, WardryxError::ApprovalAlreadyDecided),
        "expected ApprovalAlreadyDecided, got {err:?}"
    );

    // ---- 11. deny path -------------------------------------------------------
    let hold2_req = DecideRequest {
        agent_id: "agent://test-org/payments".to_string(),
        run_id: "run-2".to_string(),
        tool_names: vec!["charge".to_string()],
        est_cost_usd: 50.0,
        ..Default::default()
    };
    let hold2 = client.decide(&hold2_req).await.expect("second hold, run-2");
    assert_eq!(hold2.decision, "hold");

    let denied = client
        .decide_approval(
            &hold2.approval_id,
            ApprovalVerdict::Deny,
            "user://test-org/bob",
        )
        .await
        .expect("deny the second approval");
    assert_eq!(denied.decision, "deny");
    assert_eq!(
        denied.approval_token, None,
        "a deny must never carry an approval_token"
    );

    let approvals_after = client
        .list_approvals()
        .await
        .expect("GET /v1/approvals after the deny");
    let decided2 = approvals_after
        .iter()
        .find(|a| a.approval_id == hold2.approval_id)
        .expect("the just-denied approval must appear in the list");
    assert_eq!(decided2.decision.as_deref(), Some("deny"));
    assert!(!decided2.pending);

    // ---- cleanup / bonus coverage: delete_policy's 204 contract -----------
    client
        .delete_policy("demo")
        .await
        .expect("DELETE /v1/policies/demo");
    let after_delete = client.get_policy("demo").await;
    assert!(
        matches!(after_delete, Err(WardryxError::Api { status: 404, .. })),
        "policy must be gone after delete, got {after_delete:?}"
    );
}
