//! The Evidence Center's Agent-BOM leg, against the real idryx binary.
//!
//! Two things this closes, both found while giving the console its own
//! events file (`console.ndjson`):
//!
//! 1. `IdryxClient::agent_bom` invoked `idryx agent-bom`. idryx's subcommand
//!    is `bom`; `agent-bom` appears in idryx only as OUTPUT text ("idryx
//!    agent-bom: N agent(s)"). Every Agent-BOM build failed with
//!    `unknown command "agent-bom"` and exit 1, so the Evidence pack's
//!    Agent-BOM artifact was never produced on any box that had idryx.
//! 2. The console's own file has to be loaded under a prefix idryx accepts.
//!    `--load console:<path>` exits 1 with `unknown source "console"`, and
//!    one rejected spec aborts the whole run, so the console's file rides
//!    idryx's generic agent-event-bus prefix. Attribution is unaffected:
//!    idryx reads every identity's source from the envelope, never from the
//!    prefix (`internal/ingest/tokenfuse`'s package doc).
//!
//! Skips gracefully when idryx is not installed, the same posture as every
//! other live-tool test in this crate.

use genaryx_connectors::IdryxClient;
use std::path::PathBuf;

fn idryx_bin() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let candidate = PathBuf::from(home)
        .join(".taipan")
        .join("bin")
        .join("idryx");
    candidate.is_file().then_some(candidate)
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "genaryx-idryx-bom-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

const TOKENFUSE_LINE: &str = r#"{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-08-06T10:00:00.000Z","source":"tokenfuse","type":"tool_call","severity":"low","agent_id":"agent://acme.example/support/tier1-bot","data":{}}"#;

const CONSOLE_LINE: &str = r#"{"schema":"taipanbox.dev/agent-event/v0.2","ts":"2026-08-06T10:01:00.000Z","source":"console","type":"console_command","agent_id":"agent://acme.example/console/box","on_behalf_of":["user://acme.example/alice"],"data":{"action":"console.kill_run","target":"run-1","decision":"allow","sig_alg":"es256","sig_fpr":"software-signed","http_status":200,"verify_result":"killed:true"}}"#;

#[test]
fn agent_bom_runs_and_carries_the_consoles_own_events() {
    let Some(bin) = idryx_bin() else {
        eprintln!("idryx_agent_bom: SKIPPING: ~/.taipan/bin/idryx is not installed");
        return;
    };

    let dir = scratch("console");
    let tokenfuse = dir.join("tokenfuse.ndjson");
    let console = dir.join("console.ndjson");
    std::fs::write(&tokenfuse, format!("{TOKENFUSE_LINE}\n")).expect("write tokenfuse fixture");
    std::fs::write(&console, format!("{CONSOLE_LINE}\n")).expect("write console fixture");

    // Exactly the specs `evidence::env::agent_bom_loads` now produces: the
    // console's file under idryx's generic agent-event-bus prefix.
    let tokenfuse_s = tokenfuse.display().to_string();
    let console_s = console.display().to_string();
    let loads = [
        ("tokenfuse", tokenfuse_s.as_str()),
        ("tokenfuse", console_s.as_str()),
    ];

    let out = IdryxClient::agent_bom(&bin, &loads).expect("`idryx bom` must run and succeed");
    let doc: serde_json::Value = serde_json::from_slice(&out).expect("Agent-BOM must be JSON");

    let refs: Vec<String> = doc
        .get("components")
        .and_then(|c| c.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|c| c.get("bom-ref").and_then(|r| r.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    assert!(
        refs.iter().any(|r| r == "agent://acme.example/console/box"),
        "the console's own privileged action must appear in the Agent-BOM: {refs:?}"
    );
    assert!(
        refs.iter()
            .any(|r| r == "agent://acme.example/support/tier1-bot"),
        "and it must not have displaced the products' own agents: {refs:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The console's natural spelling is not a prefix idryx resolves, and the
/// failure is not confined to that one source: idryx returns on the first bad
/// spec, so a valid tokenfuse load alongside it produces nothing at all. This
/// is why `evidence::env` maps the prefix instead of emitting `console:`.
#[test]
fn a_console_prefixed_load_would_take_the_whole_agent_bom_down() {
    let Some(bin) = idryx_bin() else {
        eprintln!("idryx_agent_bom: SKIPPING: ~/.taipan/bin/idryx is not installed");
        return;
    };

    let dir = scratch("prefix");
    let tokenfuse = dir.join("tokenfuse.ndjson");
    let console = dir.join("console.ndjson");
    std::fs::write(&tokenfuse, format!("{TOKENFUSE_LINE}\n")).expect("write tokenfuse fixture");
    std::fs::write(&console, format!("{CONSOLE_LINE}\n")).expect("write console fixture");

    let tokenfuse_s = tokenfuse.display().to_string();
    let console_s = console.display().to_string();
    let err = IdryxClient::agent_bom(
        &bin,
        &[
            ("tokenfuse", tokenfuse_s.as_str()),
            ("console", console_s.as_str()),
        ],
    )
    .expect_err("idryx does not resolve a `console:` load prefix");
    let message = err.to_string();
    assert!(
        message.contains("unknown source"),
        "expected idryx to reject the prefix, got: {message}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
