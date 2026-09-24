//! `delegation_revoke`: the console's own cut-off switch for a compromised
//! agent's (or user's) delegated authority, against vouchryx's
//! `POST /v1/revoke`.
//!
//! Same class of act as `money_kill_run` (break-glass override, `admin`
//! role, the per-action WebAuthn ceremony when a passkey is enrolled) and
//! `remote_operator_wg_revoke` (it takes access away mid-incident): this
//! module is deliberately shaped like `money::commands`'s mutation path
//! (`finish_mutation`/`journal`), not like `journal::record_console_action`
//! (used by the lifecycle blocks and the WireGuard peer commands), because
//! those journal only an already-succeeded action under a fixed `allow`/200,
//! and a revocation this console attempted can come back refused, not
//! durable, or unreachable - outcomes that must reach the operator and the
//! journal AS THEMSELVES, never folded into a success shape.
//!
//! # What vouchryx's own wire contract does and does not say
//!
//! Read read-only from `~/Development/vouchryx`'s `internal/api/api.go`
//! (`revokeBody`, `revokeHandler`) at `19b5211` (origin/main; the local
//! checkout's `main` was two commits behind and lacked this): a 200 answers
//! `{"revoked":true,"expires":<unix>}`; every refusal - 400, 401, 403, and
//! 503 for BOTH "the list is full" and "took it in memory but could not
//! persist it" - answers through the same `refuse()` helper, which sends
//! only `{"error":"<oauth-shaped code>"}` to the CALLER and puts the real
//! reason nowhere but its own log line and event stream. So the two 503
//! causes are indistinguishable on the wire: this module reports every 503
//! as [`DelegationError::NotDurable`], honestly, rather than claiming to
//! know which one happened. `durable` itself is emitted only into vouchryx's
//! OWN event stream, never into the HTTP answer this console reads, so a 200
//! here never carries a `durable` field to forward - see
//! [`RevokeOutcome::vouchryx_response`]'s doc for how that is handled.

use super::env::RevokeConfig;
use crate::money::state::BusHandle;
use genaryx_core::{CommandRecord, command};
use serde::Serialize;
use std::time::Duration;

/// A revocation names exactly one target.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RevokeTarget {
    Jti(String),
    Subject(String),
}

/// Bounded so a pasted essay does not become an unreadable journal row.
/// Vouchryx's own bearer-key check runs first regardless (nothing here is a
/// security boundary vouchryx does not already enforce); this is a courtesy
/// to whoever reads the journal afterwards.
const MAX_REASON_LEN: usize = 500;

/// How long this console waits for vouchryx to answer before calling it
/// unreachable. Short: an operator cutting off a compromised agent is
/// watching the console, not waiting on it.
const VOUCHRYX_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DelegationError {
    /// Neither `GENARYX_VOUCHRYX_URL` nor `GENARYX_VOUCHRYX_REVOKE_KEY_FILE`
    /// is set. Names both, so the operator does not have to go read the
    /// source to learn which pair of variables this needs.
    NotConfigured,
    /// The caller passed both `subject` and `jti`, or neither.
    ExactlyOneTarget,
    /// `subject` was given but is not `agent://...` or `user://...`.
    InvalidSubject { subject: String },
    /// `reason` is empty/whitespace-only, or longer than `MAX_REASON_LEN`
    /// characters.
    InvalidReason { chars: usize },
    /// vouchryx answered and declined: 400 (malformed body), 401 (the
    /// bearer key was not accepted), 403 (vouchryx has no revoke keys
    /// configured at all, `revocation_disabled`). `error` is vouchryx's own
    /// OAuth-shaped code, verbatim - vouchryx's own design (its CLAUDE.md
    /// invariant 5) never says more than that, on purpose, and this console
    /// does not invent detail vouchryx withheld.
    Refused { status: u16, error: String },
    /// vouchryx answered 503. It could not accept this durably right now -
    /// its revocation list is full, or it took the revocation in memory and
    /// then failed to persist it before answering - and this module cannot
    /// and does not claim which, because vouchryx's own `refuse()` sends the
    /// identical body either way (see this module's doc comment). Never
    /// reported as success: a restart could forget it either way.
    NotDurable,
    /// No HTTP answer at all: a connection failure, TLS failure, or no
    /// response inside [`VOUCHRYX_TIMEOUT`].
    Unreachable { detail: String },
}

/// The result of a revocation vouchryx accepted (200).
#[derive(Debug, Clone, Serialize)]
pub struct RevokeOutcome {
    pub http_status: u16,
    pub verify_result: String,
    /// vouchryx's own response body, forwarded whole rather than re-typed
    /// into named fields. vouchryx's success answer today is
    /// `{"revoked":true,"expires":<unix>}` and carries no `durable` flag
    /// (see this module's doc comment); passing the body through means a
    /// field vouchryx adds later reaches the operator without this console
    /// needing to be taught its name first.
    pub vouchryx_response: serde_json::Value,
    pub bus_recorded: bool,
    pub bus_error: Option<String>,
}

/// Validate `subject`/`jti`/`reason` with no I/O and no knowledge of
/// configuration - split out so the hostile-input sweep below exercises
/// exactly this, and so "argument validation skipped" (a named mutant) is a
/// single call this function's absence would visibly remove.
fn validate_args(
    subject: Option<String>,
    jti: Option<String>,
    reason: &str,
) -> Result<(RevokeTarget, String), DelegationError> {
    let target = match (subject, jti) {
        (Some(s), None) => {
            if !(s.starts_with("agent://") || s.starts_with("user://")) {
                return Err(DelegationError::InvalidSubject { subject: s });
            }
            RevokeTarget::Subject(s)
        }
        (None, Some(j)) => RevokeTarget::Jti(j),
        // Both or neither. `Some(_), Some(_)` and `None, None` land here
        // together: the brief's own words are "both or neither is refused",
        // one refusal for one rule.
        _ => return Err(DelegationError::ExactlyOneTarget),
    };
    let trimmed = reason.trim();
    let chars = trimmed.chars().count();
    if chars == 0 || chars > MAX_REASON_LEN {
        return Err(DelegationError::InvalidReason { chars });
    }
    Ok((target, trimmed.to_string()))
}

#[derive(Serialize)]
struct RevokeRequest<'a> {
    jti: &'a str,
    subject: &'a str,
    actor: &'a str,
    reason: &'a str,
}

/// Revoke a delegation: an operator's cut-off switch for a compromised
/// agent's (or user's) authority, exactly one of `subject` or `jti`.
///
/// `bus` is the same console bus handle `money_kill_run` and
/// `remote_operator_wg_revoke` journal into (`crates/web/src/dispatch.rs`'s
/// `wg_journal`); a `None` bus (live-wire seeding never completed) is
/// reported honestly rather than silently skipped, same as every other
/// journaled mutation in this crate.
pub async fn delegation_revoke(
    subject: Option<String>,
    jti: Option<String>,
    reason: String,
    config: &RevokeConfig,
    bus: Option<&BusHandle>,
) -> Result<RevokeOutcome, DelegationError> {
    let (target, reason) = validate_args(subject, jti, &reason)?;

    let (vouchryx_url, revoke_key) = match config {
        RevokeConfig::NotConfigured => return Err(DelegationError::NotConfigured),
        RevokeConfig::Configured {
            vouchryx_url,
            revoke_key,
        } => (vouchryx_url, revoke_key),
    };

    let actor = crate::console_actor::operator_or(&format!(
        "user://{}/operator",
        std::env::var("GENARYX_ORG_DOMAIN").unwrap_or_else(|_| "local".to_string())
    ));

    let (jti_field, subject_field) = match &target {
        RevokeTarget::Jti(j) => (j.as_str(), ""),
        RevokeTarget::Subject(s) => ("", s.as_str()),
    };
    let body = RevokeRequest {
        jti: jti_field,
        subject: subject_field,
        actor: &actor,
        reason: &reason,
    };
    let target_label = match &target {
        RevokeTarget::Jti(j) => format!("jti:{j}"),
        RevokeTarget::Subject(s) => s.clone(),
    };

    let outcome = call_vouchryx(vouchryx_url, revoke_key.as_str(), &body).await;

    let (http_status, verify_result, vouchryx_response, result_err) = match &outcome {
        Ok(resp) => (
            200u16,
            "revoked:true".to_string(),
            resp.clone(),
            None::<DelegationError>,
        ),
        Err(CallOutcome::Http { status, body }) => {
            let error_code = body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            if *status == 503 {
                (
                    *status,
                    "not_durable".to_string(),
                    body.clone(),
                    Some(DelegationError::NotDurable),
                )
            } else {
                (
                    *status,
                    format!("refused:{error_code}"),
                    body.clone(),
                    Some(DelegationError::Refused {
                        status: *status,
                        error: error_code,
                    }),
                )
            }
        }
        Err(CallOutcome::Unreachable(detail)) => (
            0u16,
            format!("unreachable: {detail}"),
            serde_json::Value::Null,
            Some(DelegationError::Unreachable {
                detail: detail.clone(),
            }),
        ),
    };

    let org_domain = std::env::var("GENARYX_ORG_DOMAIN").unwrap_or_else(|_| "local".to_string());
    let rec = CommandRecord {
        operator: actor.clone(),
        env: org_domain.clone(),
        action: "console.revoke_delegation".to_string(),
        target: target_label,
        params: serde_json::json!({ "reason": reason }),
        decision: "break_glass".to_string(),
        sig_alg: {
            let (alg, _) = crate::console_actor::signature_or("software-signed", "software-signed");
            alg
        },
        sig_fpr: {
            let (_, fpr) = crate::console_actor::signature_or("software-signed", "software-signed");
            fpr
        },
        http_status,
        verify_result: verify_result.clone(),
    };
    let (bus_recorded, bus_error) = journal(bus, &org_domain, &rec);

    match result_err {
        None => Ok(RevokeOutcome {
            http_status,
            verify_result,
            vouchryx_response,
            bus_recorded,
            bus_error,
        }),
        Some(err) => Err(err),
    }
}

/// What calling vouchryx produced, before it is folded into an outcome: an
/// HTTP answer (whatever its status - vouchryx always answers JSON), or no
/// answer at all.
enum CallOutcome {
    Http {
        status: u16,
        body: serde_json::Value,
    },
    Unreachable(String),
}

async fn call_vouchryx(
    vouchryx_url: &str,
    revoke_key: &str,
    body: &RevokeRequest<'_>,
) -> Result<serde_json::Value, CallOutcome> {
    let client = match reqwest::Client::builder()
        .timeout(VOUCHRYX_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(e) => return Err(CallOutcome::Unreachable(e.to_string())),
    };
    let url = format!("{vouchryx_url}/v1/revoke");
    let sent = client
        .post(&url)
        .bearer_auth(revoke_key)
        .json(body)
        .send()
        .await;
    let resp = match sent {
        Ok(r) => r,
        Err(e) => return Err(CallOutcome::Unreachable(e.to_string())),
    };
    let status = resp.status().as_u16();
    let parsed: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    if status == 200 {
        Ok(parsed)
    } else {
        Err(CallOutcome::Http {
            status,
            body: parsed,
        })
    }
}

/// Journal one `CommandRecord` (best-effort: a journal failure is reported,
/// never panics, and never blocks the caller from learning vouchryx's own
/// verdict). Mirrors `money::commands::journal` exactly, with the
/// org-domain/host defaults `journal::record_console_action` uses, since
/// this module has no long-lived plane client to carry them on.
fn journal(
    bus: Option<&BusHandle>,
    org_domain: &str,
    rec: &CommandRecord,
) -> (bool, Option<String>) {
    let failed = |reason: String| -> (bool, Option<String>) {
        eprintln!(
            "genaryx: {} on {} was NOT journaled: {reason}",
            rec.action, rec.target
        );
        (false, Some(reason))
    };
    let Some(bus) = bus else {
        return failed(
            "no live event bus available (startup seeding did not complete)".to_string(),
        );
    };
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "console".to_string());
    match genaryx_core::store::Store::open(&bus.store_db_path) {
        Ok(store) => {
            match command::record(&store, &bus.console_events_path, org_domain, &host, rec) {
                Ok(()) => (true, None),
                Err(e) => failed(e.to_string()),
            }
        }
        Err(e) => failed(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- validate_args -----------------------------------------------

    #[test]
    fn exactly_one_of_subject_or_jti_is_required() {
        assert_eq!(
            validate_args(None, None, "reason").unwrap_err(),
            DelegationError::ExactlyOneTarget
        );
        assert_eq!(
            validate_args(
                Some("agent://acme.example/bot/a".into()),
                Some("jti-1".into()),
                "reason"
            )
            .unwrap_err(),
            DelegationError::ExactlyOneTarget
        );
    }

    #[test]
    fn jti_alone_is_accepted_with_no_format_constraint() {
        let (target, reason) = validate_args(None, Some("anything-at-all".into()), "why").unwrap();
        assert_eq!(target, RevokeTarget::Jti("anything-at-all".into()));
        assert_eq!(reason, "why");
    }

    #[test]
    fn subject_must_be_agent_or_user_scheme() {
        assert_eq!(
            validate_args(Some("bot://acme/x".into()), None, "why").unwrap_err(),
            DelegationError::InvalidSubject {
                subject: "bot://acme/x".into()
            }
        );
        assert!(validate_args(Some("agent://acme.example/bot/a".into()), None, "why").is_ok());
        assert!(validate_args(Some("user://acme.example/alice".into()), None, "why").is_ok());
    }

    #[test]
    fn reason_must_be_non_empty_and_bounded() {
        assert_eq!(
            validate_args(None, Some("j".into()), "").unwrap_err(),
            DelegationError::InvalidReason { chars: 0 }
        );
        assert_eq!(
            validate_args(None, Some("j".into()), "   ").unwrap_err(),
            DelegationError::InvalidReason { chars: 0 }
        );
        let exactly_max = "x".repeat(MAX_REASON_LEN);
        assert!(validate_args(None, Some("j".into()), &exactly_max).is_ok());
        let over_max = "x".repeat(MAX_REASON_LEN + 1);
        assert_eq!(
            validate_args(None, Some("j".into()), &over_max).unwrap_err(),
            DelegationError::InvalidReason {
                chars: MAX_REASON_LEN + 1
            }
        );
    }

    #[test]
    fn reason_is_trimmed_before_being_measured_and_stored() {
        let (_, reason) = validate_args(None, Some("j".into()), "  a real reason  ").unwrap();
        assert_eq!(reason, "a real reason");
    }

    /// A 200-seed sweep of hostile argument combinations: never panics, and
    /// never accepts a shape [`delegation_revoke`] would call vouchryx with
    /// that this function itself would not call well-formed. A tiny
    /// deterministic splitmix64 rather than the `rand` crate: this is a
    /// test-only need in one file, and a seeded, reproducible sweep is the
    /// house pattern (durability-sweep.sh, 200 seeds) rather than three
    /// hand-picked values.
    #[test]
    fn a_200_seed_sweep_of_hostile_args_never_panics_and_never_half_validates() {
        fn splitmix64(state: &mut u64) -> u64 {
            *state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = *state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }

        let nasty_strings: [&str; 10] = [
            "",
            "   ",
            "agent://",
            "agent:/missing-slash",
            "user://acme.example/alice",
            "AGENT://uppercase-scheme",
            "agent://acme.example/bot/a\0with-a-nul",
            "агент://кирилиця/приклад",
            "../../etc/passwd",
            "x",
        ];

        for seed in 0..200u64 {
            let mut state = seed ^ 0xD1B54A32D192ED03;
            let pick = |state: &mut u64, n: usize| (splitmix64(state) as usize) % n;

            let has_subject = splitmix64(&mut state).is_multiple_of(2);
            let has_jti = splitmix64(&mut state).is_multiple_of(2);
            let subject = has_subject
                .then(|| nasty_strings[pick(&mut state, nasty_strings.len())].to_string());
            let jti =
                has_jti.then(|| nasty_strings[pick(&mut state, nasty_strings.len())].to_string());

            let reason_len = (splitmix64(&mut state) as usize) % 2000;
            let reason: String = std::iter::repeat_n('a', reason_len).collect();
            let reason = if splitmix64(&mut state).is_multiple_of(5) {
                String::new()
            } else {
                reason
            };

            // Never panics, whatever the combination.
            let result =
                std::panic::catch_unwind(|| validate_args(subject.clone(), jti.clone(), &reason));
            assert!(
                result.is_ok(),
                "seed {seed} panicked: subject={subject:?} jti={jti:?} reason_len={}",
                reason.len()
            );
            let outcome = result.unwrap();

            // Cross-check: an Ok outcome must actually satisfy every rule
            // this module claims to enforce, so a validator that "half
            // validates" (e.g. checks the target but forgets the reason
            // bound) is caught by the sweep rather than by a hand-picked
            // case.
            if let Ok((target, out_reason)) = outcome {
                match target {
                    RevokeTarget::Subject(s) => {
                        assert!(
                            s.starts_with("agent://") || s.starts_with("user://"),
                            "seed {seed}: an accepted subject must carry an accepted scheme: {s:?}"
                        );
                        assert!(jti.is_none() || jti == Some(String::new()));
                    }
                    RevokeTarget::Jti(_) => {}
                }
                let chars = out_reason.chars().count();
                assert!(
                    chars > 0 && chars <= MAX_REASON_LEN,
                    "seed {seed}: an accepted reason must be within bounds, got {chars} chars"
                );
            }
        }
    }

    // ---- journal --------------------------------------------------------

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "genaryx-delegation-journal-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn test_rec(http_status: u16, verify_result: &str) -> CommandRecord {
        CommandRecord {
            operator: "user://acme.example/operator".to_string(),
            env: "local".to_string(),
            action: "console.revoke_delegation".to_string(),
            target: "agent://acme.example/bot/a".to_string(),
            params: serde_json::json!({ "reason": "compromised" }),
            decision: "break_glass".to_string(),
            sig_alg: "software-signed".to_string(),
            sig_fpr: "software-signed".to_string(),
            http_status,
            verify_result: verify_result.to_string(),
        }
    }

    #[test]
    fn a_journaled_attempt_lands_on_the_bus_and_conforms_even_when_refused() {
        let dir = scratch("refused");
        let bus = BusHandle::from_dirs(&dir, &dir);
        let rec = test_rec(401, "refused:invalid_client");

        let (journaled, err) = journal(Some(&bus), "acme.example", &rec);
        assert!(journaled, "journal_error: {err:?}");

        let body = std::fs::read_to_string(&bus.console_events_path).expect("read events file");
        let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 1);

        let conformer = genaryx_core::Conformer::new().expect("embedded schemas compile");
        let report = conformer.check_line(lines[0]);
        assert!(report.valid, "{:?}\n{}", report.errors, lines[0]);

        let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(
            v.pointer("/data/http_status").and_then(|s| s.as_u64()),
            Some(401),
            "a refusal must journal its REAL status, never 200"
        );
        assert_eq!(
            v.pointer("/data/verify_result").and_then(|s| s.as_str()),
            Some("refused:invalid_client")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_bus_is_reported_rather_than_silently_skipped() {
        let rec = test_rec(200, "revoked:true");
        let (journaled, err) = journal(None, "acme.example", &rec);
        assert!(!journaled);
        assert_eq!(
            err.as_deref(),
            Some("no live event bus available (startup seeding did not complete)")
        );
    }
}
