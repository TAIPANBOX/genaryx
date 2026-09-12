//! Conformance validation, porting `agent-conform` semantics (07 §1) into the
//! ingest path: full JSON Schema draft 2020-12 against the embedded v0.1, v0.2
//! and v1.0 envelope schemas. The three schema files are byte-exact copies of
//! the open `agent-passport` schemas, vendored under `src/schemas/` and held
//! identical to the canonical files by estate-gates C2.
//!
//! This is the console's first line of defense on the bus. It must catch the same
//! real defects the Go validator does (e.g. a `prev_hash` with 63 hex chars, not 64).
//!
//! One refusal here is a decision rather than a schema rule. Under v1.0 the
//! envelope's `agent_id` may carry a `claimed:` subject (agent-passport SPEC
//! 3.3): an identity the producer read from its own environment and could not
//! attest. This console has no model for a claim, and SPEC 6.4.1 gives such a
//! consumer one duty: refuse the line and count it, never show it as an agent.
//! The refusal lives here so that the ingest path quarantines it under
//! [`CLAIMED_SUBJECT_REFUSED`] and the quarantine panel counts it, in one row.

use crate::event::{AgentEvent, SchemaVersion};
use serde_json::Value;

const SCHEMA_V0_1: &str = include_str!("schemas/agent-event.v0.1.schema.json");
const SCHEMA_V0_2: &str = include_str!("schemas/agent-event.v0.2.schema.json");
const SCHEMA_V1_0: &str = include_str!("schemas/agent-event.v1.0.schema.json");

/// The `agent_id` prefix that marks a claimed subject (agent-passport SPEC 3.3).
pub const CLAIMED_PREFIX: &str = "claimed:";

/// The one reason every refused claimed subject is quarantined under.
///
/// One fixed string on purpose: [`crate::store::Store::quarantine_by_reason`]
/// groups by reason, so every such line lands in one row with a count, which is
/// the "counts" half of SPEC 6.4.1's sentence. The subject itself is in the raw
/// line beside it; putting it in the reason would give a producer that writes
/// claimed subjects one row per event and an operator no total.
pub const CLAIMED_SUBJECT_REFUSED: &str = "claimed subject refused: agent_id carries \
`claimed:` and this console does not model claimed identities (agent-passport SPEC 3.3, 6.4.1)";

/// Compiled validators for the three accepted envelope versions. Build once,
/// reuse per line.
pub struct Conformer {
    v0_1: jsonschema::Validator,
    v0_2: jsonschema::Validator,
    v1_0: jsonschema::Validator,
}

/// Outcome of validating a single line/object.
#[derive(Debug, Clone)]
pub struct ConformReport {
    pub valid: bool,
    /// Resolved from the `schema` field, if recognized.
    pub schema_version: Option<SchemaVersion>,
    /// Joined validator messages when invalid (empty when valid).
    pub errors: Vec<String>,
}

impl ConformReport {
    fn invalid(schema_version: Option<SchemaVersion>, errors: Vec<String>) -> Self {
        Self {
            valid: false,
            schema_version,
            errors,
        }
    }
}

impl Conformer {
    /// Compile the embedded schemas. Fails only if a vendored schema is malformed
    /// (a build-time invariant, exercised by the tests).
    pub fn new() -> Result<Self, String> {
        let v0_1 = compile(SCHEMA_V0_1, "v0.1")?;
        let v0_2 = compile(SCHEMA_V0_2, "v0.2")?;
        let v1_0 = compile(SCHEMA_V1_0, "v1.0")?;
        Ok(Self { v0_1, v0_2, v1_0 })
    }

    /// Validate an already-parsed JSON object.
    pub fn check_value(&self, v: &Value) -> ConformReport {
        // Route to the right schema by the declared `schema` string. An unknown or
        // missing schema is itself a conformance failure (both schemas pin `schema`
        // with `const`), reported explicitly rather than silently skipped.
        let declared = v.get("schema").and_then(Value::as_str);
        let (version, validator) = match declared.and_then(SchemaVersion::from_schema_str) {
            Some(SchemaVersion::V0_1) => (SchemaVersion::V0_1, &self.v0_1),
            Some(SchemaVersion::V0_2) => (SchemaVersion::V0_2, &self.v0_2),
            Some(SchemaVersion::V1_0) => (SchemaVersion::V1_0, &self.v1_0),
            None => {
                let got = declared.unwrap_or("<missing>");
                return ConformReport::invalid(
                    None,
                    vec![format!(
                        "unknown or missing `schema` (got {got:?}); expected {:?}, {:?} or {:?}",
                        SchemaVersion::SCHEMA_V0_1,
                        SchemaVersion::SCHEMA_V0_2,
                        SchemaVersion::SCHEMA_V1_0
                    )],
                );
            }
        };

        if validator.is_valid(v) {
            // The schema admits the form; the console does not model it. After
            // validation on purpose, so the reason names the decision and not a
            // pattern: only v1.0's pattern admits `claimed:` at all, and under
            // v0.1 and v0.2 the pattern has already refused it above.
            if v.get("agent_id")
                .and_then(Value::as_str)
                .is_some_and(|id| id.starts_with(CLAIMED_PREFIX))
            {
                return ConformReport::invalid(
                    Some(version),
                    vec![CLAIMED_SUBJECT_REFUSED.to_string()],
                );
            }
            ConformReport {
                valid: true,
                schema_version: Some(version),
                errors: Vec::new(),
            }
        } else {
            let errors = validator.iter_errors(v).map(|e| e.to_string()).collect();
            ConformReport::invalid(Some(version), errors)
        }
    }

    /// Parse then validate a single NDJSON line. Malformed JSON is a failure, not a panic.
    pub fn check_line(&self, line: &str) -> ConformReport {
        match serde_json::from_str::<Value>(line) {
            Ok(v) => self.check_value(&v),
            Err(e) => ConformReport::invalid(None, vec![format!("malformed json: {e}")]),
        }
    }

    /// Validate and, if valid, deserialize into a typed [`AgentEvent`].
    /// Returns the report on failure so the caller can quarantine with a reason.
    pub fn parse_valid(&self, line: &str) -> Result<AgentEvent, ConformReport> {
        let report = self.check_line(line);
        if !report.valid {
            return Err(report);
        }
        serde_json::from_str::<AgentEvent>(line).map_err(|e| {
            ConformReport::invalid(report.schema_version, vec![format!("decode: {e}")])
        })
    }
}

fn compile(schema_src: &str, tag: &str) -> Result<jsonschema::Validator, String> {
    let schema: Value =
        serde_json::from_str(schema_src).map_err(|e| format!("parse {tag} schema: {e}"))?;
    jsonschema::validator_for(&schema).map_err(|e| format!("compile {tag} schema: {e}"))
}
