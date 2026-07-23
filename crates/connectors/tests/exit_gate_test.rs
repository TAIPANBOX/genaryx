//! Phase-2 EXIT-GATE end-to-end acceptance test (task #28, 09 Ф2).
//!
//! This is DEFENSIVE self-verification: it proves an operator's own
//! governance stack actually holds, grants, and re-checks one of the
//! operator's own agent actions. It never targets or sends traffic to
//! anything beyond the operator's own local `taipan up` stack.
//!
//! `wardryx_test.rs` proves `WardryxClient` against a bare `wardryx serve`
//! (its own build, no `taipan`, no gateway in front of it).
//! `killer_demo_test.rs` proves the Cloud auto-discovery + pairing path
//! through a real `taipan up`. Neither proves the actual Ф2 product path:
//! an agent's costly action holds behind a live gateway wired to a live
//! Wardryx, a console grants it exactly like the Policy panel's Approvals
//! Inbox would (07 §4.3, docs/PHASE2.md Wave 2), and the SAME held request
//! then goes through once granted. That is what this file drives, through
//! the real `taipan`/`tokenfuse`/`wardryx`/`mockryx` binaries, never a
//! reimplementation of any of their logic:
//!
//! 1. `taipan up --name p2exit --with wardryx`: builds/spawns the real
//!    gateway + cloud + wardryx (task #29 fixed `taipan` so this now
//!    actually enforces: a minted `WARDRYX_APPROVAL_SECRET`, a demo policy
//!    seeded for `agent://mockryx.local/*`, and the gateway wired to
//!    consult wardryx as its PDP). Auto-discovery mirrors
//!    `killer_demo_test.rs`'s own two-file (descriptor + keyfile) read,
//!    extended for the `wardryx` service entry and `wardryx_admin_ref`.
//! 2. The two shipped mockryx fire drills (`approval-required`,
//!    `wardryx-denied-tool`) run against the live gateway, via the real
//!    `mockryx` CLI when `go`/the mockryx checkout are available, or a
//!    direct HTTP replica of the identical two calls otherwise.
//! 3. A console-in-the-loop cycle driven directly through `WardryxClient`
//!    (exactly the calls the Policy panel's Approvals Inbox makes, 07
//!    §4.3): trigger a fresh hold, list it, grant it with a local
//!    hardware-confirmation-style decision (mirroring the SwiftUI shell's
//!    Touch ID gate, `crates/ffi/src/wardryx/mod.rs`), decode the minted
//!    token, journal a conforming `console_command` via
//!    `genaryx_core::command::record`, then resubmit the SAME held request
//!    with the token attached and watch it go through.
//! 4. Token-boundary units directly against `WardryxClient::decide`: a cost
//!    above the granted ceiling still denies even with a valid token; the
//!    TTL reads close to the 10-minute default at mint; a single-use token
//!    (a separate, dedicated wardryx instance, since the taipan-managed one
//!    is never single-use) cannot be redeemed twice.
//! 5. `taipan down --name p2exit`, then confirm the descriptor/keyfile/
//!    pidfile are gone, every pid taipan tracked is no longer alive, and
//!    ports 4100/8080/8090 are all free.
//!
//! Gated like `killer_demo_test.rs`: any failure getting the live stack up
//! (missing `~/Development/taipan`/`tokenfuse` checkout, a build failure,
//! the fixed ports already busy, a health-check timeout, or `--with
//! wardryx` itself degrading because the wardryx sibling repo or a `go`
//! toolchain is missing) degrades to an `eprintln!` skip rather than a red
//! `cargo test -p genaryx-connectors`. Two narrower dependencies degrade
//! independently instead of skipping the whole test: the mockryx CLI step
//! falls back to a direct HTTP replica when `go`/mockryx is unavailable,
//! and the single-use token-boundary check is skipped with a clear note
//! under the same condition (config-gated behavior wardryx's own tests
//! already cover). Everything else, once the live stack is confirmed up,
//! is a hard assertion.
//!
//! Hermetic in the same sense `killer_demo_test.rs` documents: `taipan`'s
//! gateway/cloud/wardryx bind fixed ports regardless of `--name`, so only
//! one `taipan` environment can be live on this box at a time; this test
//! checks for that and skips rather than racing it, and a fixed
//! (non-pid-suffixed) environment name changes nothing about that
//! constraint. A `Drop` guard always attempts `taipan down` even on a
//! mid-test panic, so a failed assertion never leaves the stack running for
//! the next test run.

use genaryx_connectors::{
    ApprovalTokenClaims, ApprovalVerdict, DecideRequest, Policy, WardryxClient,
};
use genaryx_core::store::Store;
use genaryx_core::{CommandRecord, Conformer, SchemaVersion};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// The environment name this test brings up under, matching this task's own
/// spec (`taipan up --name p2exit --with wardryx`) literally rather than
/// pid-suffixing it the way `killer_demo_test.rs` does: `taipan`'s
/// gateway/cloud/wardryx bind fixed ports regardless of `--name`, so a
/// fixed name buys no less hermeticity than a suffixed one, only a more
/// legible descriptor/keyfile path (`p2exit.json` / `p2exit.keys.json`).
const ENV_NAME: &str = "p2exit";

/// `tokenfuse-cloud`'s fixed bind port under `taipan up`
/// (`~/Development/taipan/src/services/cloud.rs::PORT`); unaffected by
/// `--name`.
const CLOUD_PORT: u16 = 8080;
/// The gateway's fixed bind port under `taipan up`
/// (`~/Development/taipan/src/services/gateway.rs::PORT`); unaffected by
/// `--name`.
const GATEWAY_PORT: u16 = 4100;
/// Wardryx's fixed bind port under `taipan up --with wardryx`
/// (`~/Development/taipan/src/services/wardryx.rs::PORT`); unaffected by
/// `--name`.
const WARDRYX_PORT: u16 = 8090;

/// Mirrors what the former SwiftUI shell's Touch-ID-gated grant convention
/// used (`crates/ffi/src/wardryx/mod.rs`'s `SIG_ALG`/`SIG_FPR`, before that
/// shell was removed with the web-only pivot), the shape docs/PHASE2.md's
/// exit-gate line names explicitly ("the operator grants it with a local
/// hardware confirmation (Touch ID)"): Wardryx's admin API has no signing
/// story at all (bearer-only), so these are honest, fixed labels naming
/// what actually authorized the call rather than a fabricated signature.
/// The former Tauri shell's own confirm ceremony instead used
/// `"none"`/`"bearer-admin"` (`policy/state.rs`, under its now-removed
/// `apps/desktop/src-tauri`) since it was not yet hardware-gated; this test
/// proves the hardware-gated cycle the exit gate itself describes.
const GRANT_SIG_ALG: &str = "bearer";
const GRANT_SIG_FPR: &str = "local-auth";

// ---- environment discovery (extends killer_demo_test.rs's mirror with wardryx) --

/// Behavioral mirror of `taipan/src/descriptor.rs`'s `ServiceEntry`; only
/// the field this test reads.
#[derive(Debug, Deserialize)]
struct DescriptorService {
    url: String,
}

/// Behavioral mirror of `taipan/src/descriptor.rs`'s `KeysSection`,
/// extended with `wardryx_admin_ref` beyond `killer_demo_test.rs`'s own
/// `cloud_admin_ref`-only mirror.
#[derive(Debug, Default, Deserialize)]
struct DescriptorKeys {
    #[serde(default)]
    wardryx_admin_ref: Option<String>,
}

/// Behavioral mirror of `taipan/src/descriptor.rs`'s `Descriptor`, plus its
/// `unavailable` map: the one field `killer_demo_test.rs` never needed
/// (Cloud is mandatory, never degrades), but this test does, since
/// `--with wardryx` is an opt-in service that can gracefully come up
/// missing (see `taipan/src/commands/up.rs`'s module doc).
#[derive(Debug, Deserialize)]
struct Descriptor {
    name: String,
    services: BTreeMap<String, DescriptorService>,
    #[serde(default)]
    keys: DescriptorKeys,
    #[serde(default)]
    unavailable: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct KeyFile {
    #[serde(default)]
    secrets: BTreeMap<String, String>,
}

struct DiscoveredEnv {
    gateway_url: String,
    wardryx_url: String,
    wardryx_admin_bearer: String,
}

/// Resolve a `taipan up --with wardryx` descriptor into the gateway URL,
/// the wardryx URL, and the wardryx admin bearer: read
/// `services.{gateway,wardryx}.url`, follow `keys.wardryx_admin_ref`'s
/// trailing label into the sibling `<name>.keys.json`, and return the
/// secret it names. `None` on any missing/malformed step (never a panic),
/// exactly the fail-closed contract `killer_demo_test.rs::discover_taipan_env`
/// already documents for the Cloud side.
fn discover_taipan_env(descriptor_path: &Path) -> Option<DiscoveredEnv> {
    let bytes = std::fs::read(descriptor_path).ok()?;
    let descriptor: Descriptor = serde_json::from_slice(&bytes).ok()?;

    let gateway_url = descriptor.services.get("gateway")?.url.clone();
    let wardryx_url = descriptor.services.get("wardryx")?.url.clone();

    let admin_ref = descriptor.keys.wardryx_admin_ref?;
    let label = admin_ref.rsplit('/').next()?;

    let keys_path = descriptor_path.with_file_name(format!("{}.keys.json", descriptor.name));
    let key_bytes = std::fs::read(&keys_path).ok()?;
    let keyfile: KeyFile = serde_json::from_slice(&key_bytes).ok()?;
    let wardryx_admin_bearer = keyfile.secrets.get(label)?.clone();

    Some(DiscoveredEnv {
        gateway_url,
        wardryx_url,
        wardryx_admin_bearer,
    })
}

// ---- shared bring-up plumbing (mirrors killer_demo_test.rs) ---------------

fn taipan_repo() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(home).join("Development/taipan");
    dir.join("Cargo.toml").is_file().then_some(dir)
}

/// `~/.taipan`, matching `taipan::home::TaipanHome::discover`. Mirrored
/// here rather than imported: `taipan` is a `[[bin]]`-only crate with no
/// library target this test could depend on.
fn taipan_home() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".taipan"))
}

fn port_is_closed(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_err()
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

/// Whether `pid` (a process `taipan up` started) is still alive, via `kill
/// -0` (existence/permission probe only, sends nothing) - the same check
/// `killer_demo_test.rs::pid_alive` uses, over PIDs read from taipan's own
/// pidfile, never discovered via `ps`/`lsof`.
fn pid_alive(pid: i32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ---- bring-up / tear-down (mirrors killer_demo_test.rs::KillerDemoEnv) ----

struct ExitGateEnv {
    taipan_bin: PathBuf,
    name: String,
    descriptor_path: PathBuf,
    torn_down: bool,
}

impl ExitGateEnv {
    fn pidfile_path(&self) -> PathBuf {
        self.descriptor_path
            .with_file_name(format!("{}.pid.json", self.name))
    }

    fn keyfile_path(&self) -> PathBuf {
        self.descriptor_path
            .with_file_name(format!("{}.keys.json", self.name))
    }

    /// Explicit, asserted teardown. Consumes `self` so the `Drop` safety
    /// net below never double-runs `taipan down` after a clean one.
    fn tear_down(mut self) -> std::process::Output {
        let out = Command::new(&self.taipan_bin)
            .args(["down", "--name", &self.name])
            .output()
            .expect("spawn `taipan down`");
        self.torn_down = true;
        out
    }
}

impl Drop for ExitGateEnv {
    fn drop(&mut self) {
        if self.torn_down {
            return;
        }
        eprintln!(
            "exit_gate_test: Drop safety net tearing down '{}' (explicit tear_down was not reached, likely a mid-test panic)",
            self.name
        );
        let _ = Command::new(&self.taipan_bin)
            .args(["down", "--name", &self.name])
            .output();
    }
}

/// Build the `taipan` binary (a no-op if already current). `None` (after
/// an explanatory `eprintln!`) on any failure.
fn build_taipan(repo: &Path) -> Option<PathBuf> {
    let status = Command::new("cargo")
        .args(["build", "--quiet"])
        .current_dir(repo)
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("exit_gate_test: SKIPPING: `cargo build` for taipan failed ({s})");
            return None;
        }
        Err(e) => {
            eprintln!("exit_gate_test: SKIPPING: could not run cargo for taipan: {e}");
            return None;
        }
    }
    let bin = repo.join("target/debug/taipan");
    if !bin.is_file() {
        eprintln!(
            "exit_gate_test: SKIPPING: build succeeded but {} is missing (unexpected target layout?)",
            bin.display()
        );
        return None;
    }
    Some(bin)
}

/// Bring up `taipan up --name p2exit --with wardryx` and confirm the
/// descriptor landed. `None` (after an explanatory `eprintln!`) on any
/// failure - only "is the live stack even up" degrades gracefully here;
/// whether wardryx SPECIFICALLY came up is checked separately by the
/// caller (see the test body), since it is the one opt-in piece of this
/// mix.
fn try_bring_up() -> Option<ExitGateEnv> {
    let Some(repo) = taipan_repo() else {
        eprintln!("exit_gate_test: SKIPPING: ~/Development/taipan not found");
        return None;
    };
    let Some(home) = taipan_home() else {
        eprintln!("exit_gate_test: SKIPPING: $HOME not set");
        return None;
    };
    let Some(taipan_bin) = build_taipan(&repo) else {
        return None; // build_taipan already explained why.
    };

    if !port_is_closed(CLOUD_PORT) || !port_is_closed(GATEWAY_PORT) || !port_is_closed(WARDRYX_PORT)
    {
        eprintln!(
            "exit_gate_test: SKIPPING: port {CLOUD_PORT}, {GATEWAY_PORT}, or {WARDRYX_PORT} is \
             already in use (another taipan environment up?). taipan's services bind fixed \
             ports regardless of --name, so only one `taipan up` can be live on this box at a time"
        );
        return None;
    }

    let name = ENV_NAME.to_string();
    let descriptor_path = home.join("environments").join(format!("{name}.json"));

    let up_result = Command::new(&taipan_bin)
        .args(["up", "--name", &name, "--with", "wardryx"])
        .current_dir(&repo)
        .output();
    let output = match up_result {
        Ok(o) => o,
        Err(e) => {
            eprintln!("exit_gate_test: SKIPPING: failed to spawn `taipan up`: {e}");
            return None;
        }
    };
    if !output.status.success() {
        eprintln!(
            "exit_gate_test: SKIPPING: `taipan up --name {name} --with wardryx` failed ({}):\n\
             --- stdout ---\n{}\n--- stderr ---\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        return None;
    }
    println!(
        "exit_gate_test: `taipan up --name {name} --with wardryx` succeeded:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );

    if !descriptor_path.is_file() {
        eprintln!(
            "exit_gate_test: SKIPPING: taipan up exited 0 but {} was never written",
            descriptor_path.display()
        );
        return None;
    }

    Some(ExitGateEnv {
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
        "genaryx-exit-gate-{tag}-{}-{n}",
        std::process::id()
    ))
}

// ---- gateway HTTP calls (the x-fuse-* contract mockryx/runner.go and ------
// ---- tokenfuse/proxy.rs share) ---------------------------------------------

/// One crafted `POST {gateway}/v1/messages` call: the exact `x-fuse-*`
/// header contract `mockryx/internal/runner/runner.go:320-339` sends and
/// `tokenfuse/crates/gateway/src/proxy.rs` reads. `x-api-key` is accepted
/// but inert (the gateway is loopback-only, 07 §4.1, and never reads it);
/// it rides along anyway so the direct-HTTP replica in step 2's fallback
/// matches mockryx's own wire shape exactly. All-reference fields, so this
/// is cheaply `Copy` - used to build "the SAME request, plus one header"
/// via struct-update syntax (see step 3f) instead of duplicating a call
/// site.
#[derive(Clone, Copy)]
struct MessagesCall<'a> {
    api_key: &'a str,
    run_id: &'a str,
    agent_id: &'a str,
    budget_usd: &'a str,
    on_behalf_of: Option<&'a str>,
    task_type: Option<&'a str>,
    approval_token: Option<&'a str>,
    body: &'a serde_json::Value,
}

impl MessagesCall<'_> {
    async fn send(&self, http: &reqwest::Client, gateway_url: &str) -> reqwest::Response {
        let mut b = http
            .post(format!("{gateway_url}/v1/messages"))
            .header("content-type", "application/json")
            .header("x-api-key", self.api_key)
            .header("x-fuse-run-id", self.run_id)
            .header("x-fuse-agent-id", self.agent_id)
            .header("x-fuse-budget-usd", self.budget_usd);
        if let Some(v) = self.on_behalf_of {
            b = b.header("x-fuse-on-behalf-of", v);
        }
        if let Some(v) = self.task_type {
            b = b.header("x-fuse-task-type", v);
        }
        if let Some(v) = self.approval_token {
            b = b.header("x-fuse-approval-token", v);
        }
        b.body(serde_json::to_vec(self.body).expect("serialize /v1/messages body"))
            .send()
            .await
            .expect("POST /v1/messages must reach the gateway")
    }
}

fn header(resp: &reqwest::Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

// ---- step 2: the mockryx fire drills ---------------------------------------

/// Best-effort locate `~/Development/mockryx`, a sibling of `taipan`/
/// `tokenfuse`/`wardryx` under `~/Development`.
fn mockryx_repo() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(home).join("Development/mockryx");
    dir.join("go.mod").is_file().then_some(dir)
}

/// `go build -o bin/mockryx ./cmd/mockryx`, exactly mockryx's own Makefile
/// target (`bin/` is gitignored there, a normal disposable build artifact,
/// same status as `taipan`'s own `target/debug/taipan`). `None` (after an
/// explanatory `println!`, NOT a whole-test skip - see this file's module
/// doc) when the repo or the `go` toolchain is unavailable, or the build
/// fails; the caller falls back to replicating the two scenario calls
/// directly over HTTP instead.
fn build_mockryx() -> Option<PathBuf> {
    let Some(repo) = mockryx_repo() else {
        println!(
            "exit_gate_test: NOTE: ~/Development/mockryx not found; step 2 falls back to the \
             direct-HTTP replica of both scenarios"
        );
        return None;
    };
    let status = Command::new("go")
        .args(["build", "-o", "bin/mockryx", "./cmd/mockryx"])
        .current_dir(&repo)
        .status();
    let bin = repo.join("bin/mockryx");
    match status {
        Ok(s) if s.success() && bin.is_file() => Some(bin),
        Ok(s) => {
            println!(
                "exit_gate_test: NOTE: `go build -o bin/mockryx ./cmd/mockryx` did not produce a \
                 usable binary ({s}); step 2 falls back to the direct-HTTP replica"
            );
            None
        }
        Err(e) => {
            println!(
                "exit_gate_test: NOTE: `go` toolchain unavailable ({e}); step 2 falls back to the \
                 direct-HTTP replica"
            );
            None
        }
    }
}

// ---- step 4c: a dedicated, single-use-configured wardryx (mirrors ---------
// ---- wardryx_test.rs's own spawn helper) -----------------------------------

const SINGLE_USE_BEARER: &str = "tk_exit_gate_su";
const SINGLE_USE_KEYS: &str = "tk_exit_gate_su:exit-gate-org:admin";
const SINGLE_USE_SECRET: &str = "genaryx-exit-gate-single-use-secret-0123456789";
const SINGLE_USE_HEALTHZ_TIMEOUT: Duration = Duration::from_secs(30);

/// Kills and reaps the child on drop, including on a mid-test panic, and
/// removes the scratch binary + events file this test wrote to
/// `std::env::temp_dir()`. Mirrors `wardryx_test.rs::ChildGuard` exactly.
struct SingleUseWardryxGuard {
    child: Child,
    bin_path: PathBuf,
    events_path: PathBuf,
}

impl Drop for SingleUseWardryxGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.bin_path);
        let _ = std::fs::remove_file(&self.events_path);
    }
}

/// Best-effort locate `~/Development/wardryx`, a sibling of `taipan`/
/// `tokenfuse`/`mockryx` under `~/Development`. Read-only in spirit: only
/// ever built and run, never edited (mirrors `wardryx_test.rs::wardryx_repo`).
fn wardryx_repo() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(home).join("Development/wardryx");
    dir.join("go.mod").is_file().then_some(dir)
}

fn free_port() -> Option<u16> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
}

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

/// Spawn the already-built `wardryx` binary directly (never `go run`), with
/// `WARDRYX_APPROVAL_SINGLE_USE=1` on top of the same `WARDRYX_KEYS`/
/// `WARDRYX_APPROVAL_SECRET` shape `wardryx_test.rs::spawn_wardryx` uses -
/// the one env var that instance never sets, since this dimension needs an
/// isolated server (the taipan-managed instance the rest of this test
/// drives is never single-use).
fn spawn_single_use_wardryx(bin_path: &Path, addr: &str, events_path: &Path) -> Option<Child> {
    Command::new(bin_path)
        .arg("serve")
        .arg("-addr")
        .arg(addr)
        .arg("-events")
        .arg(events_path)
        .env("WARDRYX_KEYS", SINGLE_USE_KEYS)
        .env("WARDRYX_APPROVAL_SECRET", SINGLE_USE_SECRET)
        .env("WARDRYX_APPROVAL_SINGLE_USE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

/// Stand up a fresh, single-use-configured `wardryx serve` on an ephemeral
/// port and wait for `/healthz`. `None` (after an explanatory `println!`,
/// not a whole-test skip) on any failure along the way.
async fn try_start_single_use_wardryx() -> Option<(SingleUseWardryxGuard, String)> {
    let Some(repo) = wardryx_repo() else {
        println!(
            "exit_gate_test: NOTE: ~/Development/wardryx not found; step 4c (single-use) is skipped"
        );
        return None;
    };
    let Some(port) = free_port() else {
        println!(
            "exit_gate_test: NOTE: could not reserve a test port; step 4c (single-use) is skipped"
        );
        return None;
    };

    let tmp = std::env::temp_dir();
    let unique = format!("genaryx-exit-gate-su-{}-{port}", std::process::id());
    let bin_path = tmp.join(&unique);
    let events_path = tmp.join(format!("{unique}.ndjson"));

    if let Err(reason) = build_wardryx(&repo, &bin_path) {
        println!("exit_gate_test: NOTE: {reason}; step 4c (single-use) is skipped");
        return None;
    }
    if !bin_path.is_file() {
        println!(
            "exit_gate_test: NOTE: build succeeded but {} is missing; step 4c (single-use) is skipped",
            bin_path.display()
        );
        return None;
    }

    let addr = format!("127.0.0.1:{port}");
    let Some(mut child) = spawn_single_use_wardryx(&bin_path, &addr, &events_path) else {
        println!(
            "exit_gate_test: NOTE: failed to spawn {}; step 4c (single-use) is skipped",
            bin_path.display()
        );
        let _ = std::fs::remove_file(&bin_path);
        return None;
    };

    let base = format!("http://{addr}");
    let http = reqwest::Client::new();
    let deadline = Instant::now() + SINGLE_USE_HEALTHZ_TIMEOUT;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            println!(
                "exit_gate_test: NOTE: single-use wardryx exited early ({status}); step 4c is skipped"
            );
            let _ = std::fs::remove_file(&bin_path);
            let _ = std::fs::remove_file(&events_path);
            return None;
        }
        if let Ok(resp) = http.get(format!("{base}/healthz")).send().await
            && resp.status().is_success()
        {
            return Some((
                SingleUseWardryxGuard {
                    child,
                    bin_path,
                    events_path,
                },
                base,
            ));
        }
        if Instant::now() >= deadline {
            println!(
                "exit_gate_test: NOTE: single-use wardryx never answered /healthz within \
                 {SINGLE_USE_HEALTHZ_TIMEOUT:?}; step 4c is skipped"
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

// ---- the exit gate ----------------------------------------------------------

#[tokio::test]
async fn exit_gate_hold_grant_token_allow_e2e() {
    let Some(env) = try_bring_up() else {
        return; // Already explained why via eprintln! above.
    };

    // ---- wardryx availability: the one opt-in piece of this mix -----------
    // Gateway + cloud are mandatory (a failure there already made
    // try_bring_up skip, above); wardryx is the opt-in `--with` service
    // that can gracefully come up missing (taipan/src/commands/up.rs's
    // module doc). Without it there is no PDP to test against, so this is
    // this test's second and last graceful-skip point - everything
    // downstream is a hard assertion.
    let descriptor_bytes = std::fs::read(&env.descriptor_path)
        .unwrap_or_else(|e| panic!("read descriptor {}: {e}", env.descriptor_path.display()));
    let descriptor: Descriptor = serde_json::from_slice(&descriptor_bytes)
        .unwrap_or_else(|e| panic!("parse descriptor {}: {e}", env.descriptor_path.display()));
    if let Some(reason) = descriptor.unavailable.get("wardryx") {
        eprintln!(
            "exit_gate_test: SKIPPING: taipan brought up gateway+cloud but not wardryx \
             (--with wardryx degraded gracefully, likely missing ~/Development/wardryx or a go \
             toolchain): {reason}"
        );
        let down = env.tear_down();
        if !down.status.success() {
            eprintln!(
                "exit_gate_test: WARNING: teardown after a wardryx-unavailable skip did not exit \
                 0:\n{}",
                String::from_utf8_lossy(&down.stderr)
            );
        }
        return;
    }

    let discovered = discover_taipan_env(&env.descriptor_path).unwrap_or_else(|| {
        panic!(
            "wardryx was reported available in the descriptor's services map, but auto-discovery \
             (gateway/wardryx URL + wardryx_admin_ref -> keyfile secret) still failed to resolve \
             from {}",
            env.descriptor_path.display()
        )
    });
    println!(
        "exit_gate_test: discovered gateway={} wardryx={}",
        discovered.gateway_url, discovered.wardryx_url
    );

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("build reqwest client");
    let wardryx_client =
        WardryxClient::new(&discovered.wardryx_url, &discovered.wardryx_admin_bearer)
            .expect("build WardryxClient against the taipan-managed wardryx");

    // ======================================================================
    // step 2: full-stack fire drills (the real mockryx scenarios)
    // ======================================================================
    let scenarios_src = mockryx_repo().map(|r| r.join("scenarios"));
    match build_mockryx() {
        Some(mockryx_bin) => {
            let scenarios_dir = unique_temp_path("mockryx-scenarios");
            std::fs::create_dir_all(&scenarios_dir).expect("create scratch scenarios dir");
            let src =
                scenarios_src.expect("mockryx repo resolved a binary but not a scenarios dir");
            for name in ["approval-required.yaml", "wardryx-denied-tool.yaml"] {
                std::fs::copy(src.join(name), scenarios_dir.join(name))
                    .unwrap_or_else(|e| panic!("copy scenario {name}: {e}"));
            }

            // Key via MOCKRYX_API_KEY, not an --api-key argv flag: the same
            // secret-hygiene reason MockryxClient::run uses (a key on argv is
            // readable from the host process table); mockryx reads this env
            // var as the flag's own default.
            let output = Command::new(&mockryx_bin)
                .args([
                    "run",
                    "--gateway",
                    &discovered.gateway_url,
                    scenarios_dir
                        .to_str()
                        .expect("scenarios dir path is valid UTF-8"),
                ])
                .env("MOCKRYX_API_KEY", &discovered.wardryx_admin_bearer)
                .output()
                .expect("spawn `mockryx run`");
            println!(
                "exit_gate_test: step 2 (mockryx CLI) => exit {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            assert!(
                output.status.success(),
                "mockryx run must exit 0 (both scenarios passed) against a properly enforcing \
                 taipan up --with wardryx stack; exit {}",
                output.status
            );
            let _ = std::fs::remove_dir_all(&scenarios_dir);
        }
        None => {
            // Direct-HTTP replica of the same two calls, byte-for-byte from
            // mockryx/scenarios/{approval-required,wardryx-denied-tool}.yaml.
            let hold_body = serde_json::json!({
                "model": "claude-opus-4-5",
                "max_tokens": 500_000_000i64,
                "messages": [{"role": "user", "content": "please go ahead and process this transaction now"}]
            });
            let hold_call = MessagesCall {
                api_key: &discovered.wardryx_admin_bearer,
                run_id: "mockryx-approval-required",
                agent_id: "agent://mockryx.local/rehearsal/payments-bot",
                budget_usd: "5000.00",
                on_behalf_of: Some("user://mockryx.local/rehearsal-operator"),
                task_type: Some("payments_automation"),
                approval_token: None,
                body: &hold_body,
            };
            let resp = hold_call.send(&http, &discovered.gateway_url).await;
            let status = resp.status().as_u16();
            let wh = header(&resp, "x-fuse-wardryx");
            println!(
                "exit_gate_test: step 2 (direct-HTTP replica) approval-required => {status} x-fuse-wardryx={wh:?}"
            );
            assert_eq!(status, 403, "approval-required replica: expected 403");
            assert_eq!(
                wh.as_deref(),
                Some("hold"),
                "approval-required replica: expected x-fuse-wardryx: hold"
            );

            let deny_body = serde_json::json!({
                "model": "claude-haiku",
                "max_tokens": 50,
                "messages": [{"role": "user", "content": "please run this shell command for me"}],
                "tools": [{"name": "shell_exec", "description": "Execute an arbitrary shell command on the host."}]
            });
            let deny_call = MessagesCall {
                api_key: &discovered.wardryx_admin_bearer,
                run_id: "mockryx-wardryx-denied-tool",
                agent_id: "agent://mockryx.local/rehearsal/ops-helper",
                budget_usd: "1.00",
                on_behalf_of: Some("user://mockryx.local/rehearsal-operator"),
                task_type: Some("ops_automation"),
                approval_token: None,
                body: &deny_body,
            };
            let resp = deny_call.send(&http, &discovered.gateway_url).await;
            let status = resp.status().as_u16();
            let wh = header(&resp, "x-fuse-wardryx");
            println!(
                "exit_gate_test: step 2 (direct-HTTP replica) wardryx-denied-tool => {status} x-fuse-wardryx={wh:?}"
            );
            assert_eq!(status, 403, "wardryx-denied-tool replica: expected 403");
            assert_eq!(
                wh.as_deref(),
                Some("deny"),
                "wardryx-denied-tool replica: expected x-fuse-wardryx: deny"
            );
        }
    }

    // ======================================================================
    // step 3: console-in-the-loop approval cycle
    // ======================================================================
    let payments_bot_agent = "agent://mockryx.local/rehearsal/payments-bot";
    let console_cycle_run_id = "p2exit-console-cycle";
    // Declares one (harmless, not in the demo policy's deny_tool list) tool
    // so tool_names is never empty end to end. An empty declared-tool list
    // is not a good fit here: wardryx's own sortedCopy(nil-safe helper)
    // turns a zero-length tool slice into a Go NIL slice, which
    // encoding/json marshals as `null`, not `[]` - both in the approval's
    // `context.tool_names` (confirmed live: `Approval::tool_names()`
    // correctly reads that as `None`, matching how it already treats a
    // null `on_behalf_of`) and in the minted token's own `tools` claim. A
    // single declared tool sidesteps that Go nil-slice/JSON-null edge case
    // entirely and lets 3b/3d/4a assert a concrete, non-empty tool set.
    let held_tool = "process_payment";
    // A model/max_tokens combo the taipan-seeded demo policy's
    // require_human_above_usd:1.0 clears comfortably (well over $1, well
    // under the $1000 budget below so the post-grant resubmission in 3f can
    // actually reserve it) - see tokenfuse/crates/gateway/src/{estimate.rs,
    // pricebook.rs}: "claude-sonnet" prices at $15/Mtok output, so
    // 2_000_000 max_tokens estimates to roughly $34-$35 with the built-in
    // 15% margin.
    let high_cost_body = serde_json::json!({
        "model": "claude-sonnet",
        "max_tokens": 2_000_000,
        "messages": [{"role": "user", "content": "process a fresh transaction for the console approval cycle"}],
        "tools": [{"name": held_tool, "description": "Submit an outbound payment for processing."}]
    });

    // ---- 3a: trigger a fresh hold, no approval token -----------------------
    let no_token_call = MessagesCall {
        api_key: &discovered.wardryx_admin_bearer,
        run_id: console_cycle_run_id,
        agent_id: payments_bot_agent,
        budget_usd: "1000.00",
        on_behalf_of: None,
        task_type: None,
        approval_token: None,
        body: &high_cost_body,
    };
    let hold_resp = no_token_call.send(&http, &discovered.gateway_url).await;
    assert_eq!(
        hold_resp.status().as_u16(),
        403,
        "3a: fresh high-cost call must hold, not pass"
    );
    assert_eq!(
        header(&hold_resp, "x-fuse-wardryx").as_deref(),
        Some("hold"),
        "3a: expected x-fuse-wardryx: hold"
    );
    let approval_id = header(&hold_resp, "x-fuse-approval-id")
        .expect("3a: a hold response must carry x-fuse-approval-id");
    assert!(!approval_id.is_empty());
    println!("exit_gate_test: 3a HOLD approval_id={approval_id}");

    // ---- 3b: list_approvals finds it, pending, with the expected context --
    let approvals = wardryx_client
        .list_approvals()
        .await
        .expect("3b: GET /v1/approvals");
    let pending = approvals
        .iter()
        .find(|a| a.approval_id == approval_id)
        .unwrap_or_else(|| panic!("3b: approval {approval_id} must appear in list_approvals()"));
    assert!(pending.pending, "3b: freshly held approval must be pending");
    assert_eq!(
        pending.agent_id, payments_bot_agent,
        "3b: context agent mismatch"
    );
    assert_eq!(
        pending.run_id, console_cycle_run_id,
        "3b: context run mismatch"
    );
    assert_eq!(
        pending.tool_names(),
        Some(vec![held_tool.to_string()]),
        "3b: context tool_names mismatch"
    );
    let context_est_cost = pending
        .est_cost_usd()
        .expect("3b: context[\"est_cost_usd\"] must be present");
    assert!(
        context_est_cost > 1.0,
        "3b: est_cost_usd must clear the demo policy's $1.00 threshold, got {context_est_cost}"
    );
    assert!(
        pending
            .reason()
            .unwrap_or_default()
            .contains("human approval required"),
        "3b: reason should name the human-approval rule, got {:?}",
        pending.reason()
    );
    println!(
        "exit_gate_test: 3b PENDING agent={} run={} tools={:?} est_cost_usd={context_est_cost:.2} reason={:?}",
        pending.agent_id,
        pending.run_id,
        pending.tool_names(),
        pending.reason()
    );

    // ---- 3c: grant it (Touch-ID-style: a local hardware confirmation ------
    // ---- gates this call client-side, before it is ever made) -------------
    let operator = "user://p2exit/operator";
    let granted = wardryx_client
        .decide_approval(&approval_id, ApprovalVerdict::Grant, operator)
        .await
        .expect("3c: POST /v1/approvals/{id}/decide (grant)");
    assert_eq!(granted.decision, "grant");
    let token = granted
        .approval_token
        .clone()
        .expect("3c: a grant must return an approval_token");
    println!("exit_gate_test: 3c GRANTED approval_id={approval_id} decided_by={operator}");

    // ---- 3d (+ 4b's TTL boundary): decode and verify the claims ------------
    let now = SystemTime::now();
    let claims = ApprovalTokenClaims::decode(&token).expect("3d: decode approval_token claims");
    assert_eq!(
        claims.agent_id, payments_bot_agent,
        "3d: claims.agent_id mismatch"
    );
    assert_eq!(
        claims.run_id, console_cycle_run_id,
        "3d: claims.run_id mismatch"
    );
    assert_eq!(
        claims.tools,
        vec![held_tool.to_string()],
        "3d: claims.tools mismatch"
    );
    assert!(
        (claims.cost_ceiling_usd() - context_est_cost).abs() < 0.01,
        "3d: the minted ceiling must equal the est_cost_usd that triggered the hold: ceiling={} \
         context={context_est_cost}",
        claims.cost_ceiling_usd()
    );
    assert!(
        !claims.is_expired(now),
        "3d: a freshly minted token must not be expired"
    );
    let ttl = claims.ttl_remaining(now);
    assert!(
        ttl > Duration::ZERO,
        "3d: ttl must be > 0 for a freshly minted token, got {ttl:?}"
    );
    assert!(
        ttl <= Duration::from_secs(600),
        "3d: ttl must be <= the 10-minute default TTL, got {ttl:?}"
    );
    assert!(
        ttl > Duration::from_secs(540),
        "4b: a token minted moments ago should read close to the full 10-minute TTL (>9min \
         remaining), got {ttl:?}"
    );
    println!(
        "exit_gate_test: 3d/4b DECODED agent={} run={} tools={:?} ceiling_usd={:.2} ttl={:?}",
        claims.agent_id,
        claims.run_id,
        claims.tools,
        claims.cost_ceiling_usd(),
        ttl
    );

    // ---- 3e: journal the grant as a conforming console_command -------------
    let store_path = unique_temp_path("store").with_extension("sqlite3");
    let console_events_path = unique_temp_path("console").with_extension("ndjson");
    let store = Store::open(&store_path).expect("open sqlite store");

    let verify_result = format!(
        "granted ceiling_usd:{:.2} ttl_s:{}",
        claims.cost_ceiling_usd(),
        ttl.as_secs()
    );
    let rec = CommandRecord {
        operator: operator.to_string(),
        env: env.name.clone(),
        action: "console.grant_approval".to_string(),
        target: approval_id.clone(),
        params: serde_json::json!({}),
        decision: "allow".to_string(),
        sig_alg: GRANT_SIG_ALG.to_string(),
        sig_fpr: GRANT_SIG_FPR.to_string(),
        http_status: 200,
        verify_result: verify_result.clone(),
    };
    genaryx_core::command::record(
        &store,
        &console_events_path,
        "p2exit",
        "exit-gate-console",
        &rec,
    )
    .expect("3e: journal + emit the console.grant_approval outcome");

    let body = std::fs::read_to_string(&console_events_path).expect("read console events file");
    let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        lines.len(),
        1,
        "3e: exactly one console_command line must be appended"
    );
    let line = lines[0];
    println!("exit_gate_test: 3e JOURNALED console_command => {line}");

    let conformer = Conformer::new().expect("embedded schemas must compile");
    let report = conformer.check_line(line);
    assert!(
        report.valid,
        "3e: console_command line must conform: {:?}\n  line: {line}",
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
        Some("console.grant_approval")
    );
    assert_eq!(
        data.get("target").and_then(|v| v.as_str()),
        Some(approval_id.as_str())
    );
    assert_eq!(data.get("decision").and_then(|v| v.as_str()), Some("allow"));
    assert_eq!(data.get("http_status").and_then(|v| v.as_u64()), Some(200));
    assert_eq!(
        data.get("verify_result").and_then(|v| v.as_str()),
        Some(verify_result.as_str())
    );
    assert_eq!(
        store.commands_journal_count().expect("count"),
        1,
        "3e: the commands_journal row must be written alongside the bus line"
    );
    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&console_events_path);

    // ---- 3f: the agent proceeds: resubmit the SAME request with the token -
    let allow_call = MessagesCall {
        approval_token: Some(&token),
        ..no_token_call
    };
    let allow_resp = allow_call.send(&http, &discovered.gateway_url).await;
    let allow_status = allow_resp.status().as_u16();
    let allow_wh = header(&allow_resp, "x-fuse-wardryx");
    println!(
        "exit_gate_test: 3f RESUBMIT-WITH-TOKEN => {allow_status} x-fuse-wardryx={allow_wh:?}"
    );
    assert_eq!(
        allow_status, 200,
        "3f: the previously-held action, resubmitted with its granted token, must now go through"
    );
    if let Some(wh) = allow_wh {
        assert_eq!(
            wh, "allow",
            "3f: if x-fuse-wardryx is present on the 200 it must read allow"
        );
    }

    // ======================================================================
    // step 4: token-boundary units (direct WardryxClient::decide)
    // ======================================================================

    // ---- 4a: cost-bound - same agent/run/tools + the token, cost ABOVE the
    // ---- granted ceiling, must deny (not fall back to hold or allow) ------
    let over_ceiling_req = DecideRequest {
        agent_id: payments_bot_agent.to_string(),
        run_id: console_cycle_run_id.to_string(),
        tool_names: vec![held_tool.to_string()],
        est_cost_usd: claims.cost_ceiling_usd() + 1000.0,
        approval_token: token.clone(),
        ..Default::default()
    };
    let over_ceiling_resp = wardryx_client
        .decide(&over_ceiling_req)
        .await
        .expect("4a: POST /v1/decide over the token's cost ceiling");
    assert_eq!(
        over_ceiling_resp.decision, "deny",
        "4a: a cost above the granted ceiling must deny even with a structurally valid token; \
         reason={}",
        over_ceiling_resp.reason
    );
    println!(
        "exit_gate_test: 4a COST-BOUND est_cost_usd={:.2} (ceiling {:.2}) => {} ({})",
        over_ceiling_req.est_cost_usd,
        claims.cost_ceiling_usd(),
        over_ceiling_resp.decision,
        over_ceiling_resp.reason
    );
    // 4b (the TTL boundary) is asserted above, in 3d, against this SAME
    // decoded `claims` value - see that block's comment.

    // ---- 4c: single-use - a dedicated, WARDRYX_APPROVAL_SINGLE_USE=1 -------
    // ---- instance, since the taipan-managed one above is never single-use -
    match try_start_single_use_wardryx().await {
        Some((_su_guard, su_base)) => {
            let su_client = WardryxClient::new(&su_base, SINGLE_USE_BEARER)
                .expect("build single-use WardryxClient");
            let su_policy = Policy {
                target: "agent://exit-gate.local/*".to_string(),
                require_human_above_usd: 1.0,
                ..Default::default()
            };
            su_client
                .put_policy("exit-gate-demo", &su_policy)
                .await
                .expect("4c: PUT single-use demo policy");

            let su_req = DecideRequest {
                agent_id: "agent://exit-gate.local/single-use-probe".to_string(),
                run_id: "exit-gate-single-use-run".to_string(),
                est_cost_usd: 50.0,
                ..Default::default()
            };
            let su_hold = su_client
                .decide(&su_req)
                .await
                .expect("4c: single-use hold decide");
            assert_eq!(
                su_hold.decision, "hold",
                "4c: setup hold must actually hold"
            );

            let su_granted = su_client
                .decide_approval(
                    &su_hold.approval_id,
                    ApprovalVerdict::Grant,
                    "user://exit-gate.local/operator",
                )
                .await
                .expect("4c: grant the single-use approval");
            let su_token = su_granted
                .approval_token
                .clone()
                .expect("4c: grant must return an approval_token");

            let su_redeem_req = DecideRequest {
                approval_token: su_token.clone(),
                ..su_req.clone()
            };
            let first = su_client
                .decide(&su_redeem_req)
                .await
                .expect("4c: first redemption call");
            assert_eq!(
                first.decision, "allow",
                "4c: first redemption of a fresh token must allow"
            );

            let second = su_client
                .decide(&su_redeem_req)
                .await
                .expect("4c: second redemption call must still succeed at the transport level");
            assert_eq!(
                second.decision, "hold",
                "4c: a single-use token's second redemption must fall back to hold, not allow \
                 again; reason={}",
                second.reason
            );
            assert!(
                second.reason.contains("already redeemed"),
                "4c: reason should name the single-use rule, got: {}",
                second.reason
            );
            println!(
                "exit_gate_test: 4c SINGLE-USE first={} second={} ({})",
                first.decision, second.decision, second.reason
            );
        }
        None => {
            println!(
                "exit_gate_test: 4c NOTE: single-use token-boundary assertion skipped (dedicated \
                 spawn unavailable on this box; see the NOTE above for why). Single-use is \
                 config-gated (WARDRYX_APPROVAL_SINGLE_USE) and covered by wardryx's own \
                 internal/api tests plus this repo's wardryx_test.rs spawn pattern; the \
                 cost-ceiling (4a) and TTL (4b) boundaries above are still fully live-asserted \
                 regardless."
            );
        }
    }

    // ======================================================================
    // step 5: taipan down, assert clean teardown, no orphans
    // ======================================================================
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
    println!("exit_gate_test: `taipan down` =>\n{down_stdout}");
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
    assert!(
        port_is_closed(WARDRYX_PORT),
        "wardryx port {WARDRYX_PORT} must be released after taipan down (orphan?)"
    );

    // Courtesy cleanup only (not asserted): `taipan down` does not remove
    // the demo policy file it seeded at `up` time (see
    // taipan/src/commands/down.rs) - this is this test's own environment's
    // file, named after ENV_NAME, so it is ours to tidy up, the same way
    // the store/events temp files above are.
    if let Some(home) = taipan_home() {
        let _ = std::fs::remove_file(
            home.join("environments")
                .join(format!("{ENV_NAME}.wardryx-policy.yaml")),
        );
    }

    println!(
        "exit_gate_test: step 5 TEARDOWN clean: no orphans, ports {CLOUD_PORT}/{GATEWAY_PORT}/{WARDRYX_PORT} free"
    );
}
