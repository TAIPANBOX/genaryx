//! The Keys panel: what Keyryx custody holds and what its witness last signed,
//! read-only (CLAUDE.md invariant 53).
//!
//! Keyryx custody publishes its inventory on two routes anyone may read,
//! `GET /v1/state` (each key name's active and next key id, and the alarms
//! custody could not vouch for) and `GET /v1/keys` (every key id with its
//! algorithm, state and dates, revoked ones included, never a private member).
//! The witness publishes its latest signed tree head at `GET /v1/witness/head`.
//! The console reads those three where the operator names custody
//! ([`CUSTODY_ENV`]) and the witness ([`WITNESS_ENV`]), and shows them.
//!
//! # What it never does
//!
//! It holds no admin key and sends nothing but those three GETs: rotating and
//! revoking a key is custody's own command line, as root, on the box
//! (`@decided 2026-11-23`), and the console has no control for either. It
//! follows no redirect, gives up after [`TIMEOUT`], and reads at most
//! [`MAX_BODY_BYTES`] of an answer. A part not named is "not configured", a
//! part named that did not answer is "unreachable", and neither is shown as an
//! empty inventory.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The setting that names Keyryx custody's base URL.
pub const CUSTODY_ENV: &str = "GENARYX_KEYRYX_URL";
/// The setting that names the Keyryx witness's base URL.
pub const WITNESS_ENV: &str = "GENARYX_KEYRYX_WITNESS_URL";
/// How long one read waits.
pub const TIMEOUT: Duration = Duration::from_secs(5);
/// The most of one answer read.
pub const MAX_BODY_BYTES: usize = 1 << 20;
/// The most key entries passed on.
pub const MAX_ENTRIES: usize = 500;
/// The most alarms passed on.
pub const MAX_ALARMS: usize = 100;

/// Where the panel reads, resolved once per call from the environment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeysConfig {
    pub custody: Option<String>,
    pub witness: Option<String>,
}

/// A base URL from a setting: blank is none; anything else must be `http://`
/// or `https://`, and a trailing slash is dropped.
pub fn base_url_from(raw: &str) -> Result<Option<String>, String> {
    let t = raw.trim();
    if t.is_empty() {
        return Ok(None);
    }
    if !(t.starts_with("http://") || t.starts_with("https://")) {
        return Err("must start with http:// or https://".to_string());
    }
    Ok(Some(t.trim_end_matches('/').to_string()))
}

impl KeysConfig {
    pub fn from_env() -> (Self, Vec<(String, String)>) {
        let mut refused = Vec::new();
        let mut read = |var: &str| match base_url_from(&std::env::var(var).unwrap_or_default()) {
            Ok(v) => v,
            Err(e) => {
                refused.push((var.to_string(), e));
                None
            }
        };
        let cfg = KeysConfig {
            custody: read(CUSTODY_ENV),
            witness: read(WITNESS_ENV),
        };
        (cfg, refused)
    }
}

// ---- what Keyryx sends (keyryx internal/api and internal/wlog) ----

#[derive(Debug, Clone, Deserialize)]
struct StateBody {
    #[serde(default)]
    version: String,
    #[serde(default)]
    keys: Vec<KeyStateBody>,
    #[serde(default)]
    alarms: Vec<AlarmBody>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AlarmBody {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kid: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
struct KeysBody {
    #[serde(default)]
    keys: Vec<EntryBody>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct EntryBody {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub alg: String,
    #[serde(default)]
    pub kid: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub created: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activate_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retired: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_after: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct SigBody {
    #[serde(default)]
    pub kid: String,
}

#[derive(Debug, Clone, Deserialize)]
struct HeadBody {
    size: i64,
    root: String,
    at: i64,
    #[serde(default)]
    es384: Option<SigBody>,
    #[serde(default)]
    ml_dsa_65: Option<SigBody>,
}

// ---- what the panel answers ----

/// One key name, from custody's state and inventory together.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct KeyName {
    pub name: String,
    pub alg: String,
    /// `held` (custody signs with it) or `delegated` (custody certifies a key
    /// its consumer holds).
    pub mode: String,
    pub active_kid: Option<String>,
    pub active_since: Option<i64>,
    pub next_kid: Option<String>,
    pub next_activates_at: Option<i64>,
    /// When the newest key of this name was made: the last rotation, or the
    /// first key when it never rotated. `None` when the inventory lists none.
    pub last_rotation: Option<i64>,
    /// Inventory entries of this name by state.
    pub states: BTreeMap<String, usize>,
    pub renew_due: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CustodyPart {
    NotConfigured {
        variable: String,
    },
    Refused {
        variable: String,
        reason: String,
    },
    Unreachable {
        url: String,
        reason: String,
    },
    Read {
        url: String,
        version: String,
        names: Vec<KeyName>,
        alarms: Vec<AlarmBody>,
        alarms_total: usize,
        entries: Vec<EntryBody>,
        entries_total: usize,
        /// The inventory route did not answer, though the state route did.
        inventory_error: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WitnessPart {
    NotConfigured {
        variable: String,
    },
    Refused {
        variable: String,
        reason: String,
    },
    Unreachable {
        url: String,
        reason: String,
    },
    /// The witness answered 404: its log has no tree head yet.
    NoHeadYet {
        url: String,
    },
    Read {
        url: String,
        size: i64,
        root: String,
        at: i64,
        es384_kid: Option<String>,
        ml_dsa_65_kid: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct KeysPanel {
    pub custody: CustodyPart,
    pub witness: WitnessPart,
}

fn bounded(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push_str("...");
        t
    }
}

/// GET `url`, answering the status and the body read capped at
/// [`MAX_BODY_BYTES`], or why no answer came.
async fn get_capped(url: &str) -> Result<(u16, Vec<u8>), String> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let mut resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        if buf.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(format!(
                "the answer is longer than {MAX_BODY_BYTES} bytes; not read further"
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok((status, buf))
}

async fn get_json<T: for<'de> Deserialize<'de>>(url: &str) -> Result<T, String> {
    let (status, body) = get_capped(url).await?;
    if status != 200 {
        return Err(format!("answered {status}"));
    }
    serde_json::from_slice(&body).map_err(|e| format!("not the expected answer: {e}"))
}

/// Fold custody's state and inventory into the panel's names.
pub fn fold_custody(
    url: String,
    version: String,
    states: Vec<KeyStateBody>,
    alarms: Vec<AlarmBody>,
    entries: Option<Vec<EntryBody>>,
    inventory_error: Option<String>,
) -> CustodyPart {
    let entries_all = entries.unwrap_or_default();
    let mut names: Vec<KeyName> = states
        .into_iter()
        .map(|s| {
            let mine: Vec<&EntryBody> = entries_all.iter().filter(|e| e.name == s.name).collect();
            let mut by_state: BTreeMap<String, usize> = BTreeMap::new();
            for e in &mine {
                *by_state.entry(bounded(&e.state, 32)).or_default() += 1;
            }
            KeyName {
                name: bounded(&s.name, 128),
                alg: bounded(&s.alg, 32),
                mode: bounded(&s.mode, 32),
                active_kid: s.active_kid.map(|k| bounded(&k, 128)),
                active_since: s.active_since,
                next_kid: s.next_kid.map(|k| bounded(&k, 128)),
                next_activates_at: s.next_activates_at,
                last_rotation: mine.iter().map(|e| e.created).filter(|c| *c > 0).max(),
                states: by_state,
                renew_due: s.renew_due.unwrap_or(false),
            }
        })
        .collect();
    names.sort_by(|a, b| a.name.cmp(&b.name));
    let alarms_total = alarms.len();
    let alarms = alarms
        .into_iter()
        .take(MAX_ALARMS)
        .map(|a| AlarmBody {
            kind: bounded(&a.kind, 64),
            name: bounded(&a.name, 128),
            kid: bounded(&a.kid, 128),
            reason: bounded(&a.reason, 512),
        })
        .collect();
    let entries_total = entries_all.len();
    let mut entries: Vec<EntryBody> = entries_all;
    // Newest first, so a cut keeps what changed last.
    entries.sort_by(|a, b| b.created.cmp(&a.created).then(a.kid.cmp(&b.kid)));
    entries.truncate(MAX_ENTRIES);
    for e in &mut entries {
        e.name = bounded(&e.name, 128);
        e.kid = bounded(&e.kid, 128);
        e.alg = bounded(&e.alg, 32);
        e.mode = bounded(&e.mode, 32);
        e.state = bounded(&e.state, 32);
        e.owner = e.owner.as_deref().map(|o| bounded(o, 128));
    }
    CustodyPart::Read {
        url,
        version: bounded(&version, 64),
        names,
        alarms,
        alarms_total,
        entries,
        entries_total,
        inventory_error,
    }
}

/// One key name in custody's `GET /v1/state`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct KeyStateBody {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub alg: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub active_kid: Option<String>,
    #[serde(default)]
    pub active_since: Option<i64>,
    #[serde(default)]
    pub next_kid: Option<String>,
    #[serde(default)]
    pub next_activates_at: Option<i64>,
    #[serde(default)]
    pub renew_due: Option<bool>,
}

async fn read_custody(base: &str) -> CustodyPart {
    let state: StateBody = match get_json(&format!("{base}/v1/state")).await {
        Ok(s) => s,
        Err(reason) => {
            return CustodyPart::Unreachable {
                url: base.to_string(),
                reason: bounded(&reason, 512),
            };
        }
    };
    let (entries, inventory_error) = match get_json::<KeysBody>(&format!("{base}/v1/keys")).await {
        Ok(k) => (Some(k.keys), None),
        Err(e) => (None, Some(bounded(&e, 512))),
    };
    fold_custody(
        base.to_string(),
        state.version,
        state.keys,
        state.alarms,
        entries,
        inventory_error,
    )
}

async fn read_witness(base: &str) -> WitnessPart {
    let url = format!("{base}/v1/witness/head");
    match get_capped(&url).await {
        Ok((404, _)) => WitnessPart::NoHeadYet {
            url: base.to_string(),
        },
        Ok((200, body)) => match serde_json::from_slice::<HeadBody>(&body) {
            Ok(h) => WitnessPart::Read {
                url: base.to_string(),
                size: h.size,
                root: bounded(&h.root, 128),
                at: h.at,
                es384_kid: h.es384.map(|s| bounded(&s.kid, 128)),
                ml_dsa_65_kid: h.ml_dsa_65.map(|s| bounded(&s.kid, 128)),
            },
            Err(e) => WitnessPart::Unreachable {
                url: base.to_string(),
                reason: bounded(&format!("not the expected answer: {e}"), 512),
            },
        },
        Ok((status, _)) => WitnessPart::Unreachable {
            url: base.to_string(),
            reason: format!("answered {status}"),
        },
        Err(reason) => WitnessPart::Unreachable {
            url: base.to_string(),
            reason: bounded(&reason, 512),
        },
    }
}

/// Read the panel: custody and the witness, each on its own.
pub async fn keys_custody(cfg: &KeysConfig, refused: &[(String, String)]) -> KeysPanel {
    let refusal = |var: &str| {
        refused
            .iter()
            .find(|(v, _)| v == var)
            .map(|(_, r)| r.clone())
    };
    let custody = match (&cfg.custody, refusal(CUSTODY_ENV)) {
        (_, Some(reason)) => CustodyPart::Refused {
            variable: CUSTODY_ENV.to_string(),
            reason,
        },
        (Some(base), None) => read_custody(base).await,
        (None, None) => CustodyPart::NotConfigured {
            variable: CUSTODY_ENV.to_string(),
        },
    };
    let witness = match (&cfg.witness, refusal(WITNESS_ENV)) {
        (_, Some(reason)) => WitnessPart::Refused {
            variable: WITNESS_ENV.to_string(),
            reason,
        },
        (Some(base), None) => read_witness(base).await,
        (None, None) => WitnessPart::NotConfigured {
            variable: WITNESS_ENV.to_string(),
        },
    };
    KeysPanel { custody, witness }
}

#[cfg(test)]
mod tests;
