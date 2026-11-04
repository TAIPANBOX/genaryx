//! `case_file`: gather what the planes hold about one incident and hand it to
//! [`super::assemble`].
//!
//! A read, like every command it calls. It changes nothing in any plane, writes
//! no journal line and signs nothing.
//!
//! # WHERE EACH SECTION COMES FROM
//!
//! | Section | Read |
//! |---|---|
//! | Event, RunTimeline, SimilarIncidents | the console's own store (`console.sqlite`) |
//! | Spend | the money plane's `money_runs`, the row for the event's run |
//! | Identity, BlastRadius | idryx's identity list and alert list, for the incident's agent |
//! | Policy, Decisions | wardryx's `/v1/status` and its approvals for the event's run |
//! | PriorRulings | engram's `recall`, for the event's type |
//!
//! # WHY THE STORE IS READ FIRST AND DROPPED BEFORE ANY AWAIT
//!
//! `Store` holds a SQLite connection, which is `Send` and not `Sync`. Holding
//! it across an `.await` would make this future unusable behind the web shell's
//! handler. Every store read is therefore done in one synchronous block that
//! returns owned rows, and the connection is gone before the first plane is
//! asked.
//!
//! # WHAT AN INCIDENT ID IS
//!
//! The id Incident 360 holds. An event row is `bus:<n>` or `verdryx:<n>` (or a
//! bare `<n>`), `<n>` being the store's row id. A money, identity or posture
//! incident has no event row to start from, and is refused by naming its
//! source, never answered with an empty case.

use super::{
    Answer, CaseFile, Head, Plane, Questions, Row, Section, assemble, label, questions_for, words,
};
use crate::bus::AppState;
use crate::identity::IdentityState;
use crate::memory::MemoryState;
use crate::money::MoneyState;
use crate::policy::PolicyState;
use genaryx_connectors::IdryxIdentity;
use genaryx_core::store::{Store, StoredEvent};
use serde::Serialize;
use serde_json::{Value, json};

/// The most events one run's timeline holds.
pub const TIMELINE_CAP: usize = 200;

/// The most earlier incidents of the same type a case lists.
pub const SIMILAR_LIMIT: usize = 5;

/// The most memories a case asks Engram for.
pub const RULINGS_LIMIT: u32 = 5;

/// The most downstream identities a blast radius names. The total is always
/// carried, so a long list is cut and says so.
pub const BLAST_LIST_CAP: usize = 20;

/// The planes a case reads beyond the console's own store.
pub struct CasePlanes<'a> {
    pub bus: &'a AppState,
    pub money: &'a MoneyState,
    pub policy: &'a PolicyState,
    pub identity: &'a IdentityState,
    pub memory: &'a MemoryState,
}

/// Why no case could be started. Each is an answer, never an empty case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaseError {
    /// The console has no readable event store, so there is nothing to start
    /// a case from.
    NoStore { reason: String },
    /// The id is not one this command reads.
    BadIncidentId { incident_id: String },
    /// A money, identity or posture incident: it has no event row.
    UnsupportedSource { source: String },
    /// The id names an event row the store does not hold.
    NotFound { event_id: i64 },
}

/// The store row id an incident id names.
///
/// `bus:<n>`, `verdryx:<n>` and a bare `<n>` name an event row. `money:`,
/// `idryx:` and `posture:` are Incident 360's other sources and have none.
/// Anything else is not an id this command reads. Only ASCII digits make a row
/// id: no sign, no space, no overflow.
pub fn event_id_of(incident_id: &str) -> Result<i64, CaseError> {
    Err(CaseError::BadIncidentId {
        incident_id: incident_id.chars().take(80).collect(),
    })
}

fn row_value<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value)
        .unwrap_or_else(|e| json!({ "unserialisable": e.to_string() }))
}

/// A plane's own error, as text. Serialised rather than formatted, so the case
/// holds what the plane said and not this build's paraphrase of it.
fn why<E: Serialize>(error: &E) -> String {
    serde_json::to_string(error).unwrap_or_else(|_| "the error could not be written".to_string())
}

fn one(row: Row) -> Answer {
    Answer::Rows {
        rows: vec![row],
        note: None,
    }
}

fn nothing() -> Answer {
    Answer::Rows {
        rows: Vec::new(),
        note: None,
    }
}

/// The three sections the console's own store answers, read synchronously.
pub fn store_sections(
    store: &Store,
    event: &StoredEvent,
    questions: &Questions,
) -> Vec<(Section, Answer)> {
    let _ = (store, event, questions);
    Vec::new()
}

/// What the identity can reach, computed from idryx's own records: its
/// permissions (how many, how many admin, how many unused), who it acts for,
/// and which identities act through it.
pub fn blast_radius(me: &IdryxIdentity, all: &[IdryxIdentity], alerts: usize) -> Value {
    let _ = (me, all, alerts);
    Value::Null
}

async fn spend(money: &MoneyState, run_id: &str) -> Answer {
    match crate::money::commands::money_runs(money).await {
        Err(e) => Answer::Unavailable { reason: why(&e) },
        Ok(runs) => match runs.into_iter().find(|r| r.run_id == run_id) {
            None => nothing(),
            Some(run) => one(Row {
                plane: Plane::Tokenfuse,
                row_id: run.run_id.clone(),
                label: label::SPEND,
                value: row_value(&run),
            }),
        },
    }
}

async fn identity(state: &IdentityState, agent: &str) -> (Answer, Answer) {
    use crate::identity::commands::{identity_list_alerts, identity_list_identities};

    let identities = match identity_list_identities(state).await {
        Ok(identities) => identities,
        Err(e) => {
            let reason = why(&e);
            return (
                Answer::Unavailable {
                    reason: reason.clone(),
                },
                Answer::Unavailable { reason },
            );
        }
    };
    let Some(me) = identities.iter().find(|i| i.id == agent) else {
        return (nothing(), nothing());
    };

    let (alerts, note) = match identity_list_alerts(state).await {
        Ok(alerts) => (
            alerts
                .into_iter()
                .filter(|a| a.identity == agent)
                .collect::<Vec<_>>(),
            None,
        ),
        Err(_) => (Vec::new(), Some(words::ALERTS_UNREAD)),
    };

    let mut rows = vec![Row {
        plane: Plane::Idryx,
        row_id: me.id.clone(),
        label: label::IDENTITY,
        value: row_value(me),
    }];
    rows.extend(alerts.iter().map(|a| Row {
        plane: Plane::Idryx,
        row_id: format!("{}:{}", a.detector, a.time),
        label: label::IDENTITY_ALERT,
        value: row_value(a),
    }));

    let blast = one(Row {
        plane: Plane::Idryx,
        row_id: format!("{}#blast_radius", me.id),
        label: label::BLAST_RADIUS,
        value: blast_radius(me, &identities, alerts.len()),
    });
    (Answer::Rows { rows, note }, blast)
}

async fn policy_status(state: &PolicyState) -> Answer {
    match crate::policy::commands::policy_enforcement_status(state).await {
        Err(e) => Answer::Unavailable { reason: why(&e) },
        Ok(status) => one(Row {
            plane: Plane::Wardryx,
            row_id: status.policy_version.clone(),
            label: label::POLICY,
            value: row_value(&status),
        }),
    }
}

async fn decisions(state: &PolicyState, run_id: &str) -> Answer {
    match crate::policy::commands::policy_list_approvals(state).await {
        Err(e) => Answer::Unavailable { reason: why(&e) },
        Ok(approvals) => Answer::Rows {
            rows: approvals
                .iter()
                .filter(|a| a.run_id == run_id)
                .map(|a| Row {
                    plane: Plane::Wardryx,
                    row_id: a.approval_id.clone(),
                    label: label::DECISION,
                    value: row_value(a),
                })
                .collect(),
            note: None,
        },
    }
}

async fn rulings(state: &MemoryState, event_type: &str) -> Answer {
    let query = format!("decision about {event_type}");
    match crate::memory::commands::memory_recall(
        query,
        RULINGS_LIMIT,
        "hybrid".to_string(),
        None,
        state,
    )
    .await
    {
        Err(e) => Answer::Unavailable { reason: why(&e) },
        Ok(memories) => Answer::Rows {
            rows: memories
                .iter()
                .map(|m| Row {
                    plane: Plane::Engram,
                    row_id: m.id.clone(),
                    label: label::RULING,
                    value: row_value(m),
                })
                .collect(),
            note: None,
        },
    }
}

/// Assemble the case for one incident.
///
/// The store is read first and closed; then the planes are asked, side by
/// side, only the questions [`questions_for`] allows. A plane that is not
/// ready, not configured or not reachable is a line in the case.
pub async fn case_file(incident_id: String, planes: CasePlanes<'_>) -> Result<CaseFile, CaseError> {
    let event_id = event_id_of(&incident_id)?;

    let (event, questions, mut answers) = {
        let dir = planes.bus.events_dir.as_ref().ok_or_else(|| CaseError::NoStore {
            reason: "the console has no event store on this box".to_string(),
        })?;
        let store = Store::open(&dir.join("console.sqlite")).map_err(|e| CaseError::NoStore {
            reason: e.to_string(),
        })?;
        let event = store
            .event_by_id(event_id)
            .map_err(|e| CaseError::NoStore {
                reason: e.to_string(),
            })?
            .ok_or(CaseError::NotFound { event_id })?;
        let questions = questions_for(&event);
        let answers = store_sections(&store, &event, &questions);
        (event, questions, answers)
    };

    let not_asked = |section: Section| -> Option<&'static str> {
        questions
            .not_asked
            .iter()
            .find(|(s, _)| *s == section)
            .map(|(_, why)| *why)
    };
    let held = |section: Section| match not_asked(section) {
        Some(reason) => Err(Answer::NotAsked { reason }),
        None => Ok(()),
    };

    let spend_answer = async {
        match (held(Section::Spend), questions.run_id.as_deref()) {
            (Err(answer), _) => answer,
            (Ok(()), Some(run_id)) => spend(planes.money, run_id).await,
            (Ok(()), None) => Answer::NotAsked {
                reason: words::NO_RUN,
            },
        }
    };
    let identity_answers = async {
        match (held(Section::Identity), questions.agent.as_deref()) {
            (Err(answer), _) => {
                let blast = held(Section::BlastRadius).err().unwrap_or_else(nothing);
                (answer, blast)
            }
            (Ok(()), Some(agent)) => identity(planes.identity, agent).await,
            (Ok(()), None) => (
                Answer::NotAsked {
                    reason: words::FILED_UNDER_A_KEY,
                },
                Answer::NotAsked {
                    reason: words::FILED_UNDER_A_KEY,
                },
            ),
        }
    };
    let decisions_answer = async {
        match (held(Section::Decisions), questions.run_id.as_deref()) {
            (Err(answer), _) => answer,
            (Ok(()), Some(run_id)) => decisions(planes.policy, run_id).await,
            (Ok(()), None) => Answer::NotAsked {
                reason: words::NO_RUN,
            },
        }
    };

    let (spend_answer, (identity_answer, blast_answer), policy_answer, decisions_answer, rulings_answer) = tokio::join!(
        spend_answer,
        identity_answers,
        policy_status(planes.policy),
        decisions_answer,
        rulings(planes.memory, &questions.event_type),
    );

    answers.push((Section::Spend, spend_answer));
    answers.push((Section::Identity, identity_answer));
    answers.push((Section::BlastRadius, blast_answer));
    answers.push((Section::Policy, policy_answer));
    answers.push((Section::Decisions, decisions_answer));
    answers.push((Section::PriorRulings, rulings_answer));

    Ok(assemble(
        Head {
            incident_id,
            subject: questions.subject.clone(),
            claimed_agent: questions.claimed_agent.clone(),
            run_id: questions.run_id.clone(),
        },
        answers,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use genaryx_connectors::IdryxPermission;

    #[test]
    fn an_incident_id_names_an_event_row_or_is_refused() {
        assert_eq!(event_id_of("bus:42"), Ok(42));
        assert_eq!(event_id_of("verdryx:7"), Ok(7));
        assert_eq!(event_id_of("42"), Ok(42));
        for source in ["money", "idryx", "posture"] {
            assert_eq!(
                event_id_of(&format!("{source}:inc-1:x:2026-10-07T10:00:00Z:3")),
                Err(CaseError::UnsupportedSource {
                    source: source.to_string()
                })
            );
        }
        for bad in [
            "",
            "bus:",
            "bus:abc",
            "bus:-1",
            "bus:+1",
            "bus: 1",
            "bus:1.5",
            "bus:99999999999999999999999",
            "nonsense:5",
            ":5",
        ] {
            assert!(
                matches!(event_id_of(bad), Err(CaseError::BadIncidentId { .. })),
                "{bad:?} was read as an event row"
            );
        }
    }

    /// A deterministic sweep, so a failure names its seed.
    #[test]
    fn two_hundred_seeds_of_hostile_incident_ids_never_panic_and_never_overread() {
        const ALPHABET: &[char] = &[
            '0', '1', '9', ':', 'b', 'u', 's', 'm', 'o', 'n', 'e', 'y', ' ', '-', '+', '\'', '"',
            '\n', '\0', 'é',
        ];
        for seed in 0..200u64 {
            let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let mut id = String::new();
            for _ in 0..(seed % 40) {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                id.push(ALPHABET[(x >> 33) as usize % ALPHABET.len()]);
            }
            match event_id_of(&id) {
                Ok(n) => assert!(
                    n >= 0 && id.chars().any(|c| c.is_ascii_digit()),
                    "seed {seed}: {id:?} became {n}"
                ),
                Err(CaseError::BadIncidentId { incident_id }) => {
                    assert!(incident_id.chars().count() <= 80, "seed {seed}");
                }
                Err(_) => {}
            }
        }
    }

    fn identity_record(id: &str, on_behalf_of: &[&str], permissions: Vec<IdryxPermission>) -> IdryxIdentity {
        IdryxIdentity {
            id: id.to_string(),
            identity_type: "agent".to_string(),
            privileged: true,
            source: "agents".to_string(),
            owner: "ops".to_string(),
            created: String::new(),
            last_used: String::new(),
            runtime: String::new(),
            on_behalf_of: on_behalf_of.iter().map(|s| s.to_string()).collect(),
            permissions,
            remediation: None,
            rotation: None,
            events: 0,
            alerts: 0,
        }
    }

    fn permission(name: &str, admin: bool, used: bool) -> IdryxPermission {
        IdryxPermission {
            name: name.to_string(),
            admin,
            used,
        }
    }

    #[test]
    fn the_blast_radius_counts_what_the_identity_can_reach() {
        let me = identity_record(
            "agent://x/a",
            &["user://x/owner"],
            vec![
                permission("s3:*", true, false),
                permission("sqs:send", false, true),
                permission("iam:pass", false, false),
            ],
        );
        let downstream_b = identity_record("agent://x/b", &["agent://x/a"], vec![]);
        let downstream_c = identity_record("agent://x/c", &["user://x/owner", "agent://x/a"], vec![]);
        let unrelated = identity_record("agent://x/d", &["user://x/owner"], vec![]);
        let all = vec![me.clone(), downstream_c, downstream_b, unrelated];

        let blast = blast_radius(&me, &all, 2);
        assert_eq!(blast["permissions"], 3);
        assert_eq!(blast["admin_permissions"], 1);
        assert_eq!(blast["unused_permissions"], 2);
        assert_eq!(blast["delegated_by"], json!(["user://x/owner"]));
        assert_eq!(
            blast["delegates_to"],
            json!(["agent://x/b", "agent://x/c"]),
            "sorted, and not including itself or an identity that acts for somebody else"
        );
        assert_eq!(blast["delegates_to_total"], 2);
        assert_eq!(blast["alerts"], 2);
    }

    #[test]
    fn a_long_list_of_delegates_is_cut_and_says_how_long_it_was() {
        let me = identity_record("agent://x/a", &[], vec![]);
        let mut all = vec![me.clone()];
        for n in 0..(BLAST_LIST_CAP + 5) {
            all.push(identity_record(&format!("agent://x/d{n:03}"), &["agent://x/a"], vec![]));
        }
        let blast = blast_radius(&me, &all, 0);
        assert_eq!(
            blast["delegates_to"].as_array().map(Vec::len),
            Some(BLAST_LIST_CAP)
        );
        assert_eq!(blast["delegates_to_total"], BLAST_LIST_CAP + 5);
    }
}
