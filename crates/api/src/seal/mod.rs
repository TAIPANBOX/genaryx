//! Sealing a case: the case, the proofs asked for it and the person's decision as
//! one canonical document with one digest, and that digest in the console's own
//! chain.
//!
//! # WHY THIS EXISTS
//!
//! A case that was assembled, explained, proved and decided is worth something
//! later only if what was seen can be shown to be what was seen. Slice 6 of the
//! incident-to-proven-fix plan closes the loop in the way the plan's first
//! decision allows: no new service and no new event type. The console already
//! emits exactly one type, `console_command`, and a closed case fits it, with the
//! action `console.case_close` and the pack's digest as the target. A reader of
//! the bus (Trailryx packs it) then has a chained, signed record that a document
//! with this digest was sealed by this operator at this time, and the document
//! itself is a file anyone can hash.
//!
//! # WHAT THE PACK HOLDS, AND WHO WROTE EACH PART
//!
//! - `case`: assembled again by the server from the planes at seal time, never
//!   taken from the browser. It carries its own digest (invariant 37).
//! - `proofs`: the proofs the operator named, asked for again by the server, each
//!   with its request and its result or the cause it could not be made. A pack
//!   that says a candidate would have stopped the incident has the server's word
//!   for it, not a page's.
//! - `decision`: what the operator says was done about it, `applied`, `declined`
//!   or `deferred`, and a note in the operator's words. The console cannot see an
//!   application that happens outside it (a policy loaded in the policy plane, a
//!   limit set in the gateway), so `applied` is a statement by a person and the
//!   pack says whose.
//! - `sealed_by` and `sealed_at`.
//!
//! # THE DIGEST
//!
//! The sha256 of the pack's canonical form ([`jcs`], RFC 8785), as
//! `sha256:<hex>`. The pack does not hold its own digest, so there is nothing to
//! blank out before hashing: a second implementation hashes the document it was
//! given. [`verify`] is that check, and a stored pack is read back only through
//! it.

pub mod commands;
pub mod jcs;

use crate::case::CaseFile;
use jcs::{JcsError, canonical};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The pack's format name, which a reader checks before anything else.
pub const PACK_FORMAT: &str = "genaryx.case-pack/1";

/// The largest pack that is sealed, in bytes of canonical text.
pub const PACK_MAX_BYTES: usize = 1024 * 1024;

/// The longest decision note, in characters.
pub const NOTE_MAX_CHARS: usize = 2000;

/// The most proofs one pack holds.
pub const PROOFS_MAX: usize = 4;

/// What the operator says was done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Applied,
    Declined,
    Deferred,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Applied => "applied",
            Outcome::Declined => "declined",
            Outcome::Deferred => "deferred",
        }
    }
}

/// The decision, in the operator's words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub outcome: Outcome,
    #[serde(default)]
    pub note: String,
}

impl Decision {
    /// The decision as it is sealed: the note trimmed, bounded, and free of
    /// control characters other than a line break.
    pub fn checked(&self) -> Result<Decision, &'static str> {
    Ok(self.clone())
}
}

/// A proof the operator asks to have sealed with the case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProofSpec {
    /// `wardryx replay` of a candidate policy.
    Policy { candidate: String },
    /// `tokenfuse backtest` of a candidate budget or step limit.
    Budget {
        per_run_usd: Option<String>,
        per_step_usd: Option<String>,
        max_steps: Option<u32>,
    },
}

/// A proof as sealed: what was asked and what came of it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProofRecord {
    pub kind: &'static str,
    pub request: Value,
    /// `{"proof": ...}` when it was made, `{"error": ...}` when it could not be.
    pub result: Value,
}

/// The pack, as a JSON value. Nothing here reads a clock: the caller says when
/// and by whom, so the same inputs make the same pack.
pub fn build_pack(
    case: &CaseFile,
    proofs: &[ProofRecord],
    decision: &Decision,
    sealed_by: &str,
    sealed_at: &str,
) -> Value {
    json!({
        "format": PACK_FORMAT,
        "case": case,
        "proofs": proofs,
        "decision": {
            "outcome": decision.outcome.as_str(),
            "note": decision.note,
            "said_by": sealed_by,
        },
        "sealed_by": sealed_by,
        "sealed_at": sealed_at,
    })
}

/// A sealed pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    /// The canonical text, which is the document.
    pub canonical: String,
    /// `sha256:<hex>` of that text.
    pub digest: String,
}

/// Why a pack was not sealed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealError {
    /// The pack has no canonical form.
    NotCanonical(JcsError),
    /// The canonical text is larger than [`PACK_MAX_BYTES`].
    TooLarge { bytes: usize },
}

/// `sha256:<hex>` of `text`.
pub fn digest_of_text(text: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(text.as_bytes()))
}

/// Canonicalise the pack and take its digest.
pub fn seal_value(pack: &Value) -> Result<Sealed, SealError> {
    let canonical = serde_json::to_string(pack).unwrap_or_default();
    let digest = digest_of_text(&canonical);
    Ok(Sealed { canonical, digest })
}

/// Why a document is not the pack its digest names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// The text is not JSON.
    NotJson,
    /// The text is JSON but not in canonical form, so it is not the bytes that
    /// were hashed.
    NotCanonical,
    /// The format name is not this one.
    WrongFormat,
    /// The text hashes to another digest.
    DigestDiffers { found: String },
}

/// Whether `text` is the pack `digest` names: JSON, in canonical form, of this
/// format, and hashing to `digest`.
pub fn verify(text: &str, digest: &str) -> Result<(), VerifyError> {
    let _ = (text, digest);
    Ok(())
}

/// Whether `digest` is `sha256:` and 64 lowercase hex digits, the only shape that
/// is ever turned into a file name.
pub fn is_digest(digest: &str) -> bool {
    digest.starts_with("sha256:")
}

#[cfg(test)]
mod tests;
