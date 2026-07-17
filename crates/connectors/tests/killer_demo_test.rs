//! Killer-demo end-to-end acceptance test for the Phase-1 exit gate (#20).
//!
//! `cloud_rest_test.rs` proves `CloudClient` against a `tokenfuse-cloud`
//! spawned directly with `TOKENFUSE_CLOUD_ALLOW_DEVKEY=1` (never through
//! `taipan`). `command_test.rs` proves `command::record` with no network at
//! all. Neither proves the actual product path: an operator runs `taipan
//! up`, a console auto-discovers the environment `taipan` just wrote, and
//! pairs. That auto-discovery path is where issue #20 lived: `taipan` minted
//! `token:org:role` keys and wrote the FULL spec as the keyfile secret, but
//! the server (tokenfuse `parse_keys`, wardryx auth) indexes its key map by
//! the bare token before the first colon, so a `token:org:role` bearer never
//! matched and BOTH reads and pairing 401'd. Fixed in `taipan` (keys.rs +
//! commands/up.rs) by writing the bare token as the keyfile secret while still
//! handing the full spec to the server's key env. This test drives that real,
//! default minted path (no `--devkey`) end to end through the real CLI:
//!
//! 1. Build `taipan` (`~/Development/taipan`) and run `taipan up
//!    --name killerdemo-<pid>`: builds/spawns the real gateway + cloud
//!    binaries from `~/Development/tokenfuse`, waits for both `/healthz`,
//!    writes the descriptor + keyfile.
//! 2. Auto-discovery: read the descriptor and follow it into the keyfile,
//!    the same two-file shape `crates/ffi/src/cloud/env.rs::discover` and
//!    `apps/desktop/src-tauri/src/money/env.rs::discover` both implement.
//!    This crate cannot depend on either (both depend on `genaryx-connectors`,
//!    not the reverse), so the algorithm is mirrored here directly against
//!    the real files `taipan` just wrote, instead of re-imported.
//! 3. Build a `CloudClient`, pair a `SoftwareSigner` against the discovered
//!    bearer (`CloudClient::pair`) - the exact call that used to 401 - then
//!    `summary()` and a signed `kill_run()`.
//! 4. `genaryx_core::command::record(...)` journals the outcome as a
//!    `console_command` bus event; the appended line is read back and
//!    checked against the real `Conformer`.
//! 5. `taipan down --name killerdemo-<pid>`, then confirm every taipan-owned
//!    file for this environment is gone and no process it started survives.
//!
//! Gated like `cloud_rest_test.rs`: any failure getting the live stack up
//! (missing `~/Development/taipan` or `~/Development/tokenfuse` checkout, a
//! build failure, the fixed 4100/8080 ports already busy, a health-check
//! timeout) degrades to an `eprintln!` skip rather than a red `cargo test -p
//! genaryx-connectors`, so CI without the sibling repos stays green.
//! Everything downstream of "the stack is up" is a hard assertion: this test
//! exists specifically to prove the pairing path now works, not to prove it
//! works when it happens to.
//!
//! Hermetic: a pid-suffixed environment name, and a `Drop` guard that always
//! attempts `taipan down` even on a mid-test panic, so a failed assertion
//! never leaves the gateway/cloud pair running on the fixed ports for the
//! next test run. `taipan`'s services bind fixed ports regardless of
//! environment name (see `~/Development/taipan/src/services/{cloud,gateway}.rs`),
//! so only one `taipan up` (from this test or anything else) can be live on
//! this box at a time - the same single-instance constraint `taipan` itself
//! has today; this test checks for that and skips rather than racing it.

use genaryx_connectors::CloudClient;
use genaryx_core::store::Store;
use genaryx_core::{CommandRecord, Conformer, SchemaVersion};
use genaryx_signing::SoftwareSigner;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// `tokenfuse-cloud`'s fixed bind port under `taipan up` (see
/// `~/Development/taipan/src/services/cloud.rs::PORT`); unaffected by
/// `--name`.
const CLOUD_PORT: u16 = 8080;
/// The gateway's fixed bind port under `taipan up` (see
/// `~/Development/taipan/src/services/gateway.rs`); unaffected by `--name`.
const GATEWAY_PORT: u16 = 4100;

// ---- environment discovery ---------------------------------------------

/// Best-effort locate `~/Development/taipan`, the CLI repo this test builds
/// and drives. Read-only in spirit: only ever built and run, never edited.
fn taipan_repo() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(home).join("Development/taipan");
    dir.join("Cargo.toml").is_file().then_some(dir)
}

/// `~/.taipan`, matching `taipan::home::TaipanHome::discover` (`$HOME/.taipan`).
/// Mirrored here rather than imported: `taipan` is a `[[bin]]`-only crate
/// with no library target this test could depend on.
fn taipan_home() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".taipan"))
}

fn port_is_closed(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_err()
}

// ---- descriptor / keyfile wire shapes (read-only mirror of env::discover) --
// A behavioral mirror of `crates/ffi/src/cloud/env.rs` /
// `apps/desktop/src-tauri/src/money/env.rs`'s identical discovery twins:
// same two fields read off the descriptor, same "follow the ref's trailing
// label into the sibling keyfile" resolution. Only the fields actually read
// are modeled, same tolerance for unknown fields those modules document.

#[derive(Debug, Deserialize)]
struct DescriptorService {
    url: String,
}

#[derive(Debug, Default, Deserialize)]
struct DescriptorKeys {
    #[serde(default)]
    cloud_admin_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Descriptor {
    name: String,
    services: BTreeMap<String, DescriptorService>,
    #[serde(default)]
    keys: DescriptorKeys,
}

#[derive(Debug, Deserialize)]
struct KeyFile {
    #[serde(default)]
    secrets: BTreeMap<String, String>,
}

struct DiscoveredEnv {
    cloud_url: String,
    admin_bearer: String,
}

/// Resolve a `taipan up` descriptor into a Cloud URL and admin bearer: read
/// `services.cloud.url`, follow `keys.cloud_admin_ref`'s trailing label into
/// the sibling `<name>.keys.json`, and return the secret it names. `None` on
/// any missing/malformed step (never a panic) - the same fail-closed
/// contract the shells' own `discover()` has.
fn discover_taipan_env(descriptor_path: &Path) -> Option<DiscoveredEnv> {
    let bytes = std::fs::read(descriptor_path).ok()?;
    let descriptor: Descriptor = serde_json::from_slice(&bytes).ok()?;

    let cloud_url = descriptor.services.get("cloud")?.url.clone();
    let admin_ref = descriptor.keys.cloud_admin_ref?;
    let label = admin_ref.rsplit('/').next()?;

    let keys_path = descriptor_path.with_file_name(format!("{}.keys.json", descriptor.name));
    let key_bytes = std::fs::read(&keys_path).ok()?;
    let keyfile: KeyFile = serde_json::from_slice(&key_bytes).ok()?;
    let admin_bearer = keyfile.secrets.get(label)?.clone();

    Some(DiscoveredEnv {
        cloud_url,
        admin_bearer,
    })
}

// ---- pidfile (read-only mirror, for the post-teardown orphan check) -------

#[derive(Debug, Deserialize)]
struct PidEntry {
    pid: i32,
}

#[derive(Debug, Default, Deserialize)]
struct TaipanPidFile {
    #[serde(default)]
    processes: Vec<PidEntry>,
}

fn read_pidfile_pids(path: &Path) -> Vec<i32> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(pidfile) = serde_json::from_slice::<TaipanPidFile>(&bytes) else {
        return Vec::new();
    };
    pidfile.processes.into_iter().map(|p| p.pid).collect()
}

/// Whether `pid` (a process `taipan up` started) is still alive, checked
/// from outside taipan's own bookkeeping via `kill -0` (a pure
/// existence/permission probe, sends nothing) - the same POSIX semantics
/// `taipan`'s own `procutil::group_alive` relies on internally, reimplemented
/// here through the `kill` binary rather than the `libc` crate since this
/// crate has no other reason to depend on it.
fn pid_alive(pid: i32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ---- bring-up / tear-down --------------------------------------------------

/// One environment this test brought up via `taipan up`. `Drop`
/// always attempts `taipan down`, even on a mid-test panic, so a failed
/// assertion never leaves the gateway/cloud pair running on the fixed ports.
struct KillerDemoEnv {
    taipan_bin: PathBuf,
    name: String,
    descriptor_path: PathBuf,
    torn_down: bool,
}

impl KillerDemoEnv {
    fn pidfile_path(&self) -> PathBuf {
        self.descriptor_path
            .with_file_name(format!("{}.pid.json", self.name))
    }

    fn keyfile_path(&self) -> PathBuf {
        self.descriptor_path
            .with_file_name(format!("{}.keys.json", self.name))
    }

    /// Explicit, asserted teardown. Consumes `self` so the `Drop` safety net
    /// below never double-runs `taipan down` after a clean one.
    fn tear_down(mut self) -> std::process::Output {
        let out = Command::new(&self.taipan_bin)
            .args(["down", "--name", &self.name])
            .output()
            .expect("spawn `taipan down`");
        self.torn_down = true;
        out
    }
}

impl Drop for KillerDemoEnv {
    fn drop(&mut self) {
        if self.torn_down {
            return;
        }
        eprintln!(
            "killer_demo_test: Drop safety net tearing down '{}' (explicit tear_down was not reached, likely a mid-test panic)",
            self.name
        );
        let _ = Command::new(&self.taipan_bin)
            .args(["down", "--name", &self.name])
            .output();
    }
}

/// Build the `taipan` binary (a no-op if already current). `None` (after an
/// explanatory `eprintln!`) on any failure.
fn build_taipan(repo: &Path) -> Option<PathBuf> {
    let status = Command::new("cargo")
        .args(["build", "--quiet"])
        .current_dir(repo)
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("killer_demo_test: SKIPPING: `cargo build` for taipan failed ({s})");
            return None;
        }
        Err(e) => {
            eprintln!("killer_demo_test: SKIPPING: could not run cargo for taipan: {e}");
            return None;
        }
    }
    let bin = repo.join("target/debug/taipan");
    if !bin.is_file() {
        eprintln!(
            "killer_demo_test: SKIPPING: build succeeded but {} is missing (unexpected target layout?)",
            bin.display()
        );
        return None;
    }
    Some(bin)
}

/// Bring up `taipan up --name killerdemo-<pid>` and confirm the
/// descriptor landed. `None` (after an explanatory `eprintln!`) on any
/// failure - matches `cloud_rest_test.rs`'s gating philosophy: only "is the
/// live stack even up" degrades gracefully; everything downstream is a hard
/// assertion in the test body.
fn try_bring_up() -> Option<KillerDemoEnv> {
    let Some(repo) = taipan_repo() else {
        eprintln!("killer_demo_test: SKIPPING: ~/Development/taipan not found");
        return None;
    };
    let Some(home) = taipan_home() else {
        eprintln!("killer_demo_test: SKIPPING: $HOME not set");
        return None;
    };
    let Some(taipan_bin) = build_taipan(&repo) else {
        return None; // build_taipan already explained why.
    };

    if !port_is_closed(CLOUD_PORT) || !port_is_closed(GATEWAY_PORT) {
        eprintln!(
            "killer_demo_test: SKIPPING: port {CLOUD_PORT} or {GATEWAY_PORT} is already in use \
             (another taipan environment up?). taipan's gateway/cloud bind fixed ports \
             regardless of --name, so only one can run at a time on this box"
        );
        return None;
    }

    let name = format!("killerdemo-{}", std::process::id());
    let descriptor_path = home.join("environments").join(format!("{name}.json"));

    let up_result = Command::new(&taipan_bin)
        .args(["up", "--name", &name])
        .current_dir(&repo)
        .output();
    let output = match up_result {
        Ok(o) => o,
        Err(e) => {
            eprintln!("killer_demo_test: SKIPPING: failed to spawn `taipan up`: {e}");
            return None;
        }
    };
    if !output.status.success() {
        eprintln!(
            "killer_demo_test: SKIPPING: `taipan up --name {name}` failed ({}):\n\
             --- stdout ---\n{}\n--- stderr ---\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        return None;
    }
    println!(
        "killer_demo_test: `taipan up --name {name}` succeeded:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );

    if !descriptor_path.is_file() {
        eprintln!(
            "killer_demo_test: SKIPPING: taipan up exited 0 but {} was never written",
            descriptor_path.display()
        );
        return None;
    }

    Some(KillerDemoEnv {
        taipan_bin,
        name,
        descriptor_path,
        torn_down: false,
    })
}

fn unique_temp_path(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "genaryx-killer-demo-{tag}-{}-{n}",
        std::process::id()
    ))
}

// ---- the killer demo --------------------------------------------------------

#[tokio::test]
async fn killer_demo_minted_autodiscover_pair_kill_and_console_command_e2e() {
    let Some(env) = try_bring_up() else {
        return; // Already explained why via eprintln! above.
    };

    // ---- step 2: auto-discovery, the same descriptor + keyfile shape the
    // shells' env::discover() reads ------------------------------------------
    let discovered = discover_taipan_env(&env.descriptor_path).unwrap_or_else(|| {
        panic!(
            "taipan up must write a descriptor + keyfile auto-discovery can resolve: {}",
            env.descriptor_path.display()
        )
    });
    // Regression for the bug this test guards (issue #20): the keyfile secret
    // (the bearer a client sends) must be the BARE minted token, not the full
    // `token:org:role` spec. The server indexes its key map by the bare token,
    // so a suffixed bearer never matches and 401s for both reads and pairing.
    // taipan now writes the bare token; assert exactly that here, so a
    // regression to the full-spec secret fails loudly at this line.
    assert!(
        discovered.admin_bearer.starts_with("tp_"),
        "auto-discovered bearer must be a minted tp_ token, got {:?}",
        discovered.admin_bearer
    );
    assert!(
        !discovered.admin_bearer.contains(':'),
        "the keyfile secret must be the BARE token (no :org:role suffix), got {:?} - a suffixed \
         bearer never matches the server's bare-token key index (issue #20)",
        discovered.admin_bearer
    );
    println!(
        "killer_demo_test: auto-discovered cloud_url={} admin_bearer={}",
        discovered.cloud_url, discovered.admin_bearer
    );

    // ---- step 3: pair, read, signed kill - the path that used to 401 -------
    let mut client = CloudClient::new(&discovered.cloud_url, &discovered.admin_bearer)
        .expect("build CloudClient");
    let signer = SoftwareSigner::generate().expect("generate a software P-256 key");
    let paired = client.pair(&discovered.admin_bearer, &signer).await.expect(
        "pairing through taipan-up auto-discovery must now succeed - this is the exact 401 the \
             bare-token keyfile fix closes (issue #20)",
    );
    println!(
        "killer_demo_test: PAIR OK device_id={} org={} role={}",
        paired.device_id, paired.org, paired.role
    );
    assert_eq!(
        paired.role, "admin",
        "the minted cloud_admin key pairs with admin role"
    );
    assert!(
        paired.org.starts_with("taipan-"),
        "minted org is taipan-<env name>, got {:?}",
        paired.org
    );
    assert!(!paired.device_id.is_empty());
    assert!(!paired.device_token.is_empty());

    client.attach_device(
        paired.device_id.clone(),
        paired.device_token.clone(),
        Box::new(signer),
    );
    assert!(client.has_device());

    let summary = client
        .summary()
        .await
        .expect("GET /v1/summary through the paired device's org");
    println!(
        "killer_demo_test: summary runs={} calls={} spent_microusd={}",
        summary.runs, summary.calls, summary.spent_microusd
    );

    let run_id = format!("killerdemo-run-{}", std::process::id());
    let killed = client
        .kill_run(&run_id)
        .await
        .expect("signed kill_run must be accepted (200)");
    assert_eq!(killed.killed, run_id);
    println!(
        "killer_demo_test: KILL OK run_id={run_id} response.killed={}",
        killed.killed
    );

    // ---- step 4: journal + emit the console_command bus event --------------
    let store_path = unique_temp_path("store").with_extension("sqlite3");
    let console_events_path = unique_temp_path("console").with_extension("ndjson");
    let store = Store::open(&store_path).expect("open sqlite store");

    let rec = CommandRecord {
        operator: "user://killer-demo.test/operator".to_string(),
        env: env.name.clone(),
        action: "console.kill_run".to_string(),
        target: run_id.clone(),
        params: serde_json::json!({}),
        decision: "break_glass".to_string(),
        sig_alg: "es256".to_string(),
        sig_fpr: "software-signed".to_string(),
        http_status: 200,
        verify_result: format!("killed:{}", killed.killed == run_id),
    };
    genaryx_core::command::record(
        &store,
        &console_events_path,
        "killer-demo.test",
        "killer-demo-host",
        &rec,
    )
    .expect("journal + emit the console_command outcome");

    let body = std::fs::read_to_string(&console_events_path).expect("read console events file");
    let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        lines.len(),
        1,
        "exactly one console_command line must be appended"
    );
    let line = lines[0];
    println!("killer_demo_test: console_command line => {line}");

    let conformer = Conformer::new().expect("embedded schemas must compile");
    let report = conformer.check_line(line);
    assert!(
        report.valid,
        "console_command line must conform: {:?}\n  line: {line}",
        report.errors
    );
    assert_eq!(report.schema_version, Some(SchemaVersion::V0_2));

    let value: serde_json::Value = serde_json::from_str(line).expect("parse emitted line");
    assert_eq!(
        value.get("source").and_then(|v| v.as_str()),
        Some("console")
    );
    assert_eq!(
        value.get("type").and_then(|v| v.as_str()),
        Some("console_command")
    );
    let data = value.get("data").expect("data present");
    assert_eq!(
        data.get("action").and_then(|v| v.as_str()),
        Some("console.kill_run")
    );
    assert_eq!(
        data.get("target").and_then(|v| v.as_str()),
        Some(run_id.as_str())
    );
    assert_eq!(data.get("http_status").and_then(|v| v.as_u64()), Some(200));
    assert_eq!(
        data.get("verify_result").and_then(|v| v.as_str()),
        Some("killed:true")
    );

    assert_eq!(
        store.commands_journal_count().expect("count"),
        1,
        "the commands_journal row must be written alongside the bus line"
    );

    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&console_events_path);

    // ---- step 5: taipan down, assert clean teardown, no orphans ------------
    // Capture what taipan itself tracked for this environment BEFORE tearing
    // down, so "no orphans" is checked against the exact processes taipan
    // started, not a guess.
    let pidfile_path = env.pidfile_path();
    let keyfile_path = env.keyfile_path();
    let descriptor_path = env.descriptor_path.clone();
    let pids = read_pidfile_pids(&pidfile_path);
    assert!(
        !pids.is_empty(),
        "pidfile must list the processes taipan up started"
    );

    let down_output = env.tear_down();
    assert!(
        down_output.status.success(),
        "taipan down must exit 0 (clean teardown): {}\n{}",
        down_output.status,
        String::from_utf8_lossy(&down_output.stderr)
    );
    let down_stdout = String::from_utf8_lossy(&down_output.stdout);
    println!("killer_demo_test: `taipan down` =>\n{down_stdout}");
    assert!(
        !down_stdout.contains("STILL ALIVE"),
        "taipan down must not report a surviving process: {down_stdout}"
    );

    assert!(
        !pidfile_path.exists(),
        "pidfile must be removed after a clean taipan down"
    );
    assert!(
        !descriptor_path.exists(),
        "descriptor must be removed after a clean taipan down"
    );
    assert!(
        !keyfile_path.exists(),
        "keyfile must be removed after a clean taipan down"
    );

    for pid in pids {
        assert!(
            !pid_alive(pid),
            "pid {pid} must no longer be alive after taipan down (orphaned process)"
        );
    }
    assert!(
        port_is_closed(CLOUD_PORT),
        "cloud port {CLOUD_PORT} must be released after taipan down (orphan?)"
    );
    assert!(
        port_is_closed(GATEWAY_PORT),
        "gateway port {GATEWAY_PORT} must be released after taipan down (orphan?)"
    );
}
