//! Who a bus event is filed under, which is not always the `agent_id` on its
//! envelope.
//!
//! # THE ONE EXCEPTION, AND WHY IT EXISTS
//!
//! Every event on this bus is about the agent its envelope names, with one
//! exception: `identity_mismatch`. TokenFuse writes that event when a caller
//! presented a credential that may not speak as the agent it claimed, and it
//! puts the CLAIMED id on the envelope (`agent_id`) and the credential that
//! actually called in `data.key_id`. So the envelope names exactly the agent
//! that did NOT make the call.
//!
//! Before this module the console read the envelope as it reads every other
//! event, and an impersonation attempt landed on its victim four times over:
//! the victim's "stopped" count went up, the attempt sat on the victim's own
//! Agent 360 stop list, the incident centre named the victim as the subject
//! of a high-severity incident, and Felyx's cost-per-action counted the
//! refused calls among the victim's own. Each of those was a correct rendering
//! of a field and a false statement about who did what.
//!
//! The rule is TokenFuse's own (its invariant 81, the FOCUS export side of
//! the same event): a refused identity is filed under the credential,
//! `key:<key_id>`, and never under the agent it claimed. The `key:` prefix
//! keeps a key that happens to be named like an agent out of that agent's
//! figures, since no agent id starts with it. With no key on the event
//! (TokenFuse running without client keys), the refusal is filed under
//! [`NO_KEY`], a subject that names nobody rather than the one agent it is
//! known not to be.
//!
//! # TWO SPELLINGS, HELD EQUAL
//!
//! The rule is needed in Rust (a row already read) and in SQL (an aggregate
//! that reads no rows, see `Store::type_counts_since`). [`filed_under`] and
//! [`FILED_UNDER_SQL`] are those two spellings, and
//! `the_sql_and_the_rust_spelling_of_the_rule_agree` in `store.rs`'s tests
//! runs both over the same hostile shapes so they cannot drift apart.

use serde_json::Value;

/// The event type whose envelope `agent_id` is a claim, not a subject.
pub const IDENTITY_MISMATCH: &str = "identity_mismatch";

/// The prefix of a subject that is a credential rather than an agent.
pub const KEY_PREFIX: &str = "key:";

/// The subject of an identity refusal that carried no key. Parenthesised so it
/// cannot be read as a key someone actually named.
pub const NO_KEY: &str = "key:(none)";

/// Who `type_`'s event is filed under: its envelope `agent_id`, except for an
/// identity refusal, which is filed under the key that made the call.
///
/// Only a non-empty STRING `key_id` names a key. A number, an object, `null`,
/// an empty string or no `data` at all is [`NO_KEY`]: TokenFuse writes a
/// string or `null`, and anything else is a producer this console cannot read
/// a credential from, which is not a reason to fall back to the claim.
pub fn filed_under(type_: &str, agent_id: &str, data: Option<&Value>) -> String {
    if type_ != IDENTITY_MISMATCH {
        return agent_id.to_string();
    }
    match data
        .and_then(|d| d.get("key_id"))
        .and_then(|v| v.as_str())
        .filter(|k| !k.is_empty())
    {
        Some(key) => format!("{KEY_PREFIX}{key}"),
        None => NO_KEY.to_string(),
    }
}

/// The key an identity refusal names, for a sentence that has to say it.
/// `None` for any other event, and for a refusal that carried no key.
pub fn refused_key(type_: &str, data: Option<&Value>) -> Option<String> {
    if type_ != IDENTITY_MISMATCH {
        return None;
    }
    data.and_then(|d| d.get("key_id"))
        .and_then(|v| v.as_str())
        .filter(|k| !k.is_empty())
        .map(String::from)
}

/// [`filed_under`] as a SQL expression over the `events` table's own columns
/// (`type`, `agent_id`, `data`).
///
/// The `CASE` nesting is deliberate. SQLite does not promise to short-circuit
/// `AND`, and `json_type` raises on a value that is not JSON, so the validity
/// check has to be a branch the type check sits inside, not a conjunct beside
/// it. One malformed row must not take a whole panel down.
pub const FILED_UNDER_SQL: &str = "(CASE WHEN type = 'identity_mismatch' THEN \
     'key:' || COALESCE(NULLIF(CASE WHEN json_valid(data) THEN \
     CASE WHEN json_type(data, '$.key_id') = 'text' THEN json_extract(data, '$.key_id') END \
     END, ''), '(none)') \
     ELSE agent_id END)";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_identity_refusal_is_filed_under_its_key_and_every_other_event_under_its_agent() {
        let claimed = "agent://taipanbox.dev/routers/flint";
        let refusal = json!({ "key_id": "forge-imposter", "agent_id": claimed });
        assert_eq!(
            filed_under(IDENTITY_MISMATCH, claimed, Some(&refusal)),
            "key:forge-imposter"
        );
        assert_eq!(
            refused_key(IDENTITY_MISMATCH, Some(&refusal)).as_deref(),
            Some("forge-imposter")
        );
        // A different type carrying the very same data is about its agent.
        assert_eq!(filed_under("policy_deny", claimed, Some(&refusal)), claimed);
        assert_eq!(refused_key("policy_deny", Some(&refusal)), None);
    }

    #[test]
    fn an_identity_refusal_with_no_readable_key_names_no_agent() {
        let claimed = "agent://taipanbox.dev/routers/flint";
        for data in [
            None,
            Some(json!({})),
            Some(json!({ "key_id": null })),
            Some(json!({ "key_id": "" })),
            Some(json!({ "key_id": 42 })),
            Some(json!({ "key_id": { "nested": "x" } })),
            Some(json!("not an object")),
        ] {
            let got = filed_under(IDENTITY_MISMATCH, claimed, data.as_ref());
            assert_eq!(got, NO_KEY, "data {data:?}");
            assert!(!got.contains("flint"), "a refusal fell back to the claim");
        }
    }
}
