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
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

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
    require_break_glass_reason(rec)?;

    let ts = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

    store.insert_command(rec, &ts)?;

    // The console's own chain, advanced in memory by the one sink that owns
    // this file (see [`ChainSink`]). Deriving `prev_hash` by re-reading the
    // file tail here is what used to fork it: anything appended in between,
    // by a product or by another console command, became the link target.
    let sink = sink_for(console_events_path)?;
    let mut sink = sink.lock().unwrap_or_else(|e| e.into_inner());
    let ChainSink { file, next } = &mut *sink;
    append_chained(file, next, |prev| {
        console_command_line(org_domain, host, rec, &ts, prev)
    })
}

/// One console events file, open for append, plus the `prev_hash` its NEXT
/// line will carry.
///
/// The chain is seeded from the file's tail ONCE, in [`sink_for`], and
/// advanced in memory from then on, so one file stays one chain across a
/// restart without ever re-reading what other writers have done since. This
/// is the shape the estate already implements twice: TokenFuse's
/// `Exporter` (`crates/core/src/agent_event.rs`) and heraldyx's
/// `internal/record`. It is ported rather than reinvented, because a chain
/// written by three different mechanisms is three chances to get it wrong.
struct ChainSink {
    file: std::fs::File,
    /// `prev_hash` for the next event; `None` at a chain head.
    next: Option<String>,
}

/// Every console events file this process has opened, one [`ChainSink`]
/// each. Process-wide and keyed by path because the callers build their
/// `BusHandle` per request (`crates/web/src/dispatch.rs`'s `wg_journal` is
/// the clearest case), so the sink cannot live on the handle: it has to
/// outlive it, or two commands in flight at once each get their own idea of
/// where the chain is.
///
/// One writer per file is the invariant this upholds, and it is only true
/// for THIS process. It holds in practice because the file is the console's
/// own (see `money::state::CONSOLE_EVENTS_FILE`) and nothing else writes it.
static SINKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<ChainSink>>>>> = OnceLock::new();

/// The sink that owns `path`, opening and seeding it on first use.
///
/// Keyed by the canonicalized parent plus the file name rather than by the
/// path as given: on macOS the temp directory is reached both as `/tmp` and
/// as `/private/tmp`, and two spellings of one file would otherwise get two
/// sinks, which is exactly the forked chain this whole change removes. The
/// file name itself is not canonicalized (the file may not exist yet).
fn sink_for(path: &Path) -> Result<Arc<Mutex<ChainSink>>> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let key = canonical_key(path);

    let registry = SINKS.get_or_init(|| Mutex::new(HashMap::new()));
    // A poisoned registry is recovered rather than propagated: a panic in
    // some other command must not stop this box journaling for the rest of
    // its life. The map itself is only ever inserted into.
    let mut guard = registry.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = guard.get(&key) {
        return Ok(existing.clone());
    }

    let file = OpenOptions::new().create(true).append(true).open(path)?;
    // Resume whatever chain the file already holds, so a console restart
    // continues one chain instead of starting a second beside it. An empty
    // file, or a tail that does not parse, correctly starts a fresh one.
    let next = last_line_of(path).and_then(|l| chain_hash_of_line(&l));
    let sink = Arc::new(Mutex::new(ChainSink { file, next }));
    guard.insert(key, sink.clone());
    Ok(sink)
}

fn canonical_key(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => parent
            .canonicalize()
            .map_or_else(|_| path.to_path_buf(), |p| p.join(name)),
        _ => path.to_path_buf(),
    }
}

/// Build one line with the chain link `next` currently holds, write it and
/// its newline in a SINGLE write, then advance `next`.
///
/// Two properties, and both are load-bearing:
///
/// 1. **One write.** A line written without its newline, and the newline
///    written after, lets a concurrent `O_APPEND` write land in between and
///    produce one line that is two half-events and parses as neither.
///    TokenFuse's exporter frames the same way, for the same reason.
/// 2. **The chain does not advance on a failed write.** `?` on `write_all`
///    skips the assignment, so the next successful line re-links to the last
///    line actually on disk rather than to one that never reached it.
///
/// Generic over the writer purely so the framing can be asserted directly in
/// a test; the only production writer is the sink's `File`.
fn append_chained<W: Write>(
    w: &mut W,
    next: &mut Option<String>,
    build: impl FnOnce(Option<&str>) -> Result<String>,
) -> Result<()> {
    let line = build(next.as_deref())?;
    // Computed before the write and from the line itself: `chain_hash_of_line`
    // strips `prev_hash` first, so this is the same value a verifier
    // recomputes from the bytes on disk.
    let advanced = chain_hash_of_line(&line);
    let mut framed = line.into_bytes();
    framed.push(b'\n');
    w.write_all(&framed)?;
    *next = advanced;
    Ok(())
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
    prev_hash: Option<&str>,
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
    // Last, and only when there is one: a head event carries no `prev_hash`
    // at all rather than an empty or null field, which is what the schema and
    // the Go writer both do.
    if let Some(prev) = prev_hash {
        obj.insert("prev_hash".to_string(), Value::String(prev.to_string()));
    }

    Ok(serde_json::to_string(&Value::Object(obj))?)
}

/// The SPEC 6.5 chain hash of one serialized event line: the value the NEXT
/// event carries as its `prev_hash`.
///
/// Byte-identical to `agent-stack-go`'s `event.ChainHash`, because a chain the
/// products write and the console does not join is not one chain: the
/// conformance checker verifies a single sequence per file, so a console line
/// computed differently would break the very chain it is meant to extend.
/// `prev_hash` is removed before hashing (SPEC 6.5 defines the hash over the
/// event WITHOUT it), keys are emitted in sorted order, and there is no
/// whitespace.
///
/// The sort comes from `serde_json`'s `Map` being a `BTreeMap` in this build
/// (no `preserve_order` feature), which orders by UTF-8 bytes. That equals
/// RFC 8785's UTF-16 ordering for every key in this envelope, all of which are
/// ASCII - and the envelope's key set is fixed by the spec, so this cannot
/// drift into non-ASCII without a spec change.
pub fn chain_hash_of_line(line: &str) -> Option<String> {
    let mut value: Value = serde_json::from_str(line).ok()?;
    if let Some(obj) = value.as_object_mut() {
        obj.remove("prev_hash");
    }
    let canonical = serde_json::to_vec(&value).ok()?;
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(&canonical);
    Some(format!("sha256:{digest:x}"))
}

/// The last non-empty line of `path`, or `None` when the file is absent or
/// empty (a head event carries no `prev_hash`, which is what an empty file
/// correctly produces).
///
/// Reads the whole file rather than seeking its tail. That was already a
/// generous trade when this ran once per privileged console action; it now
/// runs once per file per process, at [`sink_for`], so a correct tail read
/// across a partially-written last line is machinery this does not need.
fn last_line_of(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(str::to_string)
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

/// Fail-closed guard for the break-glass path (06 §0.5; Phase-2 wave 3B).
/// A `decision == "break_glass"` record is an operator OVERRIDE of governance
/// and MUST carry a non-empty, non-whitespace justification in
/// `params["reason"]`; [`record`] calls this first and refuses
/// ([`crate::error::Error::BreakGlassMissingReason`]) before journaling or
/// emitting anything, so an unjustified override is never recorded at all.
/// The reason rides in `params` (which [`crate::store::Store::insert_command`]
/// persists to the audit trail) but is deliberately NOT copied into the
/// fixed-shape `console_command` bus event (see [`console_command_line`]): the
/// bus shows THAT an override happened, the queryable journal holds WHY. Any
/// other decision (`"allow"` - the Wardryx-approved or non-privileged path)
/// needs no reason and passes straight through.
fn require_break_glass_reason(rec: &CommandRecord) -> Result<()> {
    if rec.decision == "break_glass" {
        let reason = rec
            .params
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("");
        if reason.trim().is_empty() {
            return Err(crate::error::Error::BreakGlassMissingReason);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(decision: &str, params: Value) -> CommandRecord {
        CommandRecord {
            operator: "user://acme/alice".to_string(),
            env: "local".to_string(),
            action: "console.kill_run".to_string(),
            target: "run-1".to_string(),
            params,
            decision: decision.to_string(),
            sig_alg: "none".to_string(),
            sig_fpr: "test".to_string(),
            http_status: 200,
            verify_result: "killed:true".to_string(),
        }
    }

    #[test]
    fn break_glass_without_a_reason_is_refused() {
        assert!(matches!(
            require_break_glass_reason(&rec("break_glass", json!({}))),
            Err(crate::error::Error::BreakGlassMissingReason)
        ));
        assert!(matches!(
            require_break_glass_reason(&rec("break_glass", json!({ "reason": "   " }))),
            Err(crate::error::Error::BreakGlassMissingReason)
        ));
        assert!(matches!(
            require_break_glass_reason(&rec("break_glass", json!({ "reason": 42 }))),
            Err(crate::error::Error::BreakGlassMissingReason)
        ));
    }

    #[test]
    fn break_glass_with_a_reason_passes() {
        assert!(
            require_break_glass_reason(&rec("break_glass", json!({ "reason": "runaway spend" })))
                .is_ok()
        );
    }

    #[test]
    fn a_non_break_glass_decision_needs_no_reason() {
        assert!(require_break_glass_reason(&rec("allow", json!({}))).is_ok());
    }

    /// Records every `write_all` it is handed, so the framing can be asserted
    /// as what it is: a count of writes, not a shape of bytes.
    #[derive(Default)]
    struct CountingWriter(Vec<Vec<u8>>);

    impl Write for CountingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.push(buf.to_vec());
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// The defect this pins: writing the line and then the newline is two
    /// syscalls, and a concurrent `O_APPEND` write lands between them.
    #[test]
    fn a_line_and_its_newline_are_one_write() {
        let mut w = CountingWriter::default();
        let mut next = None;
        append_chained(&mut w, &mut next, |prev| {
            assert_eq!(prev, None, "an empty chain starts at a head");
            Ok(r#"{"source":"console"}"#.to_string())
        })
        .expect("append");

        assert_eq!(w.0.len(), 1, "one event must reach the file in one write");
        assert_eq!(w.0[0], b"{\"source\":\"console\"}\n");
    }

    /// The link is advanced in memory from the line just written, never by
    /// re-reading the file, which is what makes a foreign append harmless.
    #[test]
    fn the_next_link_comes_from_the_line_just_written() {
        let mut w = CountingWriter::default();
        let mut next = None;
        let first = r#"{"source":"console","ts":"1"}"#;
        append_chained(&mut w, &mut next, |_| Ok(first.to_string())).expect("append");
        assert_eq!(next, chain_hash_of_line(first));

        append_chained(&mut w, &mut next, |prev| {
            assert_eq!(
                prev,
                chain_hash_of_line(first).as_deref(),
                "the second line must link to the first"
            );
            Ok(r#"{"source":"console","ts":"2"}"#.to_string())
        })
        .expect("append");
    }

    /// A write that fails must not advance the chain: the next successful
    /// line has to re-link to the last line actually on disk.
    #[test]
    fn a_failed_write_does_not_advance_the_chain() {
        struct Failing;
        impl Write for Failing {
            fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk gone"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let mut next = Some("sha256:aa".to_string());
        let err = append_chained(&mut Failing, &mut next, |_| {
            Ok(r#"{"source":"console"}"#.to_string())
        });
        assert!(err.is_err());
        assert_eq!(
            next,
            Some("sha256:aa".to_string()),
            "the chain must stay where it was"
        );
    }

    /// One file, one sink: two lookups of the same path (in any spelling the
    /// OS resolves to it) must hand back the same writer, or two commands in
    /// flight each advance their own chain.
    #[test]
    fn one_file_gets_exactly_one_sink() {
        let dir = std::env::temp_dir().join(format!(
            "genaryx-sink-identity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let path = dir.join("console.ndjson");
        let a = sink_for(&path).expect("open the sink");
        let b = sink_for(&dir.join(".").join("console.ndjson")).expect("reopen the same file");
        assert!(
            Arc::ptr_eq(&a, &b),
            "the same file must resolve to the same sink"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
