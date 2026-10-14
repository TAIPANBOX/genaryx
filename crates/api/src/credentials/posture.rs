//! The gateway's posture: the switches in force, the client keys and the
//! identity map, and the price book with its tiers (`credentials_posture`).
//!
//! Until 2026-10-14 none of this was visible in the console. The firewall
//! mode, the PII mode, the run-budget ceiling and the wire a gateway runs
//! with were environment variables an operator had to read on the box; the
//! price book (`GET /v1/price-book`) was never read at all. Three reads, each
//! independent, each through the credentials plane's gateway client and its
//! admin key:
//!
//! - `GET /v1/settings`: the gateway's own report of its switches, newer than
//!   this console. A gateway without the route is "not reported by this
//!   gateway version", never "off". Its fields are read one by one and
//!   leniently, so a field this console does not know is listed as reported
//!   rather than dropped, and a value under a name that suggests a secret is
//!   withheld whatever the gateway sent: the route promises no secrets, and
//!   this console does not rely on the promise.
//! - `GET /v1/keys`: whether client keys and an identity map are configured,
//!   the identity check's mode, and refused attempts since startup.
//! - `GET /v1/price-book`: the book calls are priced with, the operator's
//!   overrides and additions, the fallback rate, the prompt-length tiers, and
//!   the model ids charged at the fallback since the gateway started.
//!
//! One read failing costs only its own section, said in a note; only all
//! three failing is an error, since a page of notes would read as a gateway
//! with nothing configured.

use super::commands::{CredentialsError, ready_client};
use super::state::CredentialsState;
use genaryx_connectors::{GatewayError, GatewayKeysReport, GatewayPriceBook};
use serde::Serialize;
use serde_json::Value;

/// The switches this console names, in the order it shows them, each with
/// its label. A reported field not in this list is still shown, under its
/// own name, after these.
pub const KNOWN_SETTINGS: [(&str, &str); 14] = [
    ("firewall", "Firewall (taint)"),
    ("dlp_pii", "PII detection"),
    ("dlp", "Data-loss prevention"),
    ("max_run_budget_usd", "Run budget ceiling (USD)"),
    ("wire", "Wire (provider shape)"),
    ("identity_strict", "Identity check"),
    ("client_keys", "Client keys"),
    ("require_run_id", "Run id required"),
    ("wardryx_mode", "Policy hook (wardryx)"),
    ("failmode", "Policy fail mode"),
    ("mode", "Gateway mode"),
    ("cache", "Response cache"),
    ("router", "FinOps router"),
    ("closure_enabled", "Retained-reservation closure"),
];

/// The most extra (unknown) settings shown, and the longest value printed.
pub const MAX_EXTRA_SETTINGS: usize = 32;
pub const MAX_VALUE_CHARS: usize = 120;

/// What stands in for a value withheld because of its name.
pub const WITHHELD: &str = "withheld: the console never shows a value under this name";

/// One switch.
#[derive(Debug, Clone, Serialize)]
pub struct PostureSetting {
    pub key: String,
    pub label: String,
    /// The value as text; `None` when a reporting gateway did not include
    /// this field ("not reported by this gateway version").
    pub value: Option<String>,
}

/// The settings section.
#[derive(Debug, Clone, Serialize)]
pub struct SettingsSection {
    /// False when the gateway has no `GET /v1/settings`.
    pub reported: bool,
    pub items: Vec<PostureSetting>,
}

/// The keys and identity-map section, from `GET /v1/keys`.
#[derive(Debug, Clone, Serialize)]
pub struct KeysSection {
    pub identity_strict: String,
    pub identity_map_configured: bool,
    /// Keys present in the gateway's client-key list right now.
    pub configured_keys: usize,
    /// Keys the report names at all (configured, bound, or seen).
    pub keys_listed: usize,
    pub unauthorized_attempts: u64,
}

/// One prompt-length tier.
#[derive(Debug, Clone, Serialize)]
pub struct TierRow {
    pub model: String,
    pub above_prompt_tokens: u64,
    pub input_per_mtok_usd: f64,
    pub output_per_mtok_usd: f64,
}

/// The price book section, from `GET /v1/price-book`.
#[derive(Debug, Clone, Serialize)]
pub struct PriceBookSection {
    pub rows: u64,
    pub built_in_rows: u64,
    pub operator_file: bool,
    pub overridden_models: Vec<String>,
    pub added_models: Vec<String>,
    pub fallback_input_per_mtok_usd: Option<f64>,
    pub fallback_output_per_mtok_usd: Option<f64>,
    /// `None` from a gateway older than prompt-length tiers.
    pub tiers: Option<Vec<TierRow>>,
    pub tiers_dropped_models: Vec<String>,
    pub fallback_models_seen: Vec<String>,
    pub fallback_models_seen_total: u64,
}

/// The whole view.
#[derive(Debug, Clone, Serialize)]
pub struct GatewayPostureDto {
    pub gateway_url: String,
    /// `None` only when the read failed; see `notes`.
    pub settings: Option<SettingsSection>,
    pub keys: Option<KeysSection>,
    /// `None` when the read failed or the gateway has no such route; `notes`
    /// says which.
    pub price_book: Option<PriceBookSection>,
    pub notes: Vec<String>,
}

fn per_mtok_usd(microusd: i64) -> f64 {
    microusd as f64 / 1_000_000.0
}

/// Whether a field name suggests a credential, whatever its value.
fn secret_shaped(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    [
        "secret",
        "token",
        "password",
        "passwd",
        "credential",
        "api_key",
        "apikey",
        "private",
    ]
    .iter()
    .any(|w| k.contains(w))
        || ((k.ends_with("_key") || k.ends_with("key")) && k != "client_keys")
}

/// A reported value as text, bounded. Arrays and objects are summarised,
/// never printed whole.
fn value_text(v: &Value) -> String {
    let text = match v {
        Value::Null => "not set".to_string(),
        Value::Bool(true) => "on".to_string(),
        Value::Bool(false) => "off".to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(a) => format!("{} item(s)", a.len()),
        Value::Object(o) => format!("{} field(s)", o.len()),
    };
    if text.chars().count() > MAX_VALUE_CHARS {
        let cut: String = text.chars().take(MAX_VALUE_CHARS - 1).collect();
        format!("{cut}…")
    } else {
        text
    }
}

/// The settings section from a reported body. A body that is not a JSON
/// object reports nothing this console can read: every known switch then
/// reads "not reported".
pub fn settings_section(body: &Value) -> SettingsSection {
    let empty = serde_json::Map::new();
    let map = body.as_object().unwrap_or(&empty);
    let mut items: Vec<PostureSetting> = KNOWN_SETTINGS
        .iter()
        .map(|(key, label)| PostureSetting {
            key: (*key).to_string(),
            label: (*label).to_string(),
            value: map.get(*key).map(|v| {
                if secret_shaped(key) {
                    WITHHELD.to_string()
                } else {
                    value_text(v)
                }
            }),
        })
        .collect();
    let extras = map
        .iter()
        .filter(|(k, _)| !KNOWN_SETTINGS.iter().any(|(known, _)| known == k))
        .take(MAX_EXTRA_SETTINGS)
        .map(|(k, v)| {
            let key: String = k.chars().take(64).collect();
            PostureSetting {
                label: key.clone(),
                value: Some(if secret_shaped(k) {
                    WITHHELD.to_string()
                } else {
                    value_text(v)
                }),
                key,
            }
        });
    items.extend(extras);
    SettingsSection {
        reported: true,
        items,
    }
}

/// The keys section from `GET /v1/keys`.
pub fn keys_section(report: &GatewayKeysReport) -> KeysSection {
    KeysSection {
        identity_strict: report.strict_mode.clone(),
        identity_map_configured: report.identity_map_configured,
        configured_keys: report.keys.iter().filter(|k| k.configured).count(),
        keys_listed: report.keys.len(),
        unauthorized_attempts: report.unauthorized_since_startup.attempts,
    }
}

/// The price book section from `GET /v1/price-book`.
pub fn price_book_section(book: &GatewayPriceBook) -> PriceBookSection {
    PriceBookSection {
        rows: book.rows,
        built_in_rows: book.built_in_rows,
        operator_file: book.operator_file,
        overridden_models: book.overridden_models.clone(),
        added_models: book.added_models.clone(),
        fallback_input_per_mtok_usd: book
            .fallback
            .map(|f| per_mtok_usd(f.input_per_mtok_microusd)),
        fallback_output_per_mtok_usd: book
            .fallback
            .map(|f| per_mtok_usd(f.output_per_mtok_microusd)),
        tiers: book.long_context.as_ref().map(|rows| {
            rows.iter()
                .map(|r| TierRow {
                    model: r.model.clone(),
                    above_prompt_tokens: r.above_prompt_tokens,
                    input_per_mtok_usd: per_mtok_usd(r.input_per_mtok_microusd),
                    output_per_mtok_usd: per_mtok_usd(r.output_per_mtok_microusd),
                })
                .collect()
        }),
        tiers_dropped_models: book.long_context_dropped_models.clone(),
        fallback_models_seen: book.fallback_models_seen.clone(),
        fallback_models_seen_total: book.fallback_models_seen_total,
    }
}

/// The gateway's posture: settings, keys and identity map, price book.
pub async fn credentials_posture(
    state: &CredentialsState,
) -> Result<GatewayPostureDto, CredentialsError> {
    let client = ready_client(&state).await?;
    let (settings, keys, book) = tokio::join!(
        client.client.get_settings(),
        client.client.get_keys(),
        client.client.get_price_book(),
    );
    let mut notes = Vec::new();
    let mut failures: Vec<GatewayError> = Vec::new();

    let settings = match settings {
        Ok(Some(body)) => Some(settings_section(&body)),
        Ok(None) => Some(SettingsSection {
            reported: false,
            items: Vec::new(),
        }),
        Err(e) => {
            notes.push(format!("GET /v1/settings did not answer: {e}"));
            failures.push(e);
            None
        }
    };
    let keys = match keys {
        Ok(report) => Some(keys_section(&report)),
        Err(e) => {
            notes.push(format!("GET /v1/keys did not answer: {e}"));
            failures.push(e);
            None
        }
    };
    let price_book = match book {
        Ok(Some(b)) => Some(price_book_section(&b)),
        Ok(None) => {
            notes.push(
                "This gateway has no GET /v1/price-book (older than TokenFuse v1.8.0), so the \
                 book it prices with is not reported."
                    .to_string(),
            );
            None
        }
        Err(e) => {
            notes.push(format!("GET /v1/price-book did not answer: {e}"));
            failures.push(e);
            None
        }
    };
    if failures.len() == 3 {
        return Err(CredentialsError::from(failures.remove(0)));
    }
    Ok(GatewayPostureDto {
        gateway_url: client.gateway_url.clone(),
        settings,
        keys,
        price_book,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_value_under_a_secret_shaped_name_is_withheld_and_the_rest_is_shown() {
        let s = settings_section(&json!({
            "firewall": "enforce", "client_keys": true, "admin_key": "s3cr3t-admin",
            "closure_secret": "s3cr3t-closure", "upstream_api_key": "sk-s3cr3t",
            "signing_token": "t0k3n", "max_run_budget_usd": 5.0, "region": "eu"
        }));
        let text = serde_json::to_string(&s).expect("serialize");
        for leaked in ["s3cr3t", "sk-s3cr3t", "t0k3n"] {
            assert!(!text.contains(leaked), "{leaked} reached the view: {text}");
        }
        let get = |k: &str| {
            s.items
                .iter()
                .find(|i| i.key == k)
                .and_then(|i| i.value.clone())
        };
        assert_eq!(get("firewall").as_deref(), Some("enforce"));
        assert_eq!(get("client_keys").as_deref(), Some("on"));
        assert_eq!(get("max_run_budget_usd").as_deref(), Some("5.0"));
        assert_eq!(get("admin_key").as_deref(), Some(WITHHELD));
        assert_eq!(get("region").as_deref(), Some("eu"));
        // A known switch the gateway did not report reads as not reported.
        assert_eq!(get("dlp_pii"), None);
    }

    #[test]
    fn values_are_bounded_and_summarised_never_printed_whole() {
        let long = "x".repeat(500);
        assert_eq!(value_text(&json!(long)).chars().count(), MAX_VALUE_CHARS);
        assert_eq!(value_text(&json!([1, 2, 3])), "3 item(s)");
        assert_eq!(value_text(&json!({"a": 1})), "1 field(s)");
        assert_eq!(value_text(&Value::Null), "not set");
        assert_eq!(value_text(&json!(false)), "off");
        let many: serde_json::Map<String, Value> = (0..100)
            .map(|i| (format!("extra_{i:03}"), json!(i)))
            .collect();
        let s = settings_section(&Value::Object(many));
        assert_eq!(s.items.len(), KNOWN_SETTINGS.len() + MAX_EXTRA_SETTINGS);
        assert!(
            settings_section(&json!("not an object"))
                .items
                .iter()
                .all(|i| i.value.is_none())
        );
    }
}
