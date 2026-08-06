//! One place to write a `console_command` for a privileged action that has
//! already happened.
//!
//! Every plane that mutates something already journals: the money plane's
//! kill and budget change, the policy plane's approval decision, the two
//! WireGuard peer commands. Each grew its own copy of the same twenty lines,
//! and the lifecycle blocks (freeze an agent, stop a unit, stop a user) grew
//! none at all, so the one class of action that writes real deny-all
//! enforcement across a whole fleet was the class with no bus record.
//!
//! The record is best-effort ON PURPOSE, and the shape of that matters. The
//! enforcement has already taken by the time this runs, so a journal failure
//! must not turn a completed action into a reported failure: the operator
//! would retry an action that already succeeded. It is logged loudly instead,
//! because an unjournaled privileged action is exactly what an operator needs
//! told. What this must never do is stay silent.

use crate::money::state::BusHandle;

/// Journal one already-completed privileged console action.
///
/// `action` is the `data.action` value (`console.kill_run`,
/// `console.block_unit`, ...), `target` what it applied to, and
/// `verify_result` a short human-readable outcome (`blocked:true`,
/// `killed:true`). `params` reaches the queryable `commands_journal` row but
/// deliberately never the bus envelope, which has one fixed shape whatever
/// the operator did (see `genaryx_core::command::console_command_line`).
///
/// Returns whether the record was written, plus the reason it was not. Every
/// caller today logs and carries on; the pair is returned rather than a bare
/// `bool` so a caller that wants to surface it has the reason to hand.
pub fn record_console_action(
    bus: Option<&BusHandle>,
    action: &str,
    target: &str,
    params: serde_json::Value,
    verify_result: String,
) -> (bool, Option<String>) {
    let Some(bus) = bus else {
        let why = "no live event bus".to_string();
        eprintln!("genaryx: {action} on {target} was NOT journaled: {why}");
        return (false, Some(why));
    };

    // The signature the WebAuthn gate put in scope for THIS request, so the
    // record names the credential the human actually touched. Falls back to
    // the same honest label the rest of the console uses when no passkey is
    // enrolled, never a fabricated one.
    let (sig_alg, sig_fpr) =
        crate::console_actor::signature_or("software-signed", "software-signed");
    let org_domain = std::env::var("GENARYX_ORG_DOMAIN").unwrap_or_else(|_| "local".to_string());
    let operator = crate::console_actor::operator_or(&format!("user://{org_domain}/operator"));
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "console".to_string());

    let rec = genaryx_core::command::CommandRecord {
        operator,
        env: org_domain.clone(),
        action: action.to_string(),
        target: target.to_string(),
        params,
        // Not a break-glass override: these are the sanctioned paths, gated
        // by a passkey rather than bypassing anything.
        decision: "allow".to_string(),
        sig_alg,
        sig_fpr,
        http_status: 200,
        verify_result,
    };

    let store = match genaryx_core::store::Store::open(&bus.store_db_path) {
        Ok(store) => store,
        Err(e) => {
            eprintln!("genaryx: {action} on {target} succeeded but was NOT journaled: {e}");
            return (false, Some(e.to_string()));
        }
    };
    match genaryx_core::command::record(&store, &bus.console_events_path, &org_domain, &host, &rec)
    {
        Ok(()) => (true, None),
        Err(e) => {
            eprintln!("genaryx: {action} on {target} succeeded but was NOT journaled: {e}");
            (false, Some(e.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "genaryx-journal-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn a_journaled_action_lands_on_the_bus_and_conforms() {
        let dir = scratch("ok");
        let bus = BusHandle::from_dirs(&dir, &dir);

        let (journaled, err) = record_console_action(
            Some(&bus),
            "console.block_unit",
            "payments",
            serde_json::json!({ "members": 3 }),
            "blocked:true".to_string(),
        );
        assert!(journaled, "journal_error: {err:?}");
        assert!(err.is_none());

        let body = std::fs::read_to_string(&bus.console_events_path).expect("read the events file");
        let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 1, "exactly one console_command appended");

        let conformer = genaryx_core::Conformer::new().expect("embedded schemas must compile");
        let report = conformer.check_line(lines[0]);
        assert!(
            report.valid,
            "the journaled line must conform: {:?}\n  line: {}",
            report.errors, lines[0]
        );

        let v: serde_json::Value = serde_json::from_str(lines[0]).expect("parse");
        assert_eq!(v.get("source").and_then(|s| s.as_str()), Some("console"));
        assert_eq!(
            v.get("type").and_then(|s| s.as_str()),
            Some("console_command")
        );
        assert_eq!(
            v.pointer("/data/action").and_then(|s| s.as_str()),
            Some("console.block_unit")
        );
        assert_eq!(
            v.pointer("/data/target").and_then(|s| s.as_str()),
            Some("payments")
        );
        assert_eq!(
            v.pointer("/data/verify_result").and_then(|s| s.as_str()),
            Some("blocked:true")
        );
        // `params` is journaled to the queryable row, never to the envelope.
        assert!(v.pointer("/data/members").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_bus_is_reported_rather_than_silently_skipped() {
        let (journaled, err) = record_console_action(
            None,
            "console.block_agent",
            "agent://acme.example/bot/a",
            serde_json::json!({}),
            "blocked:true".to_string(),
        );
        assert!(!journaled);
        assert_eq!(err.as_deref(), Some("no live event bus"));
    }
}
