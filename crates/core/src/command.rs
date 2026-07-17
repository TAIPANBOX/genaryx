//! CommandBroker: the governance record for a privileged console mutation
//! (06 §2). When the operator kills a run or changes a budget, the actual
//! signed Cloud call is made by the shell through
//! `genaryx-connectors::CloudClient` (Phase-1 wave 1, commit c693803);
//! `crates/core` must not depend on `crates/connectors` (it already
//! path-deps `core`, so the reverse edge would be a dependency cycle). This
//! module records the OUTCOME the shell hands back instead of executing
//! anything itself:
//!
//! 1. a row in `commands_journal` (the durable, queryable audit trail), and
//! 2. a conforming `console_command` agent-event, appended to the same
//!    NDJSON file the console's own `FileTail`/`IngestService` reads (06
//!    §0.4: "the console is itself an agent of the stack"), so the
//!    operator's own privileged action lands on the same bus the Bus
//!    Explorer displays, instead of being a UI-only side effect.
//!
//! Clean split: the connector executes and signs; the broker journals and
//! emits. Nothing here calls out to the network or to signing, both inputs
//! (the signing metadata, the HTTP status) are already-decided facts by the
//! time [`record`] is called.

use crate::error::Result;
use crate::event::SchemaVersion;
use crate::store::Store;
use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value, json};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

/// The outcome of one privileged console mutation (kill / budget change /
/// incident ack), already executed and signed elsewhere (by the shell, via
/// `genaryx-connectors::CloudClient`). [`record`] turns this into a
/// `commands_journal` row plus a conforming `console_command` bus event; it
/// never performs the mutation itself.
#[derive(Debug, Clone)]
pub struct CommandRecord {
    /// The principal that requested the mutation, e.g. `user://org/alice`.
    pub operator: String,
    /// Environment id, matching [`crate::event::Provenance::env`].
    pub env: String,
    /// e.g. `console.kill_run`, `console.set_budget`, `console.ack_incident`.
    pub action: String,
    /// The run id / incident id the mutation applies to.
    pub target: String,
    /// Action-specific parameters, e.g. `{"budget_usd":12.5}` for a budget
    /// change, `{}` for a kill. Journaled to `commands_journal` for the
    /// audit trail; deliberately NOT copied into the bus event's `data` (see
    /// [`console_command_line`]) so the emitted envelope has one fixed shape
    /// regardless of `action`.
    pub params: Value,
    /// `allow` (Wardryx-approved) or `break_glass` (operator override).
    pub decision: String,
    /// Signature algorithm, e.g. `es256`.
    pub sig_alg: String,
    /// Short key fingerprint, or an honest assurance label
    /// (`secure-enclave` / `software-signed`, matching
    /// `genaryx-signing::es256::Assurance::label`).
    pub sig_fpr: String,
    /// The HTTP status the Cloud returned for the signed mutation call.
    pub http_status: u16,
    /// A short human-readable verification result, e.g. `killed:true`.
    pub verify_result: String,
}

/// Journal `rec` and emit its `console_command` bus event: a
/// `commands_journal` row first, then one `\n`-terminated line appended to
/// `console_events_path`. Both share one timestamp, captured once here, so
/// the row and the emitted line always agree on when the command happened.
///
/// `org_domain` and `host` build the acting `agent_id`
/// (`agent://<org_domain>/console/<host>`); see [`console_command_line`] for
/// how `host` is made safe.
///
/// Fail-closed: any store or filesystem failure returns
/// [`crate::error::Error`] rather than silently dropping either half. This
/// is not atomic across the two writes: if the journal insert commits but
/// the file append fails, the caller sees an `Err` even though a durable
/// audit row now exists without a matching bus event yet. The insert is a
/// plain `INSERT` (no uniqueness constraint on the journaled fields), so a
/// caller that retries the whole [`record`] call on error is safe, it will
/// simply journal a second row alongside a second (successful) bus line.
pub fn record(
    store: &Store,
    console_events_path: &Path,
    org_domain: &str,
    host: &str,
    rec: &CommandRecord,
) -> Result<()> {
    let ts = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

    store.insert_command(rec, &ts)?;

    let line = console_command_line(org_domain, host, rec, &ts)?;
    append_line(console_events_path, &line)
}

/// Build the `console_command` agent-event line for `rec`, without touching
/// the filesystem (used directly by [`record`], and by tests that only need
/// to check conformance). `ts` is an RFC 3339 timestamp; [`record`] passes
/// the same one it journals, so the row and the line always match.
///
/// Fixed shape: schema v0.2, `source:"console"`, `type:"console_command"`,
/// `agent_id:"agent://<org_domain>/console/<host>"` (host lowercased and
/// sanitized, see [`sanitize_host`]), `on_behalf_of:[operator]` only when
/// `operator` matches the envelope's `(agent|user)://` principal pattern,
/// and `data` holding exactly the seven outcome fields the spec calls for
/// (`action`, `target`, `decision`, `sig_alg`, `sig_fpr`, `http_status`,
/// `verify_result`), never `params` (see [`CommandRecord::params`]).
pub fn console_command_line(
    org_domain: &str,
    host: &str,
    rec: &CommandRecord,
    ts: &str,
) -> Result<String> {
    let agent_id = format!("agent://{org_domain}/console/{}", sanitize_host(host));

    let data = json!({
        "action": rec.action,
        "target": rec.target,
        "decision": rec.decision,
        "sig_alg": rec.sig_alg,
        "sig_fpr": rec.sig_fpr,
        "http_status": rec.http_status,
        "verify_result": rec.verify_result,
    });

    let mut obj = Map::new();
    obj.insert(
        "schema".to_string(),
        Value::String(SchemaVersion::V0_2.as_str().to_string()),
    );
    obj.insert("ts".to_string(), Value::String(ts.to_string()));
    obj.insert("source".to_string(), Value::String("console".to_string()));
    obj.insert(
        "type".to_string(),
        Value::String("console_command".to_string()),
    );
    obj.insert("agent_id".to_string(), Value::String(agent_id));
    if is_delegatable_principal(&rec.operator) {
        obj.insert(
            "on_behalf_of".to_string(),
            Value::Array(vec![Value::String(rec.operator.clone())]),
        );
    }
    obj.insert("data".to_string(), data);

    Ok(serde_json::to_string(&Value::Object(obj))?)
}

/// Whether `s` matches the envelope's `on_behalf_of` item pattern
/// (`^(agent|user)://`, 07 §1). An operator id that does not match this
/// (should never happen for a real operator, but the broker never assumes
/// that) is left out of `on_behalf_of` rather than emitted and failing
/// conformance.
fn is_delegatable_principal(s: &str) -> bool {
    s.starts_with("agent://") || s.starts_with("user://")
}

/// Lowercase `host` and fold every character outside `[a-z0-9.-]` to `-`, so
/// an arbitrary OS hostname (mixed case, a trailing `.local`, the occasional
/// odd character) always yields a conforming `agent_id`
/// (`^agent://[a-z0-9.-]+/[a-z0-9._/-]+$`, 07 §1). The charset kept here is a
/// deliberately conservative subset of what the second path segment actually
/// allows (which is also permits `_` and `/`), so the sanitized result is
/// always safe wherever it lands in the id. Falls back to `unknown-host` if
/// nothing survives (e.g. an empty hostname).
fn sanitize_host(host: &str) -> String {
    let sanitized: String = host
        .trim()
        .chars()
        .map(|c| {
            let lower = c.to_ascii_lowercase();
            if lower.is_ascii_alphanumeric() || lower == '.' || lower == '-' {
                lower
            } else {
                '-'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "unknown-host".to_string()
    } else {
        sanitized
    }
}

/// Append `line` plus a trailing `\n` to `path`, creating the parent
/// directory and the file itself if either is missing. Always appends, never
/// truncates: the console events file is a log every source, including the
/// console's own commands, writes to over time.
fn append_line(path: &Path, line: &str) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}
