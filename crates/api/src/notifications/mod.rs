//! The Notifications panel: what heraldyx sent, and to whom (CLAUDE.md
//! invariant 51).
//!
//! heraldyx, the stack's mailer, writes one agent-event per message it sends
//! (`type: alert_sent`) to a hash-chained NDJSON journal on its own volume,
//! `sent.ndjson` (heraldyx `internal/record`). Each record names who was
//! written to, what about, which transport carried it, and whether that
//! transport took it ("accepted", never "delivered"). The console reads that
//! file read-only, where the operator mounts it and names it in
//! [`SENT_ENV`], and checks its chain the way `heraldyx --journal` does.
//!
//! # What this reads, and how much
//!
//! The newest [`MAX_READ_BYTES`] of the file, no more: a journal is appended
//! for the life of a box, and a panel read must not grow with it. When the
//! file is longer, the first (partial) line of the window is dropped and the
//! answer says the window is a tail, so a chain judged from it says what it
//! could not see. Of the records in the window the newest [`MAX_RECORDS`] are
//! returned, newest first; the counts cover the whole window.
//!
//! A path that is not a regular file is not read: a link is not followed and
//! a directory is not listed, so the setting cannot be turned into a way to
//! read some other file on the box.
//!
//! # What it never does
//!
//! It writes nothing, holds no client for heraldyx, and invents nothing: no
//! setting is "not configured", an absent or unreadable file is that, and an
//! empty journal is "no messages recorded", three answers a panel shows apart.
//! The text it returns (recipients, the dedup key, a transport's error) is the
//! journal's own and is rendered as text.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::seal::jcs;

/// The setting that names heraldyx's dispatch journal.
pub const SENT_ENV: &str = "GENARYX_HERALDYX_SENT";

/// The most of the file read for one answer: the newest bytes.
pub const MAX_READ_BYTES: u64 = 4 << 20;

/// The most records returned, newest first.
pub const MAX_RECORDS: usize = 500;

/// The most skipped lines named one by one; the rest are counted.
pub const MAX_SKIPPED_NAMED: usize = 20;

/// The longest any one string from the journal is passed on.
pub const MAX_FIELD_CHARS: usize = 512;

/// The most recipients passed on for one record.
pub const MAX_RECIPIENTS: usize = 50;

/// The journal path named by `raw`, or `None` when it is blank.
pub fn sent_path_from(raw: &str) -> Option<PathBuf> {
    let t = raw.trim();
    (!t.is_empty()).then(|| PathBuf::from(t))
}

/// [`sent_path_from`] over the process environment.
pub fn sent_path_from_env() -> Option<PathBuf> {
    sent_path_from(&std::env::var(SENT_ENV).unwrap_or_default())
}

/// One message heraldyx recorded sending.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SentRecord {
    /// 1-based line in the window read.
    pub line: usize,
    pub ts: String,
    /// `alert`, `digest`, `pulse`, ... as heraldyx wrote it.
    pub kind: String,
    /// The dedup key the message was raised under, `{type}:{subject}`.
    pub about: String,
    pub to: Vec<String>,
    /// How many recipients the record named, when more than [`MAX_RECIPIENTS`]
    /// were cut from `to`.
    pub to_total: usize,
    pub transport: String,
    /// `accepted` or `refused` as heraldyx wrote it; anything else is passed
    /// on as written and counted under neither.
    pub outcome: String,
    pub error: Option<String>,
    pub agent_id: String,
    pub run_id: Option<String>,
}

/// A line of the window that is not a record, and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SkippedLine {
    pub line: usize,
    pub reason: String,
}

/// What the SPEC 6.5 chain over the window says.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ChainVerdict {
    /// No line's `prev_hash` disagrees with the line before it.
    Verifies {
        chained: usize,
        heads: usize,
        /// Lines whose `prev_hash` could not be checked: the line before is
        /// malformed, or outside the window.
        unverifiable: usize,
    },
    /// At least one line names a predecessor other than the line before it.
    Broken { first_line: usize, breaks: usize },
    /// One record or none: a chain of one binds nothing, so it is not called
    /// verified.
    NothingBound,
}

/// The panel's answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum NotificationsPanel {
    /// No journal is named on this box.
    NotConfigured { variable: String },
    /// A journal is named and could not be read.
    Unreadable { path: String, reason: String },
    /// The journal was read.
    Read {
        path: String,
        /// Newest first, at most [`MAX_RECORDS`].
        records: Vec<SentRecord>,
        /// Records in the window.
        total: usize,
        accepted: usize,
        refused: usize,
        by_kind: BTreeMap<String, usize>,
        chain: ChainVerdict,
        /// The file is longer than [`MAX_READ_BYTES`]: what was read is its
        /// tail, and the counts and the chain are about that tail.
        window_truncated: bool,
        skipped: Vec<SkippedLine>,
        skipped_total: usize,
    },
}

/// Read the journal at `path` (`None`: not configured).
pub fn notifications_sent(path: Option<&Path>) -> NotificationsPanel {
    let Some(path) = path else {
        return NotificationsPanel::NotConfigured {
            variable: SENT_ENV.to_string(),
        };
    };
    let shown = path.display().to_string();
    match read_window(path) {
        Ok((bytes, truncated)) => fold(shown, &bytes, truncated),
        Err(reason) => NotificationsPanel::Unreadable {
            path: shown,
            reason,
        },
    }
}

/// The newest [`MAX_READ_BYTES`] of a regular file, and whether that is less
/// than the whole of it. A link is not followed and a directory is not read.
fn read_window(path: &Path) -> Result<(Vec<u8>, bool), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            "no file there: heraldyx has recorded nothing yet, or the path is not the one it writes"
                .to_string()
        }
        _ => format!("cannot read it: {e}"),
    })?;
    if !meta.file_type().is_file() {
        return Err(
            "not a regular file: a link is not followed and a directory is not read".to_string(),
        );
    }
    let mut f = std::fs::File::open(path).map_err(|e| format!("cannot open it: {e}"))?;
    let len = f
        .metadata()
        .map_err(|e| format!("cannot read it: {e}"))?
        .len();
    let truncated = len > MAX_READ_BYTES;
    if truncated {
        f.seek(SeekFrom::Start(len - MAX_READ_BYTES))
            .map_err(|e| format!("cannot read it: {e}"))?;
    }
    let mut bytes = Vec::new();
    f.take(MAX_READ_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read it: {e}"))?;
    if truncated {
        // The window starts inside a line; that partial line is not a record.
        match bytes.iter().position(|b| *b == b'\n') {
            Some(i) => {
                bytes.drain(..=i);
            }
            None => bytes.clear(),
        }
    }
    Ok((bytes, truncated))
}

/// The SPEC 6.5 hash of one event line: RFC 8785 over the object with its
/// `prev_hash` member removed. `None` when the object has no canonical form.
pub fn chain_hash(obj: &serde_json::Map<String, Value>) -> Option<String> {
    let mut without = obj.clone();
    without.remove("prev_hash");
    let canonical = jcs::canonical(&Value::Object(without)).ok()?;
    use sha2::{Digest, Sha256};
    Some(format!("sha256:{:x}", Sha256::digest(canonical.as_bytes())))
}

fn bounded(s: &str) -> String {
    if s.chars().count() <= MAX_FIELD_CHARS {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(MAX_FIELD_CHARS).collect();
        t.push_str("...");
        t
    }
}

fn str_field(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(bounded)
}

/// One record from a well-formed event object, or the reason it is not one.
fn record_of(line: usize, obj: &serde_json::Map<String, Value>) -> Result<SentRecord, String> {
    if obj.get("source").and_then(Value::as_str) != Some("heraldyx") {
        return Err("not written by heraldyx".to_string());
    }
    if obj.get("type").and_then(Value::as_str) != Some("alert_sent") {
        return Err("not an alert_sent record".to_string());
    }
    let data = obj
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "no data object".to_string())?;
    let agent_id = str_field(obj.get("agent_id")).ok_or_else(|| "no agent_id".to_string())?;
    let ts = str_field(obj.get("ts")).ok_or_else(|| "no ts".to_string())?;
    let all_to: Vec<&str> = data
        .get("to")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    Ok(SentRecord {
        line,
        ts,
        kind: str_field(data.get("kind")).unwrap_or_default(),
        about: str_field(data.get("about")).unwrap_or_default(),
        to: all_to
            .iter()
            .copied()
            .take(MAX_RECIPIENTS)
            .map(bounded)
            .collect(),
        to_total: all_to.len(),
        transport: str_field(data.get("transport")).unwrap_or_default(),
        outcome: str_field(data.get("outcome")).unwrap_or_default(),
        error: str_field(data.get("error")),
        agent_id,
        run_id: str_field(obj.get("run_id")),
    })
}

fn fold(path: String, bytes: &[u8], window_truncated: bool) -> NotificationsPanel {
    let text = String::from_utf8_lossy(bytes);
    let mut records: Vec<SentRecord> = Vec::new();
    let mut skipped: Vec<SkippedLine> = Vec::new();
    let mut skipped_total = 0usize;
    let mut skip = |line: usize, reason: String| {
        skipped_total += 1;
        if skipped.len() < MAX_SKIPPED_NAMED {
            skipped.push(SkippedLine { line, reason });
        }
    };

    // The chain runs over every well-formed event line, whatever its type.
    let mut prev: Option<String> = None;
    let (mut chained, mut heads, mut unverifiable) = (0usize, 0usize, 0usize);
    let mut breaks: Vec<usize> = Vec::new();
    let mut events = 0usize;

    for (i, raw) in text.split('\n').enumerate() {
        let line_no = i + 1;
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let obj = match serde_json::from_str::<Value>(raw) {
            Ok(Value::Object(o)) => o,
            _ => {
                skip(line_no, "not a JSON object".to_string());
                prev = None;
                continue;
            }
        };
        events += 1;
        match obj.get("prev_hash") {
            None => heads += 1,
            Some(Value::String(claimed)) => match &prev {
                Some(p) if p == claimed => chained += 1,
                Some(_) => breaks.push(line_no),
                // Its predecessor is malformed, outside the window, or had no
                // canonical form: nothing here to check it against.
                None => unverifiable += 1,
            },
            Some(_) => breaks.push(line_no),
        }
        prev = chain_hash(&obj);
        match record_of(line_no, &obj) {
            Ok(r) => records.push(r),
            Err(reason) => skip(line_no, reason),
        }
    }

    let total = records.len();
    let accepted = records.iter().filter(|r| r.outcome == "accepted").count();
    let refused = records.iter().filter(|r| r.outcome == "refused").count();
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    for r in &records {
        *by_kind.entry(r.kind.clone()).or_default() += 1;
    }
    let chain = if let Some(first) = breaks.first() {
        ChainVerdict::Broken {
            first_line: *first,
            breaks: breaks.len(),
        }
    } else if events <= 1 {
        ChainVerdict::NothingBound
    } else {
        ChainVerdict::Verifies {
            chained,
            heads,
            unverifiable,
        }
    };
    records.reverse();
    records.truncate(MAX_RECORDS);
    NotificationsPanel::Read {
        path,
        records,
        total,
        accepted,
        refused,
        by_kind,
        chain,
        window_truncated,
        skipped,
        skipped_total,
    }
}

#[cfg(test)]
mod tests;
