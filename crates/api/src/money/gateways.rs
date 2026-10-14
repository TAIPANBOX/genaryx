//! Which gateways the Cloud hears from, and which one stopped reporting
//! (`money_gateways`).
//!
//! Every gateway with a Cloud sink pushes its calls, and an empty batch as a
//! heartbeat every 30 s by default (`TOKENFUSE_CLOUD_HEARTBEAT_SECONDS`,
//! tokenfuse invariant 66), so a quiet healthy gateway still reports. The
//! Cloud keeps, per site, when it last heard from it, by its own clock
//! (`GET /v1/gateways`). A gateway that crashed, lost its network or its key
//! stops pushing, and its runs stop moving in every other view without
//! saying why. Until 2026-10-14 nothing in this console read that route, so
//! a silent gateway was invisible in the operator's own UI.
//!
//! A site is `silent` when the Cloud has not heard from it for longer than
//! [`SILENT_AFTER_MS`], five minutes, the threshold TokenFuse's own dashboard
//! uses: ten missed heartbeats at the default interval. A gateway run with
//! heartbeats off (`0`) pushes only when it has calls, so it reads silent
//! whenever it is idle; the view says so. Ages are computed against this
//! console's clock, so clock skew between the console and the Cloud moves
//! them; that too is said where the ages are shown.

use super::commands::{MoneyError, micros_to_usd, millis_to_iso, ready_client};
use super::state::MoneyState;
use genaryx_connectors::GatewayAgg;
use serde::Serialize;

/// No push for longer than this, and the site reads silent.
pub const SILENT_AFTER_MS: i64 = 5 * 60 * 1000;

/// Whether the Cloud is still hearing from a gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayState {
    Reporting,
    Silent,
}

/// One site's row.
#[derive(Debug, Clone, Serialize)]
pub struct GatewayDto {
    /// The site name, or `"unnamed"` for a key bound to no site.
    pub site: String,
    pub state: GatewayState,
    /// When the Cloud last heard from it (ISO-8601, the Cloud's clock).
    pub last_push: String,
    /// Seconds since then by this console's clock. Negative only when the
    /// two clocks disagree.
    pub last_push_age_secs: i64,
    /// The newest call timestamp the gateway itself reported, ISO-8601;
    /// `None` before any call. Descriptive only, never used for `state`.
    pub last_record: Option<String>,
    pub first_push: String,
    pub pushes: u64,
    pub calls: u64,
    pub spent_usd: f64,
}

/// The whole section.
#[derive(Debug, Clone, Serialize)]
pub struct GatewaysDto {
    /// False when this Cloud has no `GET /v1/gateways` at all: whether a
    /// gateway stopped reporting cannot be said, which is not "none did".
    pub reported: bool,
    pub silent_after_secs: i64,
    /// This console's clock when the ages were computed.
    pub checked_at: String,
    /// Silent sites first, then by last push, newest first.
    pub gateways: Vec<GatewayDto>,
}

/// The rows for `list` as of `now_ms`. Pure, so the threshold is testable
/// at its exact boundary.
pub fn gateways_view(list: &[GatewayAgg], now_ms: i64) -> Vec<GatewayDto> {
    let mut rows: Vec<GatewayDto> = list
        .iter()
        .map(|g| {
            let age_ms = now_ms.saturating_sub(g.last_push_millis);
            let state = if g.last_push_millis <= 0 || age_ms > SILENT_AFTER_MS {
                GatewayState::Silent
            } else {
                GatewayState::Reporting
            };
            GatewayDto {
                site: g.site.clone(),
                state,
                last_push: millis_to_iso(g.last_push_millis),
                last_push_age_secs: age_ms / 1000,
                last_record: (g.last_record_millis > 0)
                    .then(|| millis_to_iso(g.last_record_millis)),
                first_push: millis_to_iso(g.first_push_millis),
                pushes: g.pushes,
                calls: g.calls,
                spent_usd: micros_to_usd(g.spent_microusd),
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        (a.state == GatewayState::Reporting)
            .cmp(&(b.state == GatewayState::Reporting))
            .then_with(|| a.last_push_age_secs.cmp(&b.last_push_age_secs))
            .then_with(|| a.site.cmp(&b.site))
    });
    rows
}

/// Every gateway the Cloud has heard from, and which ones went silent.
pub async fn money_gateways(state: &MoneyState) -> Result<GatewaysDto, MoneyError> {
    let client = ready_client(&state).await?;
    let list = client.client.gateways().await.map_err(MoneyError::from)?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    Ok(GatewaysDto {
        reported: list.is_some(),
        silent_after_secs: SILENT_AFTER_MS / 1000,
        checked_at: millis_to_iso(now_ms),
        gateways: list
            .as_deref()
            .map(|l| gateways_view(l, now_ms))
            .unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_791_985_800_000;

    fn gw(site: &str, last_push: i64) -> GatewayAgg {
        GatewayAgg {
            site: site.to_string(),
            last_push_millis: last_push,
            first_push_millis: NOW - 86_400_000,
            pushes: 10,
            ..Default::default()
        }
    }

    #[test]
    fn a_gateway_is_silent_only_past_five_minutes_without_a_push() {
        let rows = gateways_view(
            &[
                gw("at-the-line", NOW - SILENT_AFTER_MS),
                gw("one-ms-past", NOW - SILENT_AFTER_MS - 1),
                gw("heartbeat", NOW - 30_000),
                gw("never", 0),
                gw("ahead", NOW + 10_000),
            ],
            NOW,
        );
        let state = |s: &str| rows.iter().find(|r| r.site == s).expect(s).state;
        assert_eq!(state("at-the-line"), GatewayState::Reporting);
        assert_eq!(state("one-ms-past"), GatewayState::Silent);
        assert_eq!(state("heartbeat"), GatewayState::Reporting);
        assert_eq!(state("never"), GatewayState::Silent);
        assert_eq!(state("ahead"), GatewayState::Reporting);
        assert_eq!(
            rows.iter()
                .find(|r| r.site == "ahead")
                .expect("row")
                .last_push_age_secs,
            -10
        );
    }

    #[test]
    fn silent_gateways_lead_and_the_rest_follow_newest_first() {
        let rows = gateways_view(
            &[
                gw("b-fresh", NOW - 5_000),
                gw("a-silent-long", NOW - 3_600_000),
                gw("c-older", NOW - 60_000),
                gw("d-silent", NOW - 600_000),
            ],
            NOW,
        );
        let order: Vec<&str> = rows.iter().map(|r| r.site.as_str()).collect();
        assert_eq!(order, ["d-silent", "a-silent-long", "b-fresh", "c-older"]);
    }

    #[test]
    fn a_gateway_with_no_call_yet_names_no_record_time() {
        let mut g = gw("quiet", NOW - 1_000);
        g.last_record_millis = 0;
        assert_eq!(gateways_view(&[g.clone()], NOW)[0].last_record, None);
        g.last_record_millis = NOW - 2_000;
        assert!(gateways_view(&[g], NOW)[0].last_record.is_some());
    }
}
