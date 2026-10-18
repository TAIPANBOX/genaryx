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

use super::env::RevokeConfig;
use crate::money::state::{BusHandle, MoneyState};
use serde::Serialize;

/// The bound `delegation_revoke` puts on a reason.
pub const MAX_REASON_LEN: usize = 500;

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

/// The estate's agent id grammar. SCAFFOLD (red-first commit): today the
/// console checks only the scheme, as `delegation_revoke` does.
pub fn is_agent_id(s: &str) -> bool {
    s.starts_with("agent://")
}

fn validate(subject: String, reason: &str) -> Result<(String, String), StopError> {
    Ok((subject, reason.to_string()))
}

/// SCAFFOLD (red-first commit): today the console has no stop, so nothing is
/// banned, killed or recorded.
pub async fn agent_stop(
    subject: String,
    reason: String,
    _config: &RevokeConfig,
    _money: &MoneyState,
    _bus: Option<&BusHandle>,
) -> Result<StopOutcome, StopError> {
    let (subject, _reason) = validate(subject, &reason)?;
    Err(StopError::NotStopped {
        outcome: Box::new(StopOutcome {
            subject,
            stopped: false,
            ban_durable: None,
            open_runs: None,
            runs_killed: 0,
            warnings: Vec::new(),
            steps: Vec::new(),
        }),
    })
}

/// SCAFFOLD (red-first commit): today the console has no lift.
pub async fn agent_lift(
    subject: String,
    reason: String,
    _config: &RevokeConfig,
    _bus: Option<&BusHandle>,
) -> Result<LiftOutcome, StopError> {
    let (subject, _reason) = validate(subject, &reason)?;
    Err(StopError::NotLifted {
        outcome: Box::new(LiftOutcome {
            subject,
            lifted: false,
            lift_durable: None,
            warnings: Vec::new(),
            steps: Vec::new(),
        }),
    })
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
