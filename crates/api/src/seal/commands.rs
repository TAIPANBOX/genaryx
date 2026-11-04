//! `case_seal` and `case_pack`: seal a case with the proofs and the decision, and
//! read a sealed pack back.
//!
//! Sealing writes in two places and says so: one `console.case_close` line in the
//! console's own chain, whose target is the pack's digest, and one file named by
//! that digest. It changes nothing in any plane (project rule 6 is about acts on
//! the estate; this records one a person says they did or did not do).
//!
//! The order is the point. The chain line is written first and the file second:
//! a digest in the chain with no file is a pack that can be handed over from the
//! answer and stored again, but a file with no line in the chain is a document
//! that claims to have been sealed and was not. So a seal that cannot be
//! journaled is refused and stores nothing.

use super::{
    Decision, PROOFS_MAX, ProofRecord, ProofSpec, SealError, Sealed, VerifyError, build_pack,
    is_digest, seal_value, verify,
};
use crate::case::commands::{CaseError, CasePlanes, case_file};
use crate::journal::record_console_action;
use crate::money::state::BusHandle;
use crate::proof::budget_commands::{BudgetRequest, budget_proof};
use crate::proof::commands::{ProofRequest, policy_proof};
use chrono::SecondsFormat;
use serde::Serialize;
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;

/// The action the sealed case is recorded under in the console's chain.
pub const CASE_CLOSE_ACTION: &str = "console.case_close";

/// The directory, under the console's store directory, that holds sealed packs.
pub const PACKS_DIR: &str = "case-packs";

/// A request to seal a case.
#[derive(Debug, Clone)]
pub struct SealRequest {
    /// The incident Incident 360 holds (`bus:<n>`, `verdryx:<n>` or `<n>`).
    pub incident_id: String,
    /// The proofs to ask for again and seal with the case. At most
    /// [`PROOFS_MAX`].
    pub proofs: Vec<ProofSpec>,
    pub decision: Decision,
}

/// Why nothing was sealed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SealFailure {
    /// The request is not one that is sealed.
    Refused { reason: &'static str },
    /// The case could not be started.
    Case { error: CaseError },
    /// The pack has no canonical form.
    NotCanonical,
    /// The pack is larger than the bound.
    TooLarge { bytes: usize },
    /// The chain line could not be written, so nothing was stored.
    NotJournaled { reason: String },
}

/// A sealed case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SealedCase {
    /// `sha256:<hex>`, the target of the chain line.
    pub digest: String,
    pub bytes: usize,
    /// Whether the file was written. The chain line was, either way.
    pub stored: bool,
    pub store_error: Option<String>,
    /// The canonical text of the pack, so it can be kept when it was not stored.
    pub pack: String,
}

fn to_value<T: Serialize>(t: &T) -> Value {
    serde_json::to_value(t).unwrap_or(Value::Null)
}

/// Ask for each named proof again, here, and keep what came of it.
async fn run_proofs(
    specs: &[ProofSpec],
    incident_id: &str,
    bus: &crate::bus::AppState,
) -> Vec<ProofRecord> {
    let mut records = Vec::new();
    for spec in specs {
        records.push(match spec {
            ProofSpec::Policy { candidate } => {
                let asked = ProofRequest {
                    incident_id: incident_id.to_string(),
                    candidate: candidate.clone(),
                };
                let result = match policy_proof(asked, bus).await {
                    Ok(proof) => json!({ "proof": to_value(&proof) }),
                    Err(error) => json!({ "error": to_value(&error) }),
                };
                ProofRecord {
                    kind: "policy",
                    request: json!({ "incident_id": incident_id, "candidate": candidate }),
                    result,
                }
            }
            ProofSpec::Budget {
                per_run_usd,
                per_step_usd,
                max_steps,
            } => {
                let asked = BudgetRequest {
                    incident_id: incident_id.to_string(),
                    per_run_usd: per_run_usd.clone(),
                    per_step_usd: per_step_usd.clone(),
                    max_steps: *max_steps,
                };
                let result = match budget_proof(asked, bus).await {
                    Ok(proof) => json!({ "proof": to_value(&proof) }),
                    Err(error) => json!({ "error": to_value(&error) }),
                };
                ProofRecord {
                    kind: "budget",
                    request: json!({
                        "incident_id": incident_id,
                        "per_run_usd": per_run_usd,
                        "per_step_usd": per_step_usd,
                        "max_steps": max_steps,
                    }),
                    result,
                }
            }
        });
    }
    records
}

/// The principal the pack is sealed by: the session's, else the console's own
/// operator placeholder, as the journal names it.
fn sealed_by() -> String {
    let org_domain = std::env::var("GENARYX_ORG_DOMAIN").unwrap_or_else(|_| "local".to_string());
    crate::console_actor::operator_or(&format!("user://{org_domain}/operator"))
}

/// The command: seal the case of an incident, on this box.
pub async fn case_seal(
    request: SealRequest,
    planes: CasePlanes<'_>,
    journal: Option<&BusHandle>,
) -> Result<SealedCase, SealFailure> {
    let sealed_at = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    case_seal_at(request, planes, journal, &sealed_at).await
}

/// [`case_seal`] at a time the caller names, so the same inputs make the same pack.
pub async fn case_seal_at(
    request: SealRequest,
    planes: CasePlanes<'_>,
    journal: Option<&BusHandle>,
    sealed_at: &str,
) -> Result<SealedCase, SealFailure> {
    let decision = request
        .decision
        .checked()
        .map_err(|reason| SealFailure::Refused { reason })?;
    if request.proofs.len() > PROOFS_MAX {
        return Err(SealFailure::Refused {
            reason: "a pack holds at most four proofs",
        });
    }
    let bus = planes.bus;
    let store_dir = bus.events_dir.clone();
    let case = case_file(request.incident_id.clone(), planes)
        .await
        .map_err(|error| SealFailure::Case { error })?;
    let records = run_proofs(&request.proofs, &request.incident_id, bus).await;
    let by = sealed_by();
    let pack = build_pack(&case, &records, &decision, &by, sealed_at);
    let sealed = seal_value(&pack).map_err(|e| match e {
        SealError::NotCanonical(_) => SealFailure::NotCanonical,
        SealError::TooLarge { bytes } => SealFailure::TooLarge { bytes },
    })?;

    let (journaled, why) = record_console_action(
        journal,
        CASE_CLOSE_ACTION,
        &sealed.digest,
        json!({
            "incident_id": request.incident_id,
            "case_digest": case.digest,
            "pack_bytes": sealed.canonical.len(),
            "proofs": records.len(),
            "outcome": decision.outcome.as_str(),
        }),
        format!("sealed:{}", decision.outcome.as_str()),
    );
    if !journaled {
        return Err(SealFailure::NotJournaled {
            reason: why.unwrap_or_default(),
        });
    }

    let (stored, store_error) = match store_dir {
        Some(dir) => match store_pack(&dir, &sealed) {
            Ok(()) => (true, None),
            Err(e) => (false, Some(e)),
        },
        None => (
            false,
            Some("the console has no store directory".to_string()),
        ),
    };
    Ok(SealedCase {
        digest: sealed.digest,
        bytes: sealed.canonical.len(),
        stored,
        store_error,
        pack: sealed.canonical,
    })
}

/// Write the pack to `<dir>/case-packs/<hex>.json`, private to this user, by a
/// temporary file renamed into place. A file already there must hold the same
/// text.
fn store_pack(dir: &Path, sealed: &Sealed) -> Result<(), String> {
    let hex = sealed
        .digest
        .strip_prefix("sha256:")
        .ok_or_else(|| "the digest is not a sha256".to_string())?;
    let packs = dir.join(PACKS_DIR);
    std::fs::create_dir_all(&packs).map_err(|e| e.to_string())?;
    let file = packs.join(format!("{hex}.json"));
    if file.exists() {
        let held = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
        return if held == sealed.canonical {
            Ok(())
        } else {
            Err("a different document is already stored under this digest".to_string())
        };
    }
    let temporary = packs.join(format!("{hex}.json.tmp-{}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut out = options.open(&temporary).map_err(|e| e.to_string())?;
    out.write_all(sealed.canonical.as_bytes())
        .and_then(|()| out.sync_all())
        .map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            e.to_string()
        })?;
    std::fs::rename(&temporary, &file).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        e.to_string()
    })
}

/// Why a stored pack was not handed back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PackError {
    /// The digest is not `sha256:` and 64 lowercase hex digits.
    BadDigest,
    /// The console has no store directory.
    NoStore,
    /// No pack is stored under this digest.
    NotFound,
    /// The file could not be read.
    Unreadable { reason: String },
    /// The file is not the pack its name says: it was altered, or it is not a
    /// pack.
    NotThePack { why: &'static str },
}

/// The command: read a sealed pack back, only if it is still the pack its digest
/// names.
pub fn case_pack(digest: &str, store_dir: Option<&Path>) -> Result<String, PackError> {
    if !is_digest(digest) {
        return Err(PackError::BadDigest);
    }
    let dir = store_dir.ok_or(PackError::NoStore)?;
    let hex = digest.strip_prefix("sha256:").unwrap_or_default();
    let file = dir.join(PACKS_DIR).join(format!("{hex}.json"));
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(PackError::NotFound),
        Err(e) => {
            return Err(PackError::Unreadable {
                reason: e.to_string(),
            });
        }
    };
    verify(&text, digest).map_err(|e| PackError::NotThePack {
        why: match e {
            VerifyError::NotJson => "the file is not JSON",
            VerifyError::NotCanonical => "the file is not in canonical form",
            VerifyError::WrongFormat => "the file is not a case pack",
            VerifyError::DigestDiffers { .. } => "the file hashes to another digest",
        },
    })?;
    Ok(text)
}
