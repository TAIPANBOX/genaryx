//! The agent-event envelope (the shared contract, 07 §1) plus the console's
//! normalized wrapper.
//!
//! `AgentEvent` mirrors the on-the-wire NDJSON object and is deliberately
//! **tolerant**: unknown top-level keys are preserved (`additionalProperties: true`)
//! and `severity` is kept as a raw string so a future enum value never makes a
//! whole line unparseable. Structural validity is decided by [`crate::conform`],
//! not by serde. `ConsoleEvent` adds provenance and the original raw line so every
//! byte we display can show its source (06 §0.8).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Schema version of the envelope. v0.1 has a closed `source` enum
/// (`tokenfuse|engram|idryx|qryx`); v0.2 opens `source` to any string (07 §1);
/// v1.0 is the contract at agent-passport 1.0 (SPEC 6.4.1, 2026-09-12): v0.2's
/// shape with the version string changed and one widening, `agent_id` may carry
/// a `claimed:` subject (SPEC 3.3). This console does not model a claimed
/// subject, so the conformer refuses such a line and the quarantine counts it,
/// which is the one duty SPEC 6.4.1 leaves a consumer that has no model for it.
///
/// v0.3, the interim version where `claimed:` first appeared, stays
/// unrecognized on purpose: SPEC 6.4 lets a consumer refuse it, and this one
/// does, by version. From 1.0 a consumer MUST accept v0.1, v0.2 and v1.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SchemaVersion {
    V0_1,
    V0_2,
    V1_0,
}

impl SchemaVersion {
    pub const SCHEMA_V0_1: &'static str = "taipanbox.dev/agent-event/v0.1";
    pub const SCHEMA_V0_2: &'static str = "taipanbox.dev/agent-event/v0.2";
    pub const SCHEMA_V1_0: &'static str = "taipanbox.dev/agent-event/v1.0";

    pub fn from_schema_str(s: &str) -> Option<Self> {
        match s {
            Self::SCHEMA_V0_1 => Some(Self::V0_1),
            Self::SCHEMA_V0_2 => Some(Self::V0_2),
            Self::SCHEMA_V1_0 => Some(Self::V1_0),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::V0_1 => Self::SCHEMA_V0_1,
            Self::V0_2 => Self::SCHEMA_V0_2,
            Self::V1_0 => Self::SCHEMA_V1_0,
        }
    }
}

/// Severity ladder (typed view). The raw envelope keeps severity as a string;
/// callers parse to this for UI/alerting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "info" => Some(Self::Info),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

/// The agent-event envelope as it appears on the bus. Tolerant by design.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    pub schema: String,
    pub ts: String,
    pub source: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub agent_id: String,

    /// Kept as a raw string for forward-compatibility; use [`AgentEvent::severity_typed`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_behalf_of: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,

    /// Any additional top-level keys, preserved verbatim (`additionalProperties: true`).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl AgentEvent {
    /// Typed severity if present and recognized, else `None`.
    pub fn severity_typed(&self) -> Option<Severity> {
        self.severity.as_deref().and_then(Severity::parse)
    }

    /// The declared schema version, if it is one we recognize.
    pub fn schema_version(&self) -> Option<SchemaVersion> {
        SchemaVersion::from_schema_str(&self.schema)
    }
}

/// Where a given event came from. Every normalized event carries this so the UI
/// can always show provenance and the raw line (06 §0.8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    /// Environment id (e.g. "local", "hetzner-demo").
    pub env: String,
    /// Connector id (e.g. "filetail:tokenfuse", "cloud-sse").
    pub connector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// When the console received/ingested the line (RFC 3339).
    pub received_ts: String,
}

/// A normalized event ready for the Store and shells: the envelope, its
/// provenance, the original raw NDJSON line, and the resolved schema version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleEvent {
    pub event: AgentEvent,
    pub provenance: Provenance,
    pub raw: String,
    pub schema_version: SchemaVersion,
}

// Manual (de)serialization for SchemaVersion as its canonical schema string,
// so ConsoleEvent round-trips cleanly.
impl Serialize for SchemaVersion {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SchemaVersion {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        SchemaVersion::from_schema_str(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown schema version: {s}")))
    }
}
