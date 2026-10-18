//! Unit month caps: every business unit, its central monthly cap, what it
//! has spent this month, and where it stands (`money_units`), plus the one
//! signed mutation that sets a cap (`money_set_unit_budget`).
//!
//! # WHERE THE NUMBERS COME FROM
//!
//! Two Cloud reads, joined here. `GET /v1/units` folds month-to-date spend
//! per unit over the current UTC calendar month, the window a gateway
//! enforces a unit cap over. `GET /v1/unit-budgets` is the map of central
//! caps set on the control plane; every gateway of the org polls it every few
//! seconds and enforces it ahead of the cap its own identity map file names
//! (tokenfuse `unitledger.rs`, `effective_cap`). Before this module the
//! console read neither for a view: only Felyx's `budgets` tool did, and the
//! watch dock wrote that units carry no budget at all.
//!
//! # WHAT IT CANNOT SEE, SAID ON THE SCREEN
//!
//! A cap written only in a gateway's identity map file never reaches the
//! Cloud, so a unit with such a cap reads "no central cap" here. And the
//! month spend is the Cloud's fold of what the gateways pushed, which can lag
//! a gateway's own in-process counter by the calls not pushed yet.
//!
//! # THE MUTATION
//!
//! `money_set_unit_budget` is the same class of act as `money_set_budget`: a
//! break-glass override of Cloud state with a mandatory reason, admin only,
//! a fresh passkey assertion per action (the ceremony column), signed with
//! the paired device, one journal entry per attempt whatever the Cloud
//! answers. It refuses before the network an amount that is not a finite,
//! non-negative number, and the literal `unassigned`, which is the Cloud's
//! bucket for unmapped spend and not a unit a gateway can resolve. A cap of
//! zero is a real cap: the gateway then refuses every call of that unit.

use super::commands::{
    MoneyError, MutationOutcome, finish_mutation, micros_to_usd, ready_client,
    require_break_glass_reason,
};
use super::state::MoneyState;
use genaryx_connectors::{ConnectorError, UnitAgg};
use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeSet, HashMap};

/// The Cloud's literal bucket for spend whose run resolved no unit.
pub const UNASSIGNED: &str = "unassigned";

/// A unit at or above this share of its cap reads "near cap", the same
/// threshold the runs board uses for a run.
const NEAR_CAP: f64 = 0.8;

/// Where one unit stands against its central cap this month.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitCapState {
    /// Spent this month at or above the cap. A gateway refuses the unit's
    /// next call (`unit_budget_exceeded`); a cap of zero is always here.
    OverCap,
    /// At or above 80% of the cap.
    NearCap,
    /// Under 80% of the cap.
    UnderCap,
    /// The cap map answered and holds none for this unit. A cap in a
    /// gateway's identity map file is not visible here.
    NoCap,
    /// The cap map could not be read, so whether the unit has a cap is
    /// unknown. Never shown as "no cap".
    CapUnknown,
    /// A cap is set and the month's spend could not be read.
    SpendUnknown,
}

/// One unit's row.
#[derive(Debug, Clone, Serialize)]
pub struct UnitCapDto {
    pub unit: String,
    /// Month-to-date spend, `None` when the Cloud has no record of the unit
    /// this month or `/v1/units` could not be read. Never a zero it did not
    /// report.
    pub month_spent_usd: Option<f64>,
    pub month_calls: Option<u64>,
    /// The central cap, `None` when none is set or the map was not read
    /// (`caps_read` on the section says which).
    pub cap_usd: Option<f64>,
    /// `month_spent / cap` when both are known and the cap is above zero.
    pub fraction: Option<f64>,
    pub state: UnitCapState,
    /// False for the `unassigned` bucket: a cap cannot apply to spend that
    /// resolved no unit, so the view offers no cap control there.
    pub cappable: bool,
}

/// The whole section.
#[derive(Debug, Clone, Serialize)]
pub struct UnitCapsDto {
    /// The UTC month the spend column covers (`2026-10`), `None` when
    /// `/v1/units` was not read.
    pub month: Option<String>,
    /// `GET /v1/units` answered.
    pub units_read: bool,
    /// `GET /v1/unit-budgets` answered.
    pub caps_read: bool,
    /// What could not be read and why, one sentence each, for the view to
    /// print rather than a blank.
    pub notes: Vec<String>,
    /// Over first, then near, then the rest; highest month spend first in
    /// each.
    pub units: Vec<UnitCapDto>,
}

fn state_rank(s: UnitCapState) -> u8 {
    match s {
        UnitCapState::OverCap => 0,
        UnitCapState::NearCap => 1,
        UnitCapState::SpendUnknown => 2,
        UnitCapState::UnderCap => 3,
        UnitCapState::CapUnknown => 4,
        UnitCapState::NoCap => 5,
    }
}

/// Where a unit stands, from its cap and spend in micros.
fn standing(
    cap_micros: Option<i64>,
    caps_read: bool,
    spent_micros: Option<i64>,
) -> (UnitCapState, Option<f64>) {
    let Some(cap) = cap_micros else {
        let state = if caps_read {
            UnitCapState::NoCap
        } else {
            UnitCapState::CapUnknown
        };
        return (state, None);
    };
    if cap <= 0 {
        // A zero (or negative) cap refuses every call: over, whatever spent.
        return (UnitCapState::OverCap, None);
    }
    let Some(spent) = spent_micros else {
        return (UnitCapState::SpendUnknown, None);
    };
    let fraction = spent as f64 / cap as f64;
    let state = if spent >= cap {
        UnitCapState::OverCap
    } else if fraction >= NEAR_CAP {
        UnitCapState::NearCap
    } else {
        UnitCapState::UnderCap
    };
    (state, Some(fraction))
}

/// Join the two reads. `units`/`caps` are `None` when that read did not
/// answer; a unit present in either is listed.
pub fn join_unit_caps(
    units: Option<&[UnitAgg]>,
    caps: Option<&HashMap<String, i64>>,
) -> Vec<UnitCapDto> {
    let caps_read = caps.is_some();
    let by_unit: HashMap<&str, &UnitAgg> = units
        .unwrap_or_default()
        .iter()
        .map(|u| (u.unit.as_str(), u))
        .collect();
    let mut names: BTreeSet<&str> = by_unit.keys().copied().collect();
    if let Some(caps) = caps {
        names.extend(caps.keys().map(String::as_str));
    }
    let mut rows: Vec<UnitCapDto> = names
        .into_iter()
        .map(|name| {
            let agg = by_unit.get(name);
            let spent = agg.map(|u| u.month_spent_microusd);
            let cap = caps.and_then(|c| c.get(name).copied());
            let (state, fraction) = standing(cap, caps_read, spent);
            UnitCapDto {
                unit: name.to_string(),
                month_spent_usd: spent.map(micros_to_usd),
                month_calls: agg.map(|u| u.month_calls),
                cap_usd: cap.map(micros_to_usd),
                fraction,
                state,
                cappable: name != UNASSIGNED,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        state_rank(a.state).cmp(&state_rank(b.state)).then_with(|| {
            b.month_spent_usd
                .unwrap_or(0.0)
                .total_cmp(&a.month_spent_usd.unwrap_or(0.0))
                .then_with(|| a.unit.cmp(&b.unit))
        })
    });
    rows
}

/// A read that did not answer, said as one sentence.
fn unread(route: &str, e: &ConnectorError) -> String {
    format!("TokenFuse Cloud did not answer {route}: {e}")
}

/// Every unit with its central cap and this month's spend.
///
/// A Cloud without either route (older than the unit fold) is a note, not an
/// error. Only when neither read answers at all, for a reason other than the
/// route being absent, is it an error: an empty section would read as a box
/// with no units.
pub async fn money_units(state: &MoneyState) -> Result<UnitCapsDto, MoneyError> {
    let client = ready_client(&state).await?;
    let (units, caps) = tokio::join!(client.client.units(), client.client.unit_budgets());
    let mut notes = Vec::new();

    let units = match units {
        Ok(Some(u)) => Some(u),
        Ok(None) => {
            notes.push(
                "This Cloud has no GET /v1/units, so no month spend can be shown.".to_string(),
            );
            None
        }
        Err(e) => {
            notes.push(unread("GET /v1/units", &e));
            if let Err(caps_err) = &caps {
                eprintln!("genaryx: money_units could not read units ({e}) or caps ({caps_err})");
                return Err(MoneyError::from(e));
            }
            None
        }
    };
    let caps = match caps {
        Ok(Some(c)) => Some(c),
        Ok(None) => {
            notes.push(
                "This Cloud has no GET /v1/unit-budgets, so whether a unit has a cap is unknown."
                    .to_string(),
            );
            None
        }
        Err(e) => {
            notes.push(unread("GET /v1/unit-budgets", &e));
            None
        }
    };

    let month = units
        .as_ref()
        .and_then(|u| u.iter().map(|a| a.month.clone()).find(|m| !m.is_empty()));
    Ok(UnitCapsDto {
        month,
        units_read: units.is_some(),
        caps_read: caps.is_some(),
        notes,
        units: join_unit_caps(units.as_deref(), caps.as_ref()),
    })
}

/// Validate a cap change before anything is signed or sent.
pub fn validate_unit_cap(unit: &str, budget_usd: f64) -> Result<(), MoneyError> {
    if unit.trim().is_empty() {
        return Err(MoneyError::InvalidArgument {
            message: "a unit cap needs a unit id".to_string(),
        });
    }
    if unit == UNASSIGNED {
        return Err(MoneyError::InvalidArgument {
            message: "\"unassigned\" is the Cloud's bucket for spend that resolved no unit; a cap cannot apply to it".to_string(),
        });
    }
    if !budget_usd.is_finite() || budget_usd < 0.0 {
        return Err(MoneyError::InvalidArgument {
            message: format!(
                "a unit cap must be a finite amount of zero or more USD, got {budget_usd}"
            ),
        });
    }
    Ok(())
}

/// Set a unit's central monthly cap: a break-glass override, same contract as
/// `money_set_budget`, journaled as `console.set_unit_budget`.
pub async fn money_set_unit_budget(
    unit: String,
    budget_usd: f64,
    reason: String,
    state: &MoneyState,
) -> Result<MutationOutcome, MoneyError> {
    require_break_glass_reason(&reason)?;
    validate_unit_cap(&unit, budget_usd)?;
    let client = ready_client(&state).await?;
    let result = client.client.set_unit_budget(&unit, budget_usd).await;
    finish_mutation(
        &client,
        "console.set_unit_budget",
        &unit,
        json!({ "reason": reason, "budget_usd": budget_usd }),
        "break_glass",
        result,
        |resp| {
            (
                format!("unit {unit} month cap set to ${budget_usd:.4}"),
                format!("budget_micros:{}", resp.budget_micros),
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agg(unit: &str, month_spent: i64) -> UnitAgg {
        UnitAgg {
            unit: unit.to_string(),
            month: "2026-10".to_string(),
            month_spent_microusd: month_spent,
            month_calls: 3,
            ..Default::default()
        }
    }

    #[test]
    fn standing_is_exact_at_the_cap_and_the_near_threshold() {
        assert_eq!(
            standing(Some(1000), true, Some(1000)).0,
            UnitCapState::OverCap
        );
        assert_eq!(
            standing(Some(1000), true, Some(999)).0,
            UnitCapState::NearCap
        );
        assert_eq!(
            standing(Some(1000), true, Some(800)).0,
            UnitCapState::NearCap
        );
        assert_eq!(
            standing(Some(1000), true, Some(799)).0,
            UnitCapState::UnderCap
        );
        assert_eq!(standing(Some(0), true, Some(0)).0, UnitCapState::OverCap);
        assert_eq!(standing(Some(0), true, None).0, UnitCapState::OverCap);
        assert_eq!(standing(Some(5), true, None).0, UnitCapState::SpendUnknown);
        assert_eq!(standing(None, true, Some(5)).0, UnitCapState::NoCap);
        assert_eq!(standing(None, false, Some(5)).0, UnitCapState::CapUnknown);
        assert_eq!(standing(Some(2000), true, Some(500)).1, Some(0.25));
    }

    #[test]
    fn the_unassigned_bucket_is_listed_and_never_offered_a_cap() {
        let rows = join_unit_caps(Some(&[agg(UNASSIGNED, 40), agg("finops", 10)][..]), None);
        let unassigned = rows.iter().find(|r| r.unit == UNASSIGNED).expect("listed");
        assert!(!unassigned.cappable);
        assert!(
            rows.iter()
                .find(|r| r.unit == "finops")
                .expect("listed")
                .cappable
        );
    }

    #[test]
    fn a_cap_change_is_refused_before_the_network_when_it_cannot_be_a_cap() {
        for (unit, usd) in [
            ("", 1.0),
            ("  ", 1.0),
            (UNASSIGNED, 1.0),
            ("finops", -0.01),
            ("finops", f64::NAN),
            ("finops", f64::INFINITY),
        ] {
            assert!(
                matches!(
                    validate_unit_cap(unit, usd),
                    Err(MoneyError::InvalidArgument { .. })
                ),
                "{unit:?} {usd}"
            );
        }
        assert!(
            validate_unit_cap("finops", 0.0).is_ok(),
            "a zero cap is a real cap"
        );
        assert!(validate_unit_cap("finops", 12.5).is_ok());
    }
}
