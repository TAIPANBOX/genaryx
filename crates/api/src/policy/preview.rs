//! What a person deciding a held tool call is shown of its arguments
//! (CLAUDE.md invariant 48, wardryx invariant 51).
//!
//! wardryx builds a preview from the argument bytes the grant is bound to,
//! hides secrets and personal data by fixed rules, seals it to the console's
//! preview key, and records only its digest. This module fetches the
//! ciphertext, opens it with the key the operator gave the console
//! (`GENARYX_PREVIEW_KEY_FILE`), checks it is the preview the hold records,
//! and hands the browser the document. It keeps nothing: no journal line, no
//! store, no log carries the preview, and Felyx never reads it.
//!
//! It also works out what a decision says the person saw: the version 2
//! approver statement's `tcd` and `pvd`.

use super::commands::{PolicyError, ready_client};
use super::state::PolicyState;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use genaryx_connectors::{Approval, PREVIEW_SEALED};
use genaryx_signing::approver::{NONE, Shown};
use genaryx_signing::preview::{Binding, PreviewKey, is_digest};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long after a person opened a preview a decision may name it.
pub const SEEN_FOR: Duration = Duration::from_secs(15 * 60);

/// The most opens remembered at once; past it the oldest go first.
const SEEN_CAP: usize = 4096;

/// Which previews were opened, by whom, in this console process. A decision
/// names a preview as seen only if this person opened that preview of that hold
/// here within [`SEEN_FOR`]: the digest alone is in the approvals list for any
/// viewer to copy, so a browser's word that it showed it is not enough. Held in
/// memory; a restart forgets, and the person opens it again.
#[derive(Default)]
pub struct SeenPreviews {
    inner: Mutex<HashMap<(String, String), (String, Instant)>>,
}

impl SeenPreviews {
    /// Record that `operator` opened preview `pvd` of hold `approval_id`.
    pub fn record(&self, operator: &str, approval_id: &str, pvd: &str) {
        let Ok(mut map) = self.inner.lock() else {
            return;
        };
        let now = Instant::now();
        map.retain(|_, (_, at)| now.duration_since(*at) < SEEN_FOR);
        if map.len() >= SEEN_CAP
            && let Some(oldest) = map
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(k, _)| k.clone())
        {
            map.remove(&oldest);
        }
        map.insert(
            (operator.to_string(), approval_id.to_string()),
            (pvd.to_string(), now),
        );
    }

    /// Whether `operator` opened preview `pvd` of hold `approval_id` here
    /// within [`SEEN_FOR`].
    pub fn confirms(&self, operator: &str, approval_id: &str, pvd: &str) -> bool {
        let Ok(map) = self.inner.lock() else {
            return false;
        };
        map.get(&(operator.to_string(), approval_id.to_string()))
            .is_some_and(|(seen, at)| seen == pvd && at.elapsed() < SEEN_FOR)
    }
}

/// The console's preview key, read once at startup.
pub enum PreviewKeyState {
    /// No key file is configured: no preview can be opened.
    NotConfigured,
    /// The key file holds a usable preview key.
    Ready(PreviewKey),
    /// A key file is configured and could not be used; the reason names the
    /// file and what was wrong, never key material.
    Failed(String),
}

impl PreviewKeyState {
    /// Read the key file named by `path`, or report that none is configured.
    pub fn load(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return PreviewKeyState::NotConfigured;
        };
        let loaded = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|text| PreviewKey::parse(&text).map_err(|e| e.to_string()));
        match loaded {
            Ok(key) => PreviewKeyState::Ready(key),
            Err(reason) => {
                let reason = format!("{}: {reason}", path.display());
                eprintln!(
                    "genaryx: WARNING: the preview key file cannot be used, so no held call's arguments can be shown until it can: {reason}"
                );
                PreviewKeyState::Failed(reason)
            }
        }
    }
}

/// Generate a preview key pair, write the private key to `out` (which must not
/// exist) readable by its owner only, and return the recipient text.
pub fn generate_key_file(out: &Path, x25519: bool) -> Result<String, String> {
    use std::io::Write as _;
    let kem = if x25519 {
        genaryx_signing::preview::KEM_X25519
    } else {
        genaryx_signing::preview::KEM_XWING
    };
    let (private, recipient) = PreviewKey::generate(kem).map_err(|e| e.to_string())?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o400);
    }
    let mut file = options
        .open(out)
        .map_err(|e| format!("{}: {e}", out.display()))?;
    writeln!(file, "{private}").map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(recipient)
}

/// What the browser is handed for one hold: the preview's state, and for a
/// sealed one the opened document, its digest (what a grant names) and when
/// wardryx stops keeping it. Never journaled, never logged, never stored.
#[derive(Debug, Clone, Serialize)]
pub struct PreviewDto {
    pub approval_id: String,
    /// `sealed`, `truncated`, `no_arguments`, `off`, `unavailable`, or `none`
    /// for a hold from a wardryx without previews or with no call.
    pub state: String,
    pub pvd: Option<String>,
    pub ruleset: Option<String>,
    pub expires_at: Option<String>,
    /// The preview document: a tree of typed nodes wardryx built, never text
    /// or markup. Set only for a sealed preview that opened and matched.
    pub document: Option<serde_json::Value>,
}

/// The preview state a hold that records none reads as.
pub const STATE_NONE: &str = "none";

/// Fetch, open and check the preview of hold `id`.
pub async fn policy_approval_preview(
    id: String,
    state: &PolicyState,
    key: &PreviewKeyState,
    seen: &SeenPreviews,
) -> Result<PreviewDto, PolicyError> {
    let client = ready_client(&state).await?;
    let operator = crate::console_actor::operator_or(&client.operator);
    let approvals = client
        .client
        .list_approvals()
        .await
        .map_err(PolicyError::from)?;
    let hold = approvals
        .iter()
        .find(|a| a.approval_id == id)
        .ok_or_else(|| PolicyError::Wardryx {
            status: Some(404),
            message: "approval not found".to_string(),
        })?;
    let empty = |state: &str| PreviewDto {
        approval_id: id.clone(),
        state: state.to_string(),
        pvd: None,
        ruleset: None,
        expires_at: None,
        document: None,
    };
    let Some(recorded) = hold.preview() else {
        return Ok(empty(STATE_NONE));
    };
    if recorded.state != PREVIEW_SEALED {
        return Ok(empty(&recorded.state));
    }
    let preview_key = match key {
        PreviewKeyState::Ready(k) => k,
        PreviewKeyState::NotConfigured => {
            return Err(PolicyError::Preview {
                reason: "no preview key is configured (GENARYX_PREVIEW_KEY_FILE), so the console cannot open the arguments wardryx sealed for it".to_string(),
            });
        }
        PreviewKeyState::Failed(reason) => {
            return Err(PolicyError::Preview {
                reason: format!("the preview key file cannot be used: {reason}"),
            });
        }
    };
    let pvd = recorded.pvd.clone().unwrap_or_default();
    let tcd = hold.tool_call().and_then(|c| c.digest).unwrap_or_default();
    if !is_digest(&pvd) || !is_digest(&tcd) {
        return Err(PolicyError::Preview {
            reason: "the hold does not record a well-formed call or preview digest".to_string(),
        });
    }
    let fetched = client
        .client
        .approval_preview(&id)
        .await
        .map_err(PolicyError::from)?;
    // The ciphertext must be the one the hold records, for the call the hold
    // names. wardryx checks this too; a console that believed the endpoint
    // alone would show whatever it was handed.
    if fetched.pvd.as_deref() != Some(pvd.as_str())
        || fetched.tool_call_digest.as_deref() != Some(tcd.as_str())
    {
        return Err(PolicyError::Preview {
            reason: "the preview wardryx sent is not the one the hold records".to_string(),
        });
    }
    let sealed = fetched
        .sealed
        .as_deref()
        .and_then(|b| STANDARD.decode(b).ok())
        .ok_or_else(|| PolicyError::Preview {
            reason: "wardryx sent a sealed preview with no readable ciphertext".to_string(),
        })?;
    let binding = Binding {
        org: hold.org().unwrap_or_default().to_string(),
        tool_call_digest: tcd,
        preview_digest: pvd.clone(),
        ruleset: recorded.ruleset.clone().unwrap_or_default(),
    };
    let document = preview_key
        .open(&binding, &sealed)
        .map_err(|e| PolicyError::Preview {
            reason: e.to_string(),
        })?;
    let document: serde_json::Value =
        serde_json::from_slice(&document).map_err(|_| PolicyError::Preview {
            reason: "the opened preview is not a JSON document".to_string(),
        })?;
    // Opened and checked: this person may now name it in a decision.
    seen.record(&operator, &id, &pvd);
    Ok(PreviewDto {
        approval_id: id,
        state: PREVIEW_SEALED.to_string(),
        pvd: Some(pvd),
        ruleset: recorded.ruleset,
        expires_at: fetched.expires_at,
        document: Some(document),
    })
}

/// What a decision on `hold` says the person saw, for a version 2 statement,
/// or `None` for a hold from a wardryx without previews, which reads only a
/// version 1 statement.
///
/// `seen` is the digest of the preview the browser showed, sent with the
/// decision and confirmed by the passkey assertion over the command's
/// arguments. A grant of a hold whose preview was sealed must name that
/// preview; a deny may name none (it opens nothing); a hold with no sealed
/// preview names none, and its grant is refused when a policy requires a
/// preview, unless the call had no arguments to show.
///
/// `opened` says whether this person opened that preview here
/// ([`SeenPreviews::confirms`]); a digest the browser sends without an open
/// behind it is refused.
pub fn shown_for(
    hold: &Approval,
    decision: &str,
    seen: Option<&str>,
    opened: bool,
) -> Result<Option<Shown>, String> {
    let Some(recorded) = hold.preview() else {
        return match seen {
            None => Ok(None),
            Some(_) => Err("this hold records no preview, so none can have been shown".to_string()),
        };
    };
    let tcd = hold
        .tool_call()
        .and_then(|c| c.digest)
        .unwrap_or_else(|| NONE.to_string());
    let sealed = recorded.state == PREVIEW_SEALED;
    let pvd = match (sealed, seen) {
        (true, Some(s)) if Some(s) == recorded.pvd.as_deref() && opened => s.to_string(),
        (true, Some(s)) if Some(s) == recorded.pvd.as_deref() => {
            return Err("this console has no record of the person deciding opening this preview; open the arguments again".to_string());
        }
        (true, Some(_)) => {
            return Err(
                "the preview shown is not the one the hold records; open the arguments again"
                    .to_string(),
            );
        }
        (true, None) if decision == "deny" => NONE.to_string(),
        (true, None) => {
            return Err("open the call's arguments before granting it".to_string());
        }
        (false, Some(_)) => {
            return Err("this hold has no preview, so none can have been shown".to_string());
        }
        (false, None) => NONE.to_string(),
    };
    if decision == "grant"
        && hold.require_preview()
        && pvd == NONE
        && recorded.state != "no_arguments"
    {
        return Err("a policy requires that a person saw this call's arguments before a grant, and they could not be shown".to_string());
    }
    Ok(Some(Shown { tcd, pvd }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TCD: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const PVD: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    fn hold(context: serde_json::Value) -> Approval {
        serde_json::from_value(serde_json::json!({
            "approval_id": "ap_1", "agent_id": "agent://acme/a", "run_id": "r",
            "requested_at": "2026-11-12T00:00:00Z", "pending": true, "context": context,
        }))
        .expect("a hold")
    }

    fn sealed(require: bool) -> Approval {
        hold(serde_json::json!({
            "tool_call": {"name": "t", "target": "s", "digest": TCD},
            "preview": {"state": "sealed", "pvd": PVD, "ruleset": "r1"},
            "require_preview": require,
        }))
    }

    #[test]
    fn a_grant_of_a_sealed_hold_names_the_preview_the_person_saw() {
        let shown = shown_for(&sealed(false), "grant", Some(PVD), true).expect("allowed");
        assert_eq!(
            shown,
            Some(Shown {
                tcd: TCD.into(),
                pvd: PVD.into()
            })
        );
    }

    #[test]
    fn a_grant_of_a_sealed_hold_nobody_opened_or_opened_stale_is_refused() {
        assert!(shown_for(&sealed(false), "grant", None, true).is_err());
        assert!(shown_for(&sealed(false), "grant", Some(&"3".repeat(64)), true).is_err());
    }

    #[test]
    fn a_digest_the_browser_sends_without_an_open_behind_it_is_refused() {
        assert!(shown_for(&sealed(false), "grant", Some(PVD), false).is_err());
    }

    #[test]
    fn an_open_is_remembered_for_that_person_and_that_hold_only() {
        let seen = SeenPreviews::default();
        seen.record("user://acme/alice", "ap_1", PVD);
        assert!(seen.confirms("user://acme/alice", "ap_1", PVD));
        assert!(!seen.confirms("user://acme/bob", "ap_1", PVD));
        assert!(!seen.confirms("user://acme/alice", "ap_2", PVD));
        assert!(!seen.confirms("user://acme/alice", "ap_1", &"3".repeat(64)));
    }

    #[test]
    fn a_deny_of_a_sealed_hold_may_name_no_preview() {
        let shown = shown_for(&sealed(false), "deny", None, true).expect("allowed");
        assert_eq!(
            shown,
            Some(Shown {
                tcd: TCD.into(),
                pvd: NONE.into()
            })
        );
    }

    #[test]
    fn a_hold_with_no_sealed_preview_is_decided_blind_and_named_so() {
        let truncated = hold(serde_json::json!({
            "tool_call": {"name": "t", "digest": TCD, "arguments_truncated": true},
            "preview": {"state": "truncated"},
        }));
        assert_eq!(
            shown_for(&truncated, "grant", None, true).expect("allowed"),
            Some(Shown {
                tcd: TCD.into(),
                pvd: NONE.into()
            })
        );
        assert!(shown_for(&truncated, "grant", Some(PVD), true).is_err());
    }

    #[test]
    fn require_preview_refuses_a_blind_grant_and_not_a_call_with_nothing_to_show() {
        let off = hold(serde_json::json!({
            "tool_call": {"name": "t", "digest": TCD},
            "preview": {"state": "off"}, "require_preview": true,
        }));
        assert!(shown_for(&off, "grant", None, true).is_err());
        assert!(shown_for(&off, "deny", None, true).is_ok());
        let nothing = hold(serde_json::json!({
            "tool_call": {"name": "t", "digest": TCD},
            "preview": {"state": "no_arguments"}, "require_preview": true,
        }));
        assert!(shown_for(&nothing, "grant", None, true).is_ok());
        assert!(shown_for(&sealed(true), "grant", Some(PVD), true).is_ok());
    }

    #[test]
    fn a_hold_from_a_wardryx_without_previews_gets_the_version_1_statement() {
        let old = hold(serde_json::json!({"tool_call": {"name": "t", "digest": TCD}}));
        assert_eq!(shown_for(&old, "grant", None, true), Ok(None));
        assert!(shown_for(&old, "grant", Some(PVD), true).is_err());
    }

    #[test]
    fn a_hold_with_no_call_names_none_for_both() {
        let model = hold(serde_json::json!({"preview": {"state": "no_arguments"}}));
        assert_eq!(
            shown_for(&model, "grant", None, true).expect("allowed"),
            Some(Shown {
                tcd: NONE.into(),
                pvd: NONE.into()
            })
        );
    }
}
