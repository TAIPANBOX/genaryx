//! Ingest: the `EventSource` abstraction and its implementations. Every source
//! yields raw, not-yet-validated records; [`IngestService`] centralizes conform
//! validation (via [`crate::conform`]), normalizes valid lines to [`ConsoleEvent`]
//! with provenance, batch-writes them to the [`Store`], journals per-file read
//! offsets, and broadcasts the batch live to the shells.
//!
//! Spec anchors: 06 §2 `IngestService`, 07 §3 (per-service NDJSON files, poll
//! fallback 250ms, offset journal per file, resilience to rotation/truncation,
//! re-open each cycle). Phase-0 is poll-only: FSEvents/inotify (`notify`) would
//! lower latency further, but the Mockryx 200ms poll pattern already confirms
//! plain polling is sufficient here, so it is left for a later phase rather
//! than spending the new-dependency risk budget where it is not needed yet.
//! Other `EventSource` impls (`SshTail`, `CloudSse`, `ApiPoll`) follow later,
//! sharing this same conform/quarantine/broadcast path.

use crate::conform::Conformer;
use crate::error::{Error, Result};
use crate::event::{ConsoleEvent, Provenance};
use crate::store::Store;
use chrono::Utc;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::broadcast;

/// One not-yet-conformed record read from a source: the raw line text plus
/// enough provenance (file + the byte offset where the line starts) to
/// journal progress and to quarantine it if it fails conformance. Conform is
/// centralized in [`IngestService::poll_once`], not per-source, so every
/// source shares one validation, quarantine, and offset-advancement path.
#[derive(Debug, Clone)]
pub struct RawRecord {
    pub raw: String,
    pub file: Option<String>,
    pub offset: Option<u64>,
}

/// A push source of agent-event lines. Impls: `FileTail`, and later `SshTail`,
/// `CloudSse`, `ApiPoll`.
pub trait EventSource {
    /// Stable id for provenance and the UI (e.g. "filetail:tokenfuse").
    fn id(&self) -> &str;

    /// Poll for newly available raw records since the last call. Never blocks
    /// forever; returns an empty vec when there is nothing new.
    fn poll(&mut self) -> Result<Vec<RawRecord>>;
}

/// Tails one NDJSON file. Poll-only in Phase-0 (see the module docs); journals
/// per-file offsets via the caller ([`IngestService::poll_once`]); survives
/// rotation and truncation by re-opening on each cycle rather than holding the
/// handle open (07 §3, matching the `verdryx.events` pattern).
pub struct FileTail {
    id: String,
    path: PathBuf,
    offset: u64,
}

impl FileTail {
    /// A tail for `path`, resuming from `offset` (0 for a never-seen file; the
    /// store's journaled offset otherwise, see [`IngestService::add_file_source`]).
    pub fn new(id: impl Into<String>, path: impl AsRef<Path>, offset: u64) -> Self {
        Self {
            id: id.into(),
            path: path.as_ref().to_path_buf(),
            offset,
        }
    }
}

impl EventSource for FileTail {
    fn id(&self) -> &str {
        &self.id
    }

    fn poll(&mut self) -> Result<Vec<RawRecord>> {
        // Re-open every cycle instead of holding the handle across polls (07
        // §3): a rotated or permission-revoked file must not wedge the loop.
        let mut file = match File::open(&self.path) {
            Ok(f) => f,
            // The product that would write this file may not have started
            // yet; that is not an ingest failure, just nothing to read.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::Io(e)),
        };

        let len = file.metadata()?.len();

        // Rotation/truncation resilience (07 §3): a current length shorter
        // than our journaled offset means the file was replaced or truncated
        // out from under us. Reset to the top rather than seeking past EOF.
        if len < self.offset {
            self.offset = 0;
        }

        if len == self.offset {
            return Ok(Vec::new());
        }

        file.seek(SeekFrom::Start(self.offset))?;
        let mut buf = Vec::with_capacity((len - self.offset) as usize);
        file.read_to_end(&mut buf)?;

        let file_key = self.path.to_string_lossy().into_owned();
        let mut records = Vec::new();
        let mut start = 0usize;
        let mut cursor = self.offset;
        // Only complete lines (ending in `\n`) are emitted; a trailing
        // partial line is left in place for the next poll, once its newline
        // arrives.
        for (i, byte) in buf.iter().enumerate() {
            if *byte != b'\n' {
                continue;
            }
            let line_offset = cursor;
            cursor += (i - start + 1) as u64; // + the newline itself
            records.push(RawRecord {
                raw: String::from_utf8_lossy(&buf[start..i]).into_owned(),
                file: Some(file_key.clone()),
                offset: Some(line_offset),
            });
            start = i + 1;
        }
        self.offset = cursor;

        Ok(records)
    }
}

/// The inode of `path`, when the platform reports one.
///
/// `None` on a platform without inodes and on any file that cannot be stat-ed
/// right now (it may simply not exist yet). Both are "cannot tell", and every
/// caller treats that as a reason to keep the previous behaviour rather than to
/// act on a guess.
#[cfg(unix)]
fn inode_of(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| m.ino())
}

#[cfg(not(unix))]
fn inode_of(_path: &Path) -> Option<u64> {
    None
}

/// Outcome of one [`IngestService::poll_once`] cycle.
#[derive(Debug, Clone, Copy, Default)]
pub struct IngestStats {
    pub inserted: usize,
    pub quarantined: usize,
}

/// Broadcast capacity: generous enough that a shell subscriber briefly busy
/// with a redraw does not miss events under normal Phase-0 volumes. A
/// receiver that falls further behind than this gets `RecvError::Lagged` (a
/// countable gap), never a hang.
const BROADCAST_CAPACITY: usize = 1024;

/// Owns a [`Store`] and one compiled [`Conformer`]; turns registered
/// [`EventSource`]s into stored + broadcast [`ConsoleEvent`]s (06 §2).
///
/// The `Store` wraps a `rusqlite::Connection`, which is `Send` but not `Sync`:
/// rather than share it behind a lock, one `IngestService` owns it exclusively
/// and runs its poll loop on a single thread ([`IngestService::run_blocking`]).
pub struct IngestService {
    store: Store,
    conformer: Conformer,
    env: String,
    sources: Vec<Box<dyn EventSource + Send>>,
    sender: broadcast::Sender<ConsoleEvent>,
}

impl IngestService {
    /// Build a service around `store`: one `Conformer` compiled once, one
    /// broadcast channel shells can [`IngestService::subscribe`] to.
    pub fn new(store: Store, env: impl Into<String>) -> Result<Self> {
        let conformer = Conformer::new().map_err(Error::Conform)?;
        let (sender, _receiver) = broadcast::channel(BROADCAST_CAPACITY);
        Ok(Self {
            store,
            conformer,
            env: env.into(),
            sources: Vec::new(),
            sender,
        })
    }

    /// Read-only access to the underlying store, e.g. for `event_count`/
    /// `quarantine_count` after a poll.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Register a `FileTail` source for `path`, seeding its offset from the
    /// store's journal when this file has been ingested before (a never-seen
    /// file starts at 0).
    ///
    /// The journaled offset is only trusted when the file is still the SAME
    /// file. Against a scratch store this never mattered, because the journal
    /// died with the process; against a durable one, a file rotated away and
    /// replaced while the console was down would otherwise be resumed at an
    /// offset that belongs to a file that no longer exists. `FileTail` catches
    /// that only when the replacement is SHORTER than the old offset. When it
    /// is longer, the tail silently starts in the middle and every event before
    /// that point is lost with nothing reporting it.
    ///
    /// So: same inode, resume; different inode, start at the top and let the
    /// dedupe key sort out anything already stored. No inode recorded (an older
    /// store, or a platform that does not report one) means "cannot tell", and
    /// the answer there is to resume, which is exactly the behaviour that came
    /// before.
    pub fn add_file_source(&mut self, id: impl Into<String>, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let key = path.to_string_lossy().into_owned();
        let journaled = self.store.get_source_state(&key)?;
        let live_inode = inode_of(path);

        let offset = match (journaled, live_inode) {
            (None, _) => 0,
            (Some(state), Some(now)) => match state.inode {
                Some(then) if then != now => {
                    eprintln!(
                        "genaryx: {key} was replaced while this console was not reading it \
                         (inode {then} -> {now}); re-reading from the top, already-stored lines \
                         are skipped by their dedupe key"
                    );
                    0
                }
                _ => state.offset,
            },
            (Some(state), None) => state.offset,
        };

        self.sources.push(Box::new(FileTail::new(id, path, offset)));
        Ok(())
    }

    /// Subscribe to the live broadcast of newly ingested (conforming) events.
    /// Call this before the first [`IngestService::poll_once`] /
    /// [`IngestService::run_blocking`] tick to avoid missing its batch.
    pub fn subscribe(&self) -> broadcast::Receiver<ConsoleEvent> {
        self.sender.subscribe()
    }

    /// One poll cycle across every registered source: poll -> conform -> batch
    /// insert -> journal offsets -> broadcast. A malformed or non-conforming
    /// line is quarantined, never dropped and never a panic (06 §0.5).
    pub fn poll_once(&mut self) -> Result<IngestStats> {
        let mut stats = IngestStats::default();
        let mut valid_events: Vec<ConsoleEvent> = Vec::new();
        // The furthest offset actually processed (valid OR quarantined) per
        // file this cycle, journaled after the batch insert commits. Never
        // advanced past a partial line, since a source withholds those until
        // their newline arrives (09 §3 anti-footgun).
        let mut offsets_to_journal: HashMap<String, u64> = HashMap::new();

        for source in &mut self.sources {
            let connector = source.id().to_string();
            for record in source.poll()? {
                let received_ts = Utc::now().to_rfc3339();
                let raw_len = record.raw.len() as u64;

                if let (Some(file), Some(offset)) = (&record.file, record.offset) {
                    let next = offset + raw_len + 1; // + the newline consumed with it
                    offsets_to_journal
                        .entry(file.clone())
                        .and_modify(|o| *o = (*o).max(next))
                        .or_insert(next);
                }

                match self.conformer.parse_valid(&record.raw) {
                    Ok(event) => {
                        let schema_version = event.schema_version().ok_or_else(|| {
                            Error::Conform(format!(
                                "event passed conformance under an unrecognized schema: {:?}",
                                event.schema
                            ))
                        })?;
                        valid_events.push(ConsoleEvent {
                            event,
                            provenance: Provenance {
                                env: self.env.clone(),
                                connector: connector.clone(),
                                file: record.file.clone(),
                                offset: record.offset,
                                endpoint: None,
                                received_ts,
                            },
                            raw: record.raw,
                            schema_version,
                        });
                    }
                    Err(report) => {
                        let reason = if report.errors.is_empty() {
                            "conformance failed".to_string()
                        } else {
                            report.errors.join("; ")
                        };
                        self.store.quarantine(
                            &self.env,
                            record.file.as_deref(),
                            record.offset,
                            &record.raw,
                            &reason,
                            &received_ts,
                        )?;
                        stats.quarantined += 1;
                    }
                }
            }
        }

        // The store's own count, not the size of the batch: a durable store
        // skips lines it already holds, and reporting the batch size would
        // describe how much this cycle READ rather than how much it learned.
        // The two differ on every restart, and on every `stack-up` restart
        // that rewrites a file the console has already seen.
        stats.inserted = self.store.insert_batch(&valid_events)?;
        for (file, offset) in &offsets_to_journal {
            self.store
                .set_offset(file, *offset, inode_of(Path::new(file)))?;
        }
        for event in valid_events {
            // No subscribers is not a failure; ignore the send error.
            let _ = self.sender.send(event);
        }

        Ok(stats)
    }

    /// A plain blocking loop: poll, sleep `interval` (250ms is the Phase-0
    /// default the caller should pass), repeat until `stop` is set. Runs on
    /// one thread start to finish; `broadcast::Sender::send` is synchronous,
    /// so no async runtime is required here.
    pub fn run_blocking(mut self, interval: Duration, stop: Arc<AtomicBool>) -> Result<()> {
        while !stop.load(Ordering::Relaxed) {
            self.poll_once()?;
            std::thread::sleep(interval);
        }
        Ok(())
    }
}
