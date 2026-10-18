//! `agent_stop` and `agent_lift`: stopping one agent everywhere is one
//! console action, and letting it back is another.
//!
//! `@decided 2026-10-18`: an operator stops an agent with one action in the
//! console. The action bans the agent at its issuer, so every token it holds
//! dies at every door that polls the revocation list and no new one is issued
//! until the ban is lifted, then kills the agent's open runs, and records
//! every step. Lifting the ban is the same ceremony. A step that fails is
//! shown and recorded, and an action with a failed step is never reported as
//! a stop or a lift.
//!
//! # The steps, in order
//!
//! 1. **The ceremony.** Nothing here runs one: the router's per-action
//!    passkey gate (`crates/web/src/main.rs`'s `webauthn_gate`, the
//!    `ceremony` column of `roles.rs`) has already passed before this module
//!    is reached, and its signature rides in on
//!    [`crate::console_actor::signature_or`]. The first journal line says
//!    which it was: a verified assertion (`webauthn-es256` and the credential
//!    id) or the documented fallback for a caller with no passkey enrolled
//!    (`software-signed`), so the record never claims a ceremony that did
//!    not happen.
//! 2. **The ban.** `POST /v1/revoke` with `{subject, actor, reason,
//!    ban: true}` (vouchryx invariant 25). Only an answer carrying both
//!    `"revoked":true` and `"banned":true` is a ban: a vouchryx older than
//!    the ban decodes the body, ignores the field it does not know and
//!    answers `{"revoked":true}`, which revokes the tokens the agent holds
//!    and leaves it free to get new ones. That answer is recorded as
//!    `revoked_not_banned` and the action is not a stop.
//! 3. **The runs.** No route in TokenFuse kills runs by agent (Cloud and
//!    gateway both kill one run id at a time, `POST /v1/runs/{id}/kill`), so
//!    the console lists the Cloud's runs (`GET /v1/runs`), keeps the ones
//!    whose `agent_id` is the agent and that are not already killed, and
//!    kills each through the same signed Cloud call `money_kill_run` makes.
//!    A kill counts only when the Cloud answers with that run id. The kills
//!    are attempted even when the ban failed: stopping spend now is never
//!    worse, and the verdict still says the agent was not stopped.
//! 4. **The verdict.** One last journal line, `console.stop_agent`, says
//!    `stopped` or names what failed.
//!
//! Every line goes into the console's own chain (`console.ndjson`, invariant
//! 3) through the journal `delegation_revoke` writes to, under
//! `decision: break_glass` with the operator's reason.

use super::commands::{MAX_REASON_LEN, journal, post_capped};
use super::env::RevokeConfig;
use crate::money::state::{BusHandle, MoneyInner, MoneyState};
use genaryx_connectors::CloudClient;
use genaryx_core::CommandRecord;
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;

/// The signature algorithm the router's ceremony sets on a verified
/// assertion (`crates/web/src/main.rs`'s `webauthn_gate`).
pub const CEREMONY_ALG: &str = "webauthn-es256";

/// The most runs one stop kills. Each kill is one signed call to the Cloud;
/// an agent with more open runs than this is reported as not stopped, with
/// the count left, and the operator runs Stop again for the rest.
pub const MAX_RUNS_PER_STOP: usize = 500;

/// One step of a stop or a lift, as it was journaled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StopStep {
    /// `ceremony`, `ban`, `list_runs`, `kill_run`, `lift`, or `verdict`.
    pub step: String,
    /// The agent, or the run id for a `kill_run`.
    pub target: String,
    pub ok: bool,
    /// The status the service answered with, 0 when no call was made or no
    /// answer came.
    pub http_status: u16,
    pub verify_result: String,
    /// Whether this step's line reached the console's chain.
    pub journaled: bool,
    pub journal_error: Option<String>,
}

/// What a stop did.
#[derive(Debug, Clone, Serialize)]
pub struct StopOutcome {
    pub subject: String,
    /// True only when the ban was confirmed, the runs were listed, and every
    /// open run was killed.
    pub stopped: bool,
    /// `Some(false)` when vouchryx holds the ban without a store, so a
    /// restart of vouchryx forgets it. `None` when no ban was confirmed.
    pub ban_durable: Option<bool>,
    /// The agent's open runs, `None` when they could not be listed.
    pub open_runs: Option<usize>,
    pub runs_killed: usize,
    pub warnings: Vec<String>,
    pub steps: Vec<StopStep>,
}

/// What a lift did.
#[derive(Debug, Clone, Serialize)]
pub struct LiftOutcome {
    pub subject: String,
    pub lifted: bool,
    pub lift_durable: Option<bool>,
    pub warnings: Vec<String>,
    pub steps: Vec<StopStep>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StopError {
    /// Not an agent id (`agent://<domain>/<path>`, the estate's grammar).
    /// Refused before anything is called or journaled.
    InvalidSubject { subject: String },
    /// Empty, or longer than the bound `delegation_revoke` uses.
    InvalidReason { chars: usize },
    /// At least one step failed; every step, done or failed, is listed.
    NotStopped { outcome: Box<StopOutcome> },
    /// The lift failed; the ban stands as far as this console knows.
    NotLifted { outcome: Box<LiftOutcome> },
}

/// The estate's agent id grammar, `^agent://[a-z0-9.-]+/[a-z0-9._/-]+$`
/// (`crates/core/src/schemas/agent-event.v0.2.schema.json`).
pub fn is_agent_id(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("agent://") else {
        return false;
    };
    let Some((domain, path)) = rest.split_once('/') else {
        return false;
    };
    !domain.is_empty()
        && !path.is_empty()
        && domain
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        && path.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'/' | b'-')
        })
}

fn validate(subject: String, reason: &str) -> Result<(String, String), StopError> {
    if !is_agent_id(&subject) {
        return Err(StopError::InvalidSubject { subject });
    }
    let trimmed = reason.trim();
    let chars = trimmed.chars().count();
    if chars == 0 || chars > MAX_REASON_LEN {
        return Err(StopError::InvalidReason { chars });
    }
    Ok((subject, trimmed.to_string()))
}

fn org_domain() -> String {
    std::env::var("GENARYX_ORG_DOMAIN").unwrap_or_else(|_| "local".to_string())
}

/// Writes each step into the console's chain as it happens, and keeps the
/// list the operator is shown.
struct Steps<'a> {
    bus: Option<&'a BusHandle>,
    org_domain: String,
    actor: String,
    reason: String,
    sig_alg: String,
    sig_fpr: String,
    /// `console.stop_agent` or `console.lift_agent`.
    action: &'static str,
    list: Vec<StopStep>,
}

impl Steps<'_> {
    fn record(&mut self, step: &str, target: &str, ok: bool, http_status: u16, verify: String) {
        let action = if step == "verdict" {
            self.action.to_string()
        } else {
            format!("{}.{step}", self.action)
        };
        let rec = CommandRecord {
            operator: self.actor.clone(),
            env: self.org_domain.clone(),
            action,
            target: target.to_string(),
            params: json!({ "reason": self.reason, "step": step }),
            decision: "break_glass".to_string(),
            sig_alg: self.sig_alg.clone(),
            sig_fpr: self.sig_fpr.clone(),
            http_status,
            verify_result: verify.clone(),
        };
        let (journaled, journal_error) = journal(self.bus, &self.org_domain, &rec);
        self.list.push(StopStep {
            step: step.to_string(),
            target: target.to_string(),
            ok,
            http_status,
            verify_result: verify,
            journaled,
            journal_error,
        });
    }
}

fn steps_for<'a>(action: &'static str, bus: Option<&'a BusHandle>, reason: String) -> Steps<'a> {
    let org = org_domain();
    let actor = crate::console_actor::operator_or(&format!("user://{org}/operator"));
    let (sig_alg, sig_fpr) =
        crate::console_actor::signature_or("software-signed", "software-signed");
    Steps {
        bus,
        org_domain: org,
        actor,
        reason,
        sig_alg,
        sig_fpr,
        action,
        list: Vec::new(),
    }
}

/// The first step of both actions: what confirmed it. Without a verified
/// assertion the action still runs (the router's documented fallback for a
/// caller with no passkey enrolled, on a console that does not require one),
/// and the operator is told so.
fn record_ceremony(s: &mut Steps<'_>, subject: &str, warnings: &mut Vec<String>) {
    let verify = if s.sig_alg == CEREMONY_ALG {
        format!("confirmed:{CEREMONY_ALG}")
    } else {
        warnings.push(
            "no passkey confirmed this action: the caller has none enrolled, so it ran on \
             the session alone (GENARYX_WEB_REQUIRE_PASSKEY makes a passkey mandatory)"
                .to_string(),
        );
        "not_confirmed:no_passkey_enrolled".to_string()
    };
    s.record("ceremony", subject, true, 0, verify);
}

/// One answer from vouchryx, folded into a step.
struct Answer {
    ok: bool,
    http_status: u16,
    verify: String,
    durable: Option<bool>,
}

/// vouchryx's own refusals carry only `{"error":"<code>"}`.
fn refused(status: u16, body: &Value) -> Answer {
    let code = body
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let verify = if status == 503 {
        // vouchryx's 503 means it could not keep this durably; for a ban it
        // may hold it in memory until a restart, which is not a stop.
        format!("not_durable:{code}")
    } else {
        format!("refused:{code}")
    };
    Answer {
        ok: false,
        http_status: status,
        verify,
        durable: None,
    }
}

fn not_configured() -> Answer {
    Answer {
        ok: false,
        http_status: 0,
        verify:
            "not_configured: GENARYX_VOUCHRYX_URL and GENARYX_VOUCHRYX_REVOKE_KEY_FILE are unset"
                .to_string(),
        durable: None,
    }
}

async fn ban_at_vouchryx(
    config: &RevokeConfig,
    subject: &str,
    actor: &str,
    reason: &str,
) -> Answer {
    let RevokeConfig::Configured {
        vouchryx_url,
        revoke_key,
    } = config
    else {
        return not_configured();
    };
    let body = json!({
        "jti": "",
        "subject": subject,
        "actor": actor,
        "reason": reason,
        "ban": true,
    });
    let url = format!("{vouchryx_url}/v1/revoke");
    match post_capped(&url, revoke_key.as_str(), &body).await {
        Err(detail) => Answer {
            ok: false,
            http_status: 0,
            verify: format!("unreachable: {detail}"),
            durable: None,
        },
        Ok((200, answer)) => {
            let revoked = answer.get("revoked").and_then(Value::as_bool) == Some(true);
            let banned = answer.get("banned").and_then(Value::as_bool) == Some(true);
            match (revoked, banned) {
                (true, true) => {
                    let durable = answer.get("durable").and_then(Value::as_bool);
                    Answer {
                        ok: true,
                        http_status: 200,
                        verify: if durable == Some(true) {
                            "banned:durable".to_string()
                        } else {
                            "banned:not_durable".to_string()
                        },
                        durable: Some(durable == Some(true)),
                    }
                }
                (true, false) => Answer {
                    ok: false,
                    http_status: 200,
                    verify: "revoked_not_banned: vouchryx revoked the tokens the agent holds and \
                             did not confirm a ban, so new tokens can still be issued"
                        .to_string(),
                    durable: None,
                },
                _ => Answer {
                    ok: false,
                    http_status: 200,
                    verify: "200 without revoked:true confirmation".to_string(),
                    durable: None,
                },
            }
        }
        Ok((status, answer)) => refused(status, &answer),
    }
}

async fn lift_at_vouchryx(
    config: &RevokeConfig,
    subject: &str,
    actor: &str,
    reason: &str,
) -> Answer {
    let RevokeConfig::Configured {
        vouchryx_url,
        revoke_key,
    } = config
    else {
        return not_configured();
    };
    let body = json!({ "subject": subject, "actor": actor, "reason": reason });
    let url = format!("{vouchryx_url}/v1/revoke/lift");
    match post_capped(&url, revoke_key.as_str(), &body).await {
        Err(detail) => Answer {
            ok: false,
            http_status: 0,
            verify: format!("unreachable: {detail}"),
            durable: None,
        },
        Ok((200, answer)) => {
            if answer.get("lifted").and_then(Value::as_bool) == Some(true) {
                let durable = answer.get("durable").and_then(Value::as_bool) == Some(true);
                Answer {
                    ok: true,
                    http_status: 200,
                    verify: if durable {
                        "lifted:durable".to_string()
                    } else {
                        "lifted:not_durable".to_string()
                    },
                    durable: Some(durable),
                }
            } else {
                Answer {
                    ok: false,
                    http_status: 200,
                    verify: "200 without lifted:true confirmation".to_string(),
                    durable: None,
                }
            }
        }
        Ok((status, answer)) => refused(status, &answer),
    }
}

/// The Cloud client of a Ready Money panel, or why there is none.
async fn cloud_client(money: &MoneyState) -> Result<Arc<CloudClient>, String> {
    match &*money.inner.lock().await {
        MoneyInner::Ready(c) => Ok(Arc::clone(&c.client)),
        MoneyInner::Bootstrapping => Err("the Cloud connection is still starting".to_string()),
        MoneyInner::NoEnvironment => Err("no TokenFuse Cloud is configured".to_string()),
        MoneyInner::PairingFailed { reason, .. } => {
            Err(format!("the Cloud connection failed: {reason}"))
        }
    }
}

/// Stop one agent: ban it at vouchryx, kill its open runs, record each step.
///
/// `bus` is the console's own journal (`crates/web/src/dispatch.rs`'s
/// `wg_journal`); `money` is the Money panel whose Cloud client lists and
/// kills runs. Returns `Ok` only for a complete stop; any failed step comes
/// back as [`StopError::NotStopped`] with every step listed.
pub async fn agent_stop(
    subject: String,
    reason: String,
    config: &RevokeConfig,
    money: &MoneyState,
    bus: Option<&BusHandle>,
) -> Result<StopOutcome, StopError> {
    let (subject, reason) = validate(subject, &reason)?;
    let mut s = steps_for("console.stop_agent", bus, reason);
    let mut warnings = Vec::new();

    // (a) the ceremony the router already ran.
    record_ceremony(&mut s, &subject, &mut warnings);

    // (b) the ban.
    let actor = s.actor.clone();
    let reason = s.reason.clone();
    let ban = ban_at_vouchryx(config, &subject, &actor, &reason).await;
    s.record("ban", &subject, ban.ok, ban.http_status, ban.verify.clone());
    if ban.durable == Some(false) {
        warnings.push(
            "the ban is not durable: vouchryx keeps no revocation store, so restarting it \
             lifts the ban"
                .to_string(),
        );
    }

    // (c) the agent's open runs.
    let mut open_runs = None;
    let mut runs_killed = 0usize;
    let mut kills_failed = 0usize;
    let mut not_attempted = 0usize;
    match cloud_client(money).await {
        Err(why) => s.record("list_runs", &subject, false, 0, format!("error: {why}")),
        Ok(client) => match client.runs().await {
            Err(e) => s.record(
                "list_runs",
                &subject,
                false,
                crate::money::commands::status_of(&e),
                format!("error: {e}"),
            ),
            Ok(runs) => {
                let mut open: Vec<_> = runs
                    .into_iter()
                    .filter(|r| r.agent_id == subject && !r.killed)
                    .collect();
                // The newest first: the run most likely still spending.
                open.sort_by(|a, b| b.last_seen_millis.cmp(&a.last_seen_millis));
                open_runs = Some(open.len());
                s.record(
                    "list_runs",
                    &subject,
                    true,
                    200,
                    format!("listed:{} open", open.len()),
                );
                not_attempted = open.len().saturating_sub(MAX_RUNS_PER_STOP);
                for run in open.iter().take(MAX_RUNS_PER_STOP) {
                    let id = run.run_id.as_str();
                    match client.kill_run(id).await {
                        Ok(resp) if resp.killed == id => {
                            runs_killed += 1;
                            s.record("kill_run", id, true, 200, "killed:true".to_string());
                        }
                        Ok(resp) => {
                            kills_failed += 1;
                            s.record(
                                "kill_run",
                                id,
                                false,
                                200,
                                format!("unconfirmed: the Cloud answered killed={:?}", resp.killed),
                            );
                        }
                        Err(e) => {
                            kills_failed += 1;
                            s.record(
                                "kill_run",
                                id,
                                false,
                                crate::money::commands::status_of(&e),
                                format!("error: {e}"),
                            );
                        }
                    }
                }
            }
        },
    }

    // (d) the verdict.
    // Stopped only when the ban held and every open run the list named was
    // killed: counted from the kills that succeeded, so a kill skipped for
    // any reason can never read as done.
    let stopped = ban.ok && open_runs == Some(runs_killed);
    let verdict = if stopped {
        if ban.durable == Some(true) {
            "stopped".to_string()
        } else {
            "stopped:ban_not_durable".to_string()
        }
    } else {
        let mut why = Vec::new();
        if !ban.ok {
            why.push(format!("ban {}", ban.verify));
        }
        match open_runs {
            None => why.push("runs not listed".to_string()),
            Some(n) => {
                if kills_failed > 0 {
                    why.push(format!("{kills_failed} of {n} run kill(s) failed"));
                }
                if not_attempted > 0 {
                    why.push(format!(
                        "{not_attempted} run(s) over the limit of {MAX_RUNS_PER_STOP} not attempted"
                    ));
                }
            }
        }
        format!("not_stopped: {}", why.join("; "))
    };
    s.record("verdict", &subject, stopped, 0, verdict);

    let outcome = StopOutcome {
        subject,
        stopped,
        ban_durable: ban.durable,
        open_runs,
        runs_killed,
        warnings,
        steps: s.list,
    };
    if stopped {
        Ok(outcome)
    } else {
        Err(StopError::NotStopped {
            outcome: Box::new(outcome),
        })
    }
}

/// Lift an agent's ban at vouchryx, after the same ceremony, and record it.
/// The tokens the agent held when it was stopped stay revoked (vouchryx
/// invariant 25); nothing is restarted here.
pub async fn agent_lift(
    subject: String,
    reason: String,
    config: &RevokeConfig,
    bus: Option<&BusHandle>,
) -> Result<LiftOutcome, StopError> {
    let (subject, reason) = validate(subject, &reason)?;
    let mut s = steps_for("console.lift_agent", bus, reason);
    let mut warnings = Vec::new();

    record_ceremony(&mut s, &subject, &mut warnings);

    let actor = s.actor.clone();
    let reason = s.reason.clone();
    let lift = lift_at_vouchryx(config, &subject, &actor, &reason).await;
    s.record(
        "lift",
        &subject,
        lift.ok,
        lift.http_status,
        lift.verify.clone(),
    );
    if lift.durable == Some(false) {
        warnings.push("the lift is not durable: vouchryx keeps no revocation store".to_string());
    }

    let verdict = if lift.ok {
        "lifted".to_string()
    } else {
        format!("not_lifted: {}", lift.verify)
    };
    s.record("verdict", &subject, lift.ok, 0, verdict);

    let outcome = LiftOutcome {
        subject,
        lifted: lift.ok,
        lift_durable: lift.durable,
        warnings,
        steps: s.list,
    };
    if outcome.lifted {
        Ok(outcome)
    } else {
        Err(StopError::NotLifted {
            outcome: Box::new(outcome),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_id_follows_the_estate_grammar() {
        assert!(is_agent_id("agent://acme.example/bot/a"));
        assert!(is_agent_id("agent://local/genaryx/felyx"));
        assert!(is_agent_id("agent://a-b.c/x_y.z-1/2"));
        for bad in [
            "",
            "agent://",
            "agent://acme.example",
            "agent://acme.example/",
            "agent:///bot",
            "AGENT://acme.example/bot",
            "agent://Acme.example/bot",
            "agent://acme.example/Bot",
            "user://acme.example/alice",
            "agent://acme example/bot",
            "agent://acme.example/bot?x=1",
            "agent://acme.example/bot\0",
            "agent://acme_example/bot",
        ] {
            assert!(!is_agent_id(bad), "{bad:?} must not read as an agent id");
        }
    }

    /// A 200-seed sweep of hostile subjects and reasons: never panics, and an
    /// accepted pair always satisfies both rules.
    #[test]
    fn two_hundred_seeds_of_hostile_stop_arguments_never_half_validate() {
        fn splitmix64(state: &mut u64) -> u64 {
            *state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = *state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }
        // Written apart from `is_agent_id`, so the sweep does not check the
        // validator against itself.
        fn grammar(s: &str) -> bool {
            let Some(rest) = s.strip_prefix("agent://") else {
                return false;
            };
            let mut halves = rest.splitn(2, '/');
            let domain = halves.next().unwrap_or("");
            let path = halves.next().unwrap_or("");
            !domain.is_empty()
                && !path.is_empty()
                && domain
                    .chars()
                    .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '.' | '-'))
                && path
                    .chars()
                    .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '.' | '_' | '/' | '-'))
        }
        let alphabet: &[u8] = b"agent:/.-_ABCxyz019?#%\0\x7f";
        for seed in 0..200u64 {
            let mut st = seed ^ 0x5EED_0001;
            let len = (splitmix64(&mut st) % 40) as usize;
            let mut subject: String = (0..len)
                .map(|_| alphabet[(splitmix64(&mut st) as usize) % alphabet.len()] as char)
                .collect();
            if splitmix64(&mut st).is_multiple_of(3) {
                subject = format!("agent://acme.example/{subject}");
            }
            let reason_len = (splitmix64(&mut st) % 700) as usize;
            let reason = "r".repeat(reason_len);
            let got = std::panic::catch_unwind(|| validate(subject.clone(), &reason));
            let got = got.unwrap_or_else(|_| panic!("seed {seed} panicked on {subject:?}"));
            if let Ok((s, r)) = got {
                assert!(grammar(&s), "seed {seed}: accepted {s:?}");
                let n = r.chars().count();
                assert!(n > 0 && n <= MAX_REASON_LEN, "seed {seed}: reason of {n}");
            }
        }
    }
}
