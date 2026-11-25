//! The Record panel: what Trailryx sealed, whether the current root carries a
//! Keyryx signature, and, on request, what `trailryx-verify` says about a pack
//! of it (CLAUDE.md invariant 54).
//!
//! # The last seal, from the node's own files and decoder
//!
//! A Trailryx node seals a segment by writing its manifest, `s0-NNNNNN.mf`, in
//! its data directory; the store root it signs is the tree over every
//! manifest of the shard (trailryx-node `signing.rs`, `current_root`). This
//! module reads those files where [`DATA_ENV`] names the directory and decodes
//! them with Trailryx's own `decode_manifest`, `ShardTree` and `StoreTree`, so
//! the root shown is the root the node computes. No journal is opened: the
//! cost is one small file per segment. A manifest that is a link, a
//! directory, too large, undecodable, or whose name and contents disagree
//! makes the store unreadable, never smaller.
//!
//! The root's signature is the node's `signing/signed/<root>` file
//! (`generated_at`, `algorithm=es384`, `public_key`, `signature`); a root
//! still waiting is in `signing/pending/<root>`. Both are read with exact
//! field and length checks. A signature on file is shown as on file: the node
//! checked it before storing it, and Verify is what proves it.
//!
//! # Verify, only when asked
//!
//! [`record_verify`] runs `trailryx-node read --data DIR --pack <scratch>` and
//! then `trailryx-verify <scratch>`, through the proofs' process machinery
//! (no shell, `PATH` alone, one deadline, capped output, at most two at once
//! under the proof slots). Verified is exit 0 with a last line of exactly
//! `VERIFIED`; not verified is exit 1 with `NOT VERIFIED`; anything else is
//! "could not be checked", never a verdict. The last result is kept in this
//! process with the root it was taken for and when.

use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use trailryx_index::{ShardTree, StoreTree};
use trailryx_record::{SegmentId, ShardIx};
use trailryx_store::evidence::decode_manifest;

use crate::proof::commands::{PROOF_SLOTS, Scratch, run_bounded};
use crate::proof::slots::Slots;

/// The setting that names the Trailryx node's data directory.
pub const DATA_ENV: &str = "GENARYX_TRAILRYX_DATA_DIR";
/// Trailryx's own setting for the witness attestations folder, passed on to
/// the pack step only.
pub const WITNESS_ENV: &str = "TRAILRYX_WITNESS_ATTESTATIONS";
/// The largest manifest file read.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// The most manifests read.
pub const MAX_MANIFESTS: usize = 100_000;
/// The largest signing file read.
pub const MAX_SIGNING_BYTES: u64 = 4 * 1024;
/// One deadline for both steps of a verification.
pub const VERIFY_TIMEOUT: Duration = Duration::from_secs(60);
/// The largest pack verified.
pub const MAX_PACK_BYTES: u64 = 256 * 1024 * 1024;
/// The most findings lines passed on, and their length.
pub const MAX_FINDINGS: usize = 200;
pub const MAX_FINDING_CHARS: usize = 300;

const SHARD: ShardIx = ShardIx(0);

/// The data directory named by `raw`: blank is none; otherwise an absolute
/// path with no `..`.
pub fn data_dir_from(raw: &str) -> Result<Option<PathBuf>, String> {
    let t = raw.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let p = PathBuf::from(t);
    if !p.is_absolute() {
        return Err("must be an absolute path".to_string());
    }
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("must not contain ..".to_string());
    }
    Ok(Some(p))
}

/// Where the panel reads and what it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordConfig {
    pub data_dir: Result<Option<PathBuf>, String>,
    pub witness_dir: Option<OsString>,
    pub node_bin: PathBuf,
    pub verify_bin: PathBuf,
}

impl RecordConfig {
    pub fn from_env() -> Self {
        Self {
            data_dir: data_dir_from(&std::env::var(DATA_ENV).unwrap_or_default()),
            witness_dir: std::env::var_os(WITNESS_ENV).filter(|v| !v.is_empty()),
            node_bin: PathBuf::from("trailryx-node"),
            verify_bin: PathBuf::from("trailryx-verify"),
        }
    }
}

/// The last sealed segment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LastSegment {
    pub segment: u64,
    pub records: u64,
    /// Unix seconds.
    pub first_recorded_at: u64,
    pub last_recorded_at: u64,
}

/// What the node's signing directory holds for the current root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Signature {
    /// A Keyryx signature is on file. `key` is the first 16 hex of the
    /// SHA-256 of the public key; `at` is unix seconds.
    Signed {
        key: String,
        algorithm: String,
        at: u64,
    },
    /// The root is queued for a signature since `since` (unix seconds).
    Queued { since: u64 },
    /// Neither file is there.
    None,
    /// A file is there and is not one this console reads.
    Unreadable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SealState {
    NotConfigured {
        variable: String,
    },
    Refused {
        variable: String,
        reason: String,
    },
    /// The directory or a file in it could not be read as the node wrote it.
    Unreadable {
        path: String,
        reason: String,
    },
    /// The directory holds no manifest: nothing is sealed yet.
    NothingSealed {
        path: String,
    },
    Sealed {
        path: String,
        segments: usize,
        records: u64,
        last: LastSegment,
        /// The store root, 96 hex.
        root: String,
        signature: Signature,
        /// Other shards' manifests seen and not read.
        other_shards: usize,
    },
}

/// The last verification this process made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VerifyState {
    /// Not verified since this console started.
    NotRun,
    Verified {
        root: String,
        at: u64,
        millis: u64,
        findings: Vec<String>,
    },
    NotVerified {
        root: String,
        at: u64,
        millis: u64,
        findings: Vec<String>,
    },
    /// trailryx-node refused to pack this store (it exits 2 when a journal
    /// does not reproduce its manifest, among other refusals): a red state,
    /// with the node's own words.
    Refused {
        root: String,
        at: u64,
        reason: String,
    },
    /// The check could not be made; this is not a verdict.
    NotChecked { at: u64, reason: String },
}

/// The panel's answer: the seal, and the last verification with whether it
/// was taken for the root shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordPanel {
    pub seal: SealState,
    pub verify: VerifyState,
    /// The last verdict was for another root than the one shown.
    pub verify_stale: bool,
}

fn bounded(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push_str("...");
        t
    }
}

fn unix_seconds(nanos: u64) -> u64 {
    nanos / 1_000_000_000
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2)
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// A small file read whole, only when it is a regular file of at most `max`
/// bytes. `Ok(None)` when it is not there.
fn read_small(path: &Path, max: u64) -> Result<Option<Vec<u8>>, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    if !meta.file_type().is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if meta.len() > max {
        return Err(format!("{} is larger than {max} bytes", path.display()));
    }
    // Judged again on the open file, and read no further than the cap, so a
    // file swapped or grown after the check above is still refused.
    use std::io::Read;
    let file =
        std::fs::File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let opened = file
        .metadata()
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if !opened.file_type().is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    let mut out = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if out.len() as u64 > max {
        return Err(format!("{} is larger than {max} bytes", path.display()));
    }
    Ok(Some(out))
}

/// `key=value` lines with exactly the keys in `want`, each once.
fn fields<'a>(text: &'a str, want: &[&str]) -> Result<Vec<(&'a str, &'a str)>, String> {
    let mut out: Vec<(&str, &str)> = Vec::new();
    // Every line, as the node reads it: an empty line is not key=value.
    for line in text.lines() {
        let (k, v) = line
            .split_once('=')
            .ok_or_else(|| "a line is not key=value".to_string())?;
        if !want.contains(&k) {
            return Err(format!("an unknown field {}", bounded(k, 32)));
        }
        if out.iter().any(|(seen, _)| *seen == k) {
            return Err(format!("the field {k} twice"));
        }
        out.push((k, v));
    }
    for k in want {
        if !out.iter().any(|(seen, _)| seen == k) {
            return Err(format!("no {k} field"));
        }
    }
    Ok(out)
}

fn field<'a>(fs: &[(&'a str, &'a str)], k: &str) -> &'a str {
    fs.iter()
        .find(|(n, _)| *n == k)
        .map(|(_, v)| *v)
        .unwrap_or_default()
}

/// The node's `signing/signed/<root>` file: four fields, P-384 lengths.
pub fn parse_signed(text: &str) -> Result<Signature, String> {
    let fs = fields(
        text,
        &["generated_at", "algorithm", "public_key", "signature"],
    )?;
    let at: u64 = field(&fs, "generated_at")
        .parse()
        .map_err(|_| "generated_at is not a number".to_string())?;
    if field(&fs, "algorithm") != "es384" {
        return Err("the algorithm is not es384".to_string());
    }
    let key = unhex(field(&fs, "public_key")).ok_or("public_key is not lowercase hex")?;
    if key.len() != 97 || key[0] != 0x04 {
        return Err("public_key is not an uncompressed P-384 point".to_string());
    }
    let sig = unhex(field(&fs, "signature")).ok_or("signature is not lowercase hex")?;
    if sig.len() != 96 {
        return Err("signature is not 96 bytes".to_string());
    }
    use sha2::{Digest, Sha256};
    Ok(Signature::Signed {
        key: hex(&Sha256::digest(&key))[..16].to_string(),
        algorithm: "es384".to_string(),
        at: unix_seconds(at),
    })
}

/// The node's `signing/pending/<root>` file: a tenant and when it was queued.
pub fn parse_pending(text: &str) -> Result<Signature, String> {
    let fs = fields(text, &["tenant", "generated_at"])?;
    let since: u64 = field(&fs, "generated_at")
        .parse()
        .map_err(|_| "generated_at is not a number".to_string())?;
    Ok(Signature::Queued {
        since: unix_seconds(since),
    })
}

fn signature_of(dir: &Path, root_hex: &str) -> Signature {
    let read = |sub: &str| {
        read_small(
            &dir.join("signing").join(sub).join(root_hex),
            MAX_SIGNING_BYTES,
        )
    };
    match read("signed") {
        Err(reason) => return Signature::Unreadable { reason },
        Ok(Some(bytes)) => {
            return match std::str::from_utf8(&bytes)
                .map_err(|_| "not UTF-8".to_string())
                .and_then(parse_signed)
            {
                Ok(s) => s,
                Err(reason) => Signature::Unreadable {
                    reason: format!(
                        "signing/signed/{}: {reason}",
                        &root_hex[..16.min(root_hex.len())]
                    ),
                },
            };
        }
        Ok(None) => {}
    }
    match read("pending") {
        Err(reason) => Signature::Unreadable { reason },
        Ok(Some(bytes)) => match std::str::from_utf8(&bytes)
            .map_err(|_| "not UTF-8".to_string())
            .and_then(parse_pending)
        {
            Ok(s) => s,
            Err(reason) => Signature::Unreadable {
                reason: format!(
                    "signing/pending/{}: {reason}",
                    &root_hex[..16.min(root_hex.len())]
                ),
            },
        },
        Ok(None) => Signature::None,
    }
}

/// The segment a manifest file name names for shard 0, the node's
/// `segment_of`: `s0-` then digits then `.mf`.
fn segment_of(name: &str) -> Option<u64> {
    let prefix = format!("{SHARD}-");
    // `parse::<u64>`, exactly as trailryx-node's `segment_of` reads it, so
    // the two list the same files.
    name.strip_prefix(&prefix)?
        .strip_suffix(".mf")?
        .parse()
        .ok()
}

fn is_other_shard_manifest(name: &str) -> bool {
    name.starts_with('s') && name.ends_with(".mf") && segment_of(name).is_none()
}

/// What one read of the store found.
struct StoreSummary {
    root: String,
    segments: usize,
    records: u64,
    last: LastSegment,
    other_shards: usize,
}

/// The root and the last segment of the store at `dir`, or the reason it
/// could not be read. `Ok(None)` when nothing is sealed.
fn read_store(dir: &Path) -> Result<Option<StoreSummary>, String> {
    let meta = std::fs::symlink_metadata(dir).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => {
            "no directory there: the path is not the one the node writes".to_string()
        }
        _ => format!("cannot read it: {e}"),
    })?;
    if !meta.file_type().is_dir() {
        return Err("not a directory (a link is not followed)".to_string());
    }
    let mut found = Vec::new();
    let mut other_shards = 0usize;
    let entries = std::fs::read_dir(dir).map_err(|e| format!("cannot list it: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot list it: {e}"))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(segment) = segment_of(name) else {
            if is_other_shard_manifest(name) {
                other_shards += 1;
            }
            continue;
        };
        if found.len() >= MAX_MANIFESTS {
            return Err(format!("more than {MAX_MANIFESTS} manifests"));
        }
        let bytes = read_small(&entry.path(), MAX_MANIFEST_BYTES)?
            .ok_or_else(|| format!("{name} went away while it was read"))?;
        let manifest = decode_manifest(&bytes)
            .ok_or_else(|| format!("{name} is not a manifest this console reads"))?;
        if manifest.segment != SegmentId(segment) || manifest.shard != SHARD {
            return Err(format!("{name} holds another segment's manifest"));
        }
        found.push((segment, manifest));
    }
    if found.is_empty() {
        return Ok(None);
    }
    found.sort_by_key(|(segment, _)| *segment);
    let records: u64 = found.iter().map(|(_, m)| m.records).sum();
    let (last_no, last) = found.last().expect("not empty");
    let last = LastSegment {
        segment: *last_no,
        records: last.records,
        first_recorded_at: unix_seconds(last.first_recorded_at.as_nanos()),
        last_recorded_at: unix_seconds(last.last_recorded_at.as_nanos()),
    };
    let segments = found.len();
    let mut tree = ShardTree::new(SHARD);
    for (_, manifest) in found {
        tree.push(manifest);
    }
    let root = StoreTree::from_shards(&[tree]).root().to_hex();
    Ok(Some(StoreSummary {
        root,
        segments,
        records,
        last,
        other_shards,
    }))
}

/// Read the seal at `cfg`'s data directory.
pub fn seal_state(cfg: &RecordConfig) -> SealState {
    let dir = match &cfg.data_dir {
        Err(reason) => {
            return SealState::Refused {
                variable: DATA_ENV.to_string(),
                reason: reason.clone(),
            };
        }
        Ok(None) => {
            return SealState::NotConfigured {
                variable: DATA_ENV.to_string(),
            };
        }
        Ok(Some(d)) => d,
    };
    let path = dir.display().to_string();
    match read_store(dir) {
        Err(reason) => SealState::Unreadable {
            path,
            reason: bounded(&reason, 512),
        },
        Ok(None) => SealState::NothingSealed { path },
        Ok(Some(StoreSummary {
            root,
            segments,
            records,
            last,
            other_shards,
        })) => {
            let signature = signature_of(dir, &root);
            SealState::Sealed {
                path,
                segments,
                records,
                last,
                root,
                signature,
                other_shards,
            }
        }
    }
}

static LAST_VERIFY: LazyLock<Mutex<VerifyState>> =
    LazyLock::new(|| Mutex::new(VerifyState::NotRun));

fn last_verify() -> VerifyState {
    LAST_VERIFY
        .lock()
        .map(|g| g.clone())
        .unwrap_or(VerifyState::NotRun)
}

fn verdict_at(v: &VerifyState) -> Option<u64> {
    match v {
        VerifyState::Verified { at, .. }
        | VerifyState::NotVerified { at, .. }
        | VerifyState::Refused { at, .. } => Some(*at),
        _ => None,
    }
}

/// Keep `result` as the last verdict when it is one and is not older than the
/// one kept. A check that could not be made replaces nothing.
pub fn keep_verdict(kept: &mut VerifyState, result: &VerifyState) {
    if let Some(at) = verdict_at(result) {
        if verdict_at(kept).is_none_or(|k| at >= k) {
            *kept = result.clone();
        }
    }
}

fn panel(seal: SealState, verify: VerifyState) -> RecordPanel {
    let shown = match &seal {
        SealState::Sealed { root, .. } => Some(root.clone()),
        _ => None,
    };
    let verified_root = match &verify {
        VerifyState::Verified { root, .. }
        | VerifyState::NotVerified { root, .. }
        | VerifyState::Refused { root, .. } => Some(root.clone()),
        _ => None,
    };
    // A verdict is current only for the root shown now: another root, or no
    // readable root at all, makes it stale.
    let verify_stale = verified_root.is_some() && shown != verified_root;
    RecordPanel {
        seal,
        verify,
        verify_stale,
    }
}

/// The command: the seal and the last verification.
pub fn record_status(cfg: &RecordConfig) -> RecordPanel {
    panel(seal_state(cfg), last_verify())
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// What `trailryx-verify` said, judged strictly: the verdict needs both the
/// exit code and the last non-empty line to agree.
pub fn judge_verify(exit: Option<i32>, stdout: &str) -> Result<bool, String> {
    let last = stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    match (exit, last) {
        (Some(0), "VERIFIED") => Ok(true),
        (Some(1), "NOT VERIFIED") => Ok(false),
        // trailryx-verify read the pack and found it broken (exit 1).
        (Some(1), line) if line.starts_with("BROKEN:") => Ok(false),
        (Some(2), _) => Err("trailryx-verify could not read the pack".to_string()),
        (code, line) => Err(format!(
            "trailryx-verify answered exit {} with a last line of {:?}, which is no verdict",
            code.map(|c| c.to_string())
                .unwrap_or_else(|| "none".to_string()),
            bounded(line, 80)
        )),
    }
}

/// The findings lines (`[level] name: text`), bounded, as data.
pub fn findings_of(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('['))
        .take(MAX_FINDINGS)
        .map(|l| bounded(l, MAX_FINDING_CHARS))
        .collect()
}

/// Make a pack and verify it, under `slots`, with `timeout` for both steps.
pub fn verify_in(slots: &Slots, cfg: &RecordConfig, timeout: Duration) -> VerifyState {
    let at = now_unix();
    let not_checked = |reason: String| VerifyState::NotChecked {
        at,
        reason: bounded(&reason, 512),
    };
    let dir = match &cfg.data_dir {
        Ok(Some(d)) => d.clone(),
        Ok(None) => return not_checked(format!("{DATA_ENV} is not set on this box")),
        Err(reason) => return not_checked(format!("{DATA_ENV} {reason}")),
    };
    let (root, segments, records) = match read_store(&dir) {
        Ok(Some(summary)) => (summary.root, summary.segments, summary.records),
        Ok(None) => return not_checked("nothing is sealed yet".to_string()),
        Err(reason) => return not_checked(reason),
    };
    let _slot = match slots.try_acquire() {
        Ok(slot) => slot,
        Err(busy) => {
            return not_checked(format!(
                "every one of the {} proof slots is in use; ask again when one has finished",
                busy.limit
            ));
        }
    };
    let scratch = match Scratch::new() {
        Ok(s) => s,
        Err(e) => return not_checked(format!("no scratch directory: {e}")),
    };
    let pack = scratch.0.join("record.trxevid");
    let started = Instant::now();
    let deadline = started + timeout;
    let mut extra: Vec<(&str, &OsStr)> = Vec::new();
    if let Some(w) = &cfg.witness_dir {
        extra.push((WITNESS_ENV, w.as_os_str()));
    }
    let pack_args = [
        OsString::from("read"),
        OsString::from("--data"),
        dir.clone().into_os_string(),
        OsString::from("--pack"),
        pack.clone().into_os_string(),
    ];
    let made = match run_bounded(
        &cfg.node_bin,
        &pack_args,
        &extra,
        deadline,
        timeout.as_secs(),
    ) {
        Ok(f) => f,
        Err(e) => return not_checked(format!("making the pack: {e:?}")),
    };
    if made.exit == Some(2) {
        return VerifyState::Refused {
            root,
            at,
            reason: bounded(made.stderr.trim(), 512),
        };
    }
    if made.exit != Some(0) {
        return not_checked(format!(
            "trailryx-node could not make the pack (exit {}): {}",
            made.exit
                .map(|c| c.to_string())
                .unwrap_or_else(|| "none".to_string()),
            bounded(made.stderr.trim(), 300)
        ));
    }
    // The pack is of the root read above only if the store did not move
    // while it was made: roots only grow, so the same store before and after
    // is that root. The node's own count must agree with this read too.
    match read_store(&dir) {
        Ok(Some(after))
            if after.root == root && after.segments == segments && after.records == records => {}
        Ok(_) => {
            return not_checked(
                "the record was sealed further while it was packed; ask again".to_string(),
            );
        }
        Err(reason) => {
            return not_checked(format!(
                "the store could not be read after packing: {reason}"
            ));
        }
    }
    let expected = format!("{segments} sealed segment(s), {records} record(s), in ");
    if !made
        .stdout
        .lines()
        .next()
        .unwrap_or("")
        .starts_with(&expected)
    {
        return not_checked(
            "trailryx-node packed a store that does not match what this console read".to_string(),
        );
    }
    match std::fs::symlink_metadata(&pack) {
        Ok(m) if m.file_type().is_file() && m.len() <= MAX_PACK_BYTES => {}
        Ok(m) if m.file_type().is_file() => {
            return not_checked(format!(
                "the pack is larger than {MAX_PACK_BYTES} bytes; use trailryx-verify on the box"
            ));
        }
        _ => return not_checked("trailryx-node wrote no pack".to_string()),
    }
    let checked = match run_bounded(
        &cfg.verify_bin,
        &[pack.into_os_string()],
        &[],
        deadline,
        timeout.as_secs(),
    ) {
        Ok(f) => f,
        Err(e) => return not_checked(format!("verifying the pack: {e:?}")),
    };
    let millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match judge_verify(checked.exit, &checked.stdout) {
        Ok(true) => VerifyState::Verified {
            root,
            at,
            millis,
            findings: findings_of(&checked.stdout),
        },
        Ok(false) => VerifyState::NotVerified {
            root,
            at,
            millis,
            findings: findings_of(&checked.stdout),
        },
        Err(reason) => not_checked(reason),
    }
}

/// The command: verify now, keep the result, and answer the panel.
pub async fn record_verify(cfg: RecordConfig) -> RecordPanel {
    let slots = PROOF_SLOTS.clone();
    let cfg2 = cfg.clone();
    let result = tokio::task::spawn_blocking(move || verify_in(&slots, &cfg2, VERIFY_TIMEOUT))
        .await
        .unwrap_or_else(|e| VerifyState::NotChecked {
            at: now_unix(),
            reason: bounded(&e.to_string(), 512),
        });
    if let Ok(mut g) = LAST_VERIFY.lock() {
        keep_verdict(&mut g, &result);
    }
    panel(seal_state(&cfg), result)
}

#[cfg(test)]
mod tests;
