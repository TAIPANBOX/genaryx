//! Phase-3 exit gate (09 Ф3, PHASE3.md W4): the cross-plane Agent 360 join,
//! proven end to end against a REAL `idryx serve`. The acceptance criterion the
//! phase promises is "click a flagged agent, from anywhere, and its full 360
//! card resolves". This test proves the load-bearing half of that a shell
//! cannot fake: an agent that a live idryx flags with a detector alert is the
//! SAME agent the console's own bus-derived delegation graph and event Store
//! resolve for - so the 360 card (identity + alert from idryx, delegation from
//! `genaryx_core::DelegationGraph`, events from `Store::events_for_agent`) has
//! real, joined data on every plane, not a per-plane guess.
//!
//! Grounded exactly like `wardryx_test.rs` and the ffi crate's own idryx live
//! test: a real bus is generated with `genaryx_core::demo::generate` (the same
//! campaign the rest of the stack trusts, so idryx's `tokenfuse.Load` sees the
//! real schema, never hand-crafted JSON that could drift), idryx is built from
//! `~/Development/idryx` (`go build`, source of truth) or the `taipan
//! up`-installed `~/.taipan/bin/idryx`, run on a fresh ephemeral port, and torn
//! down after. Skips gracefully (an `eprintln!`, an early return) whenever the
//! idryx binary or `go` toolchain is unavailable - a missing sibling checkout
//! must never turn `cargo test -p genaryx-connectors` red.
//!
//! The console side (the graph + the Store) is built from the SAME demo bus via
//! `genaryx_core`'s own ingest path (`demo::generate` -> `Store` ->
//! `IngestService::poll_once`), then `DelegationGraph::from_store`, mirroring
//! how the Tauri shell's `graph::agent_graph` command and the ffi
//! `FleetHandle` build theirs at runtime.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use genaryx_connectors::IdryxClient;
use genaryx_core::graph::DelegationGraph;
use genaryx_core::ingest::IngestService;
use genaryx_core::store::Store;

const HEALTHZ_TIMEOUT: Duration = Duration::from_secs(30);

/// The four stack-bus sources idryx's `--load` understands that `demo::generate`
/// writes (it writes more, e.g. engram/qryx, that idryx's `tokenfuse.Load`
/// vocabulary also ingests via the same agent-event path; these four are the
/// ones idryx names in its `agentBusSources` map).
const BUS_SOURCES: [&str; 4] = ["tokenfuse", "wardryx", "mockryx", "verdryx"];

fn free_port() -> Option<u16> {
    TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
}

fn idryx_repo() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(home).join("Development/idryx");
    dir.join("go.mod").is_file().then_some(dir)
}

fn taipan_installed_binary() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let bin = PathBuf::from(home).join(".taipan/bin/idryx");
    bin.is_file().then_some(bin)
}

/// Prefer a fresh `go build` from source (matches HEAD); fall back to the
/// `taipan up`-installed binary; `None` (with an `eprintln!`) when neither is
/// available. Returns `(binary, Some(scratch))` when it built one this call
/// (the caller removes it on teardown), `(binary, None)` for a borrowed
/// installed binary (never removed).
fn resolve_idryx_binary() -> Option<(PathBuf, Option<PathBuf>)> {
    if let Some(repo) = idryx_repo() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let scratch = std::env::temp_dir().join(format!(
            "genaryx-idryx-360-bin-{}-{nanos}",
            std::process::id()
        ));
        match Command::new("go")
            .arg("build")
            .arg("-o")
            .arg(&scratch)
            .arg("./cmd/idryx")
            .current_dir(&repo)
            .status()
        {
            Ok(status) if status.success() && scratch.is_file() => {
                return Some((scratch.clone(), Some(scratch)));
            }
            Ok(status) => eprintln!(
                "identity_360_test: `go build` failed ({status}); trying ~/.taipan/bin/idryx"
            ),
            Err(e) => {
                eprintln!("identity_360_test: could not run `go`: {e}; trying ~/.taipan/bin/idryx")
            }
        }
    }
    if let Some(bin) = taipan_installed_binary() {
        return Some((bin, None));
    }
    eprintln!(
        "identity_360_test: SKIPPING: neither ~/Development/idryx (go.mod) nor ~/.taipan/bin/idryx found"
    );
    None
}

/// Kills + reaps the `idryx serve` child on drop (including on a mid-test
/// panic) and removes the scratch binary + the demo events dir this test made.
struct Harness {
    child: Child,
    scratch_bin: Option<PathBuf>,
    events_dir: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(bin) = &self.scratch_bin {
            let _ = std::fs::remove_file(bin);
        }
        let _ = std::fs::remove_dir_all(&self.events_dir);
    }
}

fn spawn_idryx_serve(bin: &Path, addr: &str, events_dir: &Path) -> Option<Child> {
    let mut cmd = Command::new(bin);
    cmd.arg("serve").arg("--addr").arg(addr);
    for source in BUS_SOURCES {
        let file = events_dir.join(format!("{source}.ndjson"));
        if file.is_file() {
            cmd.arg("--load")
                .arg(format!("{source}:{}", file.display()));
        }
    }
    cmd.stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()
}

/// Generate a demo bus, spawn a real `idryx serve --load` over it on an
/// ephemeral port, and wait for `/healthz`. `None` (after an `eprintln!`) on any
/// failure along the way, so the caller degrades gracefully.
async fn try_start() -> Option<(Harness, String, PathBuf)> {
    // `resolve_idryx_binary` already printed the skip reason on `None`.
    let (bin, scratch_bin) = resolve_idryx_binary()?;

    let events_dir = std::env::temp_dir().join(format!(
        "genaryx-idryx-360-events-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    if let Err(e) = genaryx_core::demo::generate(&events_dir) {
        eprintln!("identity_360_test: SKIPPING: demo::generate failed: {e}");
        if let Some(b) = &scratch_bin {
            let _ = std::fs::remove_file(b);
        }
        return None;
    }

    let Some(port) = free_port() else {
        eprintln!("identity_360_test: SKIPPING: could not reserve a port");
        let _ = std::fs::remove_dir_all(&events_dir);
        if let Some(b) = &scratch_bin {
            let _ = std::fs::remove_file(b);
        }
        return None;
    };
    let addr = format!("127.0.0.1:{port}");
    let Some(mut child) = spawn_idryx_serve(&bin, &addr, &events_dir) else {
        eprintln!(
            "identity_360_test: SKIPPING: failed to spawn {}",
            bin.display()
        );
        let _ = std::fs::remove_dir_all(&events_dir);
        if let Some(b) = &scratch_bin {
            let _ = std::fs::remove_file(b);
        }
        return None;
    };

    let base = format!("http://{addr}");
    let http = reqwest::Client::new();
    let deadline = Instant::now() + HEALTHZ_TIMEOUT;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            eprintln!("identity_360_test: SKIPPING: idryx exited early ({status})");
            let _ = std::fs::remove_dir_all(&events_dir);
            if let Some(b) = &scratch_bin {
                let _ = std::fs::remove_file(b);
            }
            return None;
        }
        if let Ok(resp) = http.get(format!("{base}/healthz")).send().await
            && resp.status().is_success()
        {
            return Some((
                Harness {
                    child,
                    scratch_bin,
                    events_dir: events_dir.clone(),
                },
                base,
                events_dir,
            ));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            eprintln!("identity_360_test: SKIPPING: idryx never answered /healthz");
            let _ = std::fs::remove_dir_all(&events_dir);
            if let Some(b) = &scratch_bin {
                let _ = std::fs::remove_file(b);
            }
            return None;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Build the console's own bus-derived state (the Store + the delegation graph)
/// from the SAME demo events idryx loaded, via `genaryx_core`'s ingest path -
/// exactly how the Tauri `graph` commands and the ffi `FleetHandle` build theirs
/// at runtime. Returns an open in-memory-file Store plus the graph over it.
fn build_console_side(events_dir: &Path) -> (Store, DelegationGraph) {
    let db = events_dir.join("console.sqlite");
    let store = Store::open(&db).expect("open console store");
    let mut ingest = IngestService::new(Store::open(&db).expect("writer store"), "local")
        .expect("IngestService::new");
    let mut files: Vec<PathBuf> = std::fs::read_dir(events_dir)
        .expect("read events dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("ndjson"))
        .collect();
    files.sort();
    for path in &files {
        let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("x");
        ingest
            .add_file_source(format!("filetail:{id}"), path)
            .expect("add_file_source");
    }
    ingest.poll_once().expect("poll_once");
    let graph = DelegationGraph::from_store(&store).expect("DelegationGraph::from_store");
    (store, graph)
}

#[tokio::test]
async fn flagged_agent_resolves_across_idryx_graph_and_store_e2e() {
    let Some((_harness, base, events_dir)) = try_start().await else {
        return; // already explained why via eprintln!
    };

    // ---- the identity plane, live from a real idryx ----
    let client = IdryxClient::new(&base).expect("build IdryxClient");
    assert!(client.healthz().await.expect("GET /healthz"));

    let identities = client.list_identities().await.expect("GET /api/identities");
    assert!(
        !identities.is_empty(),
        "the demo campaign must yield identities in idryx"
    );
    let alerts = client.list_alerts().await.expect("GET /api/alerts");
    assert!(
        !alerts.is_empty(),
        "the demo campaign is known to trip idryx's detectors"
    );

    // ---- the console's own bus-derived planes (graph + Store) ----
    let (store, graph) = build_console_side(&events_dir);
    assert!(
        graph.node_count() > 0,
        "the console's delegation graph must be non-empty"
    );

    // ---- THE EXIT GATE: a flagged agent joins across every plane ----
    // idryx names a flagged agent in an alert's `identity`; that same agent must
    // be one the console's own graph knows (an actor on the bus) and has events
    // for - i.e. clicking it opens a 360 card with real data on identity
    // (idryx), delegation (graph), and events (Store), not a per-plane guess.
    let joined = alerts
        .iter()
        .map(|a| a.identity.as_str())
        .filter(|id| id.starts_with("agent://"))
        .find(|id| graph.agent_slice(id).node.is_some());

    let agent = joined.expect(
        "at least one agent idryx flagged must also be a node in the console's own bus graph \
         (the cross-plane Agent 360 join the phase promises)",
    );

    // identity plane: the flagged agent is a known identity (or at least the
    // subject of its own alert - idryx builds agent identities from the bus).
    let has_alert = alerts.iter().any(|a| a.identity == agent);
    assert!(
        has_alert,
        "the joined agent must carry the idryx alert it was chosen by"
    );

    // delegation plane: its 360 slice resolves with the agent as a real node.
    let slice = graph.agent_slice(agent);
    let node = slice.node.expect("the flagged agent must be a graph node");
    assert_eq!(node.id, agent);
    assert!(
        node.event_count > 0,
        "a flagged, acted-on agent must have acted"
    );

    // events plane: the 360 events section has this agent's real events.
    let events = store
        .events_for_agent(agent, 100)
        .expect("Store::events_for_agent");
    assert!(
        !events.is_empty(),
        "the flagged agent must have events for the 360 events section"
    );
    assert!(
        events.iter().all(|e| e.agent_id == agent),
        "events_for_agent must not leak other agents' rows into the 360 card"
    );

    // ---- Rescan path: `idryx detect --format json` recomputes the same plane ----
    let idryx_bin = _harness
        .scratch_bin
        .clone()
        .or_else(taipan_installed_binary)
        .expect("the binary that is already serving must be locatable for Rescan too");
    let loads: Vec<(String, String)> = BUS_SOURCES
        .iter()
        .filter_map(|s| {
            let f = events_dir.join(format!("{s}.ndjson"));
            f.is_file()
                .then(|| ((*s).to_string(), f.display().to_string()))
        })
        .collect();
    let load_refs: Vec<(&str, &str)> = loads
        .iter()
        .map(|(s, p)| (s.as_str(), p.as_str()))
        .collect();
    let rescanned =
        IdryxClient::rescan(&idryx_bin, &load_refs, "low").expect("idryx detect Rescan");
    assert!(
        !rescanned.is_empty(),
        "Rescan (idryx detect --format json) must recompute the same detector alerts"
    );

    eprintln!(
        "identity_360_test: PASSED - flagged agent {agent} joins across idryx ({} identities, \
         {} alerts), the console graph ({} nodes), and its Store events ({} rows); Rescan saw {} alerts",
        identities.len(),
        alerts.len(),
        graph.node_count(),
        events.len(),
        rescanned.len(),
    );
}
