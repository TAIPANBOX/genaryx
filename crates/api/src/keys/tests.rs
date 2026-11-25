//! Invariant 53: the fold of custody's state and inventory, and the settings.

use super::*;

fn entry(name: &str, kid: &str, state: &str, created: i64) -> EntryBody {
    EntryBody {
        name: name.to_string(),
        alg: "ES384".to_string(),
        kid: kid.to_string(),
        mode: "held".to_string(),
        state: state.to_string(),
        created,
        activate_at: None,
        retired: None,
        revoked: None,
        owner: None,
        not_after: None,
    }
}

fn state(name: &str) -> KeyStateBody {
    KeyStateBody {
        name: name.to_string(),
        alg: "ES384".to_string(),
        mode: "held".to_string(),
        active_kid: Some(format!("{name}-k2")),
        active_since: Some(1_700_000_100),
        next_kid: None,
        next_activates_at: None,
        renew_due: None,
    }
}

#[test]
fn the_two_settings_come_from_their_own_variables() {
    assert_eq!(CUSTODY_ENV, "GENARYX_KEYRYX_URL");
    assert_eq!(WITNESS_ENV, "GENARYX_KEYRYX_WITNESS_URL");
    assert_eq!(base_url_from("  "), Ok(None));
    assert_eq!(
        base_url_from(" http://127.0.0.1:4340/ \n"),
        Ok(Some("http://127.0.0.1:4340".to_string()))
    );
    assert!(base_url_from("127.0.0.1:4340").is_err());
    assert!(base_url_from("file:///etc/passwd").is_err());
}

#[test]
fn the_last_rotation_is_when_the_newest_key_of_a_name_was_made() {
    let part = fold_custody(
        "http://c".to_string(),
        "v0.1.5".to_string(),
        vec![state("wardryx.approval")],
        vec![],
        Some(vec![
            entry("wardryx.approval", "k1", "retired", 1_700_000_000),
            entry("wardryx.approval", "k2", "active", 1_700_000_100),
            entry("trailryx.seal", "s1", "active", 1_800_000_000),
        ]),
        None,
    );
    let CustodyPart::Read {
        names,
        entries_total,
        ..
    } = part
    else {
        panic!("expected a read inventory");
    };
    assert_eq!(entries_total, 3);
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].last_rotation, Some(1_700_000_100));
    assert_eq!(names[0].states.get("retired"), Some(&1));
    assert_eq!(names[0].states.get("active"), Some(&1));
    assert_eq!(names[0].active_kid.as_deref(), Some("wardryx.approval-k2"));
}

#[test]
fn an_inventory_that_did_not_answer_is_said_and_no_rotation_is_invented() {
    let part = fold_custody(
        "http://c".to_string(),
        "v".to_string(),
        vec![state("wardryx.approval")],
        vec![],
        None,
        Some("answered 500".to_string()),
    );
    let CustodyPart::Read {
        names,
        inventory_error,
        entries,
        ..
    } = part
    else {
        panic!("expected a read state");
    };
    assert_eq!(inventory_error.as_deref(), Some("answered 500"));
    assert!(entries.is_empty());
    assert_eq!(names[0].last_rotation, None);
}

#[test]
fn alarms_and_entries_are_bounded_and_entries_come_newest_first() {
    let alarms: Vec<AlarmBody> = (0..150)
        .map(|i| AlarmBody {
            kind: "state_unverified".to_string(),
            name: format!("n{i}"),
            kid: String::new(),
            reason: "r".repeat(2000),
        })
        .collect();
    let entries: Vec<EntryBody> = (0..600)
        .map(|i| entry("n", &format!("k{i}"), "retired", i))
        .collect();
    let CustodyPart::Read {
        alarms,
        alarms_total,
        entries,
        entries_total,
        ..
    } = fold_custody("u".into(), "v".into(), vec![], alarms, Some(entries), None)
    else {
        panic!("expected a read");
    };
    assert_eq!((alarms.len(), alarms_total), (MAX_ALARMS, 150));
    assert!(alarms[0].reason.chars().count() <= 515);
    assert_eq!((entries.len(), entries_total), (MAX_ENTRIES, 600));
    assert_eq!(entries[0].created, 599);
}
