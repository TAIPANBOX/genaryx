//! Wire DTOs and error taxonomy for [`super::MemoryHandle`], mirroring
//! `crates/ffi/src/crypto/dto.rs`'s shape (UniFFI `Record`/`Error` types
//! instead of `genaryx_connectors`' plain Rust structs) but over the Engram
//! contract (docs/PHASE4.md W2, `crates/connectors/src/engram.rs`'s own doc
//! comment).
//!
//! Every Record here keeps the `Engram` prefix (unlike, say, Quality's bare
//! `ScoreRecord`/`BaselineRecord`): `Stats`/`Memory`/`Provenance` alone would
//! be far too generic in a flat, six-plane UniFFI namespace, and `Memory`
//! specifically would collide in spirit with [`super::MemoryHandle`] itself
//! (the panel/plane name) - see `crypto/dto.rs`'s own module doc for the same
//! judgment call applied to `Ncsc`/`Evidence`.
//!
//! ## `EngramProvenance` crosses FFI as a fielded `uniffi::Enum`
//!
//! [`genaryx_connectors::EngramProvenance`] is a `#[serde(tag = "kind")]`
//! two-variant enum (`Semantic` / `Episodic`, each with its own field set,
//! discriminated on `why`'s `kind` wire field). UniFFI's `Enum` derive
//! supports variants carrying named fields exactly like a Rust enum does, so
//! [`EngramProvenanceRecord`] mirrors it field-for-field rather than
//! flattening to one record with a `kind` string plus two all-optional
//! sub-records: the enum shape lets the Swift panel `switch` exhaustively
//! (`.semantic(...)` / `.episodic(...)`) with every field non-optional on the
//! branch it actually took, instead of unwrapping a bag of `Option`s by hand.

use genaryx_connectors::{
    EngramCounts as ConnEngramCounts, EngramForgetResult as ConnEngramForgetResult,
    EngramMemory as ConnEngramMemory, EngramProvenance as ConnEngramProvenance,
    EngramStats as ConnEngramStats, McpError as ConnMcpError,
};

// ============================================================================
// DTOs: stats
// ============================================================================

/// Per-kind memory counts: exact field set of
/// `genaryx_connectors::EngramCounts`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EngramCountsRecord {
    pub episodic: i64,
    pub semantic: i64,
    /// ALWAYS `0` in this Engram version - the store implements only
    /// episodic + semantic (`genaryx_connectors::EngramCounts::procedural`'s
    /// own doc: "`mcp_server.py:302-304` says so verbatim"). Carried through
    /// verbatim, never inflated or hidden; the Swift panel MUST label this
    /// "not implemented in this Engram version", never a real zero
    /// (docs/PHASE4.md W2 guard), mirroring
    /// `crate::crypto::dto::NcscPriorityRecord::migrated_count`'s own
    /// verbatim-never-real-progress contract.
    pub procedural: i64,
}

impl From<&ConnEngramCounts> for EngramCountsRecord {
    fn from(c: &ConnEngramCounts) -> Self {
        Self {
            episodic: c.episodic,
            semantic: c.semantic,
            procedural: c.procedural,
        }
    }
}

/// `stats`: store-wide counts for the effective agent scope - exact field set
/// of `genaryx_connectors::EngramStats`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EngramStatsRecord {
    pub agent_id: Option<String>,
    pub counts: EngramCountsRecord,
    pub vector_index_size: i64,
    pub facts_total: i64,
    pub facts_active: i64,
    pub facts_superseded: i64,
    pub entities: i64,
    pub reflections: i64,
    pub db_path: String,
    /// `None` for an in-memory store or a file that does not exist yet - the
    /// Swift panel renders this as "in-memory / n/a", never a fabricated 0
    /// (docs/PHASE4.md W2 guard).
    pub db_size_bytes: Option<i64>,
}

impl From<&ConnEngramStats> for EngramStatsRecord {
    fn from(s: &ConnEngramStats) -> Self {
        Self {
            agent_id: s.agent_id.clone(),
            counts: EngramCountsRecord::from(&s.counts),
            vector_index_size: s.vector_index_size,
            facts_total: s.facts_total,
            facts_active: s.facts_active,
            facts_superseded: s.facts_superseded,
            entities: s.entities,
            reflections: s.reflections,
            db_path: s.db_path.clone(),
            db_size_bytes: s.db_size_bytes,
        }
    }
}

// ============================================================================
// DTOs: recall
// ============================================================================

/// One `recall` hit, ranked by relevance (most relevant first) - exact field
/// set of `genaryx_connectors::EngramMemory`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EngramMemoryRecord {
    pub id: String,
    pub content: String,
    pub score: f64,
    pub importance: f64,
    pub timestamp: String,
    pub actors: Vec<String>,
    pub tags: Vec<String>,
}

impl From<&ConnEngramMemory> for EngramMemoryRecord {
    fn from(m: &ConnEngramMemory) -> Self {
        Self {
            id: m.id.clone(),
            content: m.content.clone(),
            score: m.score,
            importance: m.importance,
            timestamp: m.timestamp.clone(),
            actors: m.actors.clone(),
            tags: m.tags.clone(),
        }
    }
}

// ============================================================================
// DTOs: why (provenance)
// ============================================================================

/// `why`'s two-shape provenance answer - exact field set of
/// `genaryx_connectors::EngramProvenance`. See the module doc for why this is
/// a fielded enum rather than one flattened, all-optional record.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum EngramProvenanceRecord {
    /// An LLM-derived fact triple with its full extraction lineage.
    Semantic {
        id: String,
        subject: String,
        predicate: String,
        object: String,
        confidence: f64,
        valid_from: String,
        valid_to: Option<String>,
        recorded_at: String,
        extracted_from: Option<String>,
        extracted_by_reflection_run: Option<String>,
        extraction_model: Option<String>,
    },
    /// A raw observation: no extraction chain, so this carries encoding +
    /// access metadata instead.
    Episodic {
        id: String,
        content: String,
        timestamp: String,
        actors: Vec<String>,
        tags: Vec<String>,
        salience: Option<f64>,
        emotional_valence: Option<f64>,
        importance_score: Option<f64>,
        /// The episode ids this episode summarizes - a list on the wire
        /// (`Episode.summary_of: list[str]`), not a scalar.
        summary_of: Vec<String>,
        agent_id: Option<String>,
        access_count: i64,
        last_accessed: Option<String>,
        note: String,
    },
}

impl From<&ConnEngramProvenance> for EngramProvenanceRecord {
    fn from(p: &ConnEngramProvenance) -> Self {
        match p {
            ConnEngramProvenance::Semantic {
                id,
                subject,
                predicate,
                object,
                confidence,
                valid_from,
                valid_to,
                recorded_at,
                extracted_from,
                extracted_by_reflection_run,
                extraction_model,
            } => Self::Semantic {
                id: id.clone(),
                subject: subject.clone(),
                predicate: predicate.clone(),
                object: object.clone(),
                confidence: *confidence,
                valid_from: valid_from.clone(),
                valid_to: valid_to.clone(),
                recorded_at: recorded_at.clone(),
                extracted_from: extracted_from.clone(),
                extracted_by_reflection_run: extracted_by_reflection_run.clone(),
                extraction_model: extraction_model.clone(),
            },
            ConnEngramProvenance::Episodic {
                id,
                content,
                timestamp,
                actors,
                tags,
                salience,
                emotional_valence,
                importance_score,
                summary_of,
                agent_id,
                access_count,
                last_accessed,
                note,
            } => Self::Episodic {
                id: id.clone(),
                content: content.clone(),
                timestamp: timestamp.clone(),
                actors: actors.clone(),
                tags: tags.clone(),
                salience: *salience,
                emotional_valence: *emotional_valence,
                importance_score: *importance_score,
                summary_of: summary_of.clone(),
                agent_id: agent_id.clone(),
                access_count: *access_count,
                last_accessed: last_accessed.clone(),
                note: note.clone(),
            },
        }
    }
}

// ============================================================================
// DTOs: forget
// ============================================================================

/// The result of [`super::MemoryHandle::forget`] - exact field set of
/// `genaryx_connectors::EngramForgetResult`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EngramForgetResultRecord {
    pub id: String,
    /// `episodic` or `semantic` - which store the id was found in.
    pub kind: String,
    pub deleted: bool,
}

impl From<&ConnEngramForgetResult> for EngramForgetResultRecord {
    fn from(f: &ConnEngramForgetResult) -> Self {
        Self {
            id: f.id.clone(),
            kind: f.kind.clone(),
            deleted: f.deleted,
        }
    }
}

// ============================================================================
// error taxonomy
// ============================================================================

/// Every failure mode a [`super::MemoryHandle`] call can surface, fail-closed
/// throughout (06 §0.5). Collapsed from `genaryx_connectors::McpError`'s six
/// variants, plus [`Self::NoEnvironment`] - an ffi-layer-only addition with no
/// connector-level equivalent, exactly like `CryptoError::NoEnvironment`.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MemoryError {
    /// [`super::env::discover_bin`] found no `engram-mcp` binary anywhere it
    /// knows to look - a normal, renderable "no memory plane" outcome
    /// (docs/PHASE4.md W2: "honest empty state when no engram-mcp/db"), not a
    /// bug.
    #[error(
        "no memory plane found (no engram-mcp on PATH, in a venv, or at ~/.taipan/bin/engram-mcp)"
    )]
    NoEnvironment,
    /// `engram-mcp` could not be spawned - missing, not executable, or (since
    /// this handle spawns AT CONSTRUCTION, unlike every sibling handle - see
    /// `super`'s module doc) a bad db path the process could not open either.
    #[error("could not start engram-mcp: {reason}")]
    Spawn { reason: String },
    /// Writing to or reading from the child's stdio failed, usually because
    /// it has already exited.
    #[error("engram-mcp io error: {reason}")]
    Io { reason: String },
    /// Malformed MCP framing - a stdout line was not valid JSON, or a
    /// response was missing a field the protocol requires.
    #[error("engram-mcp protocol error: {reason}")]
    Protocol { reason: String },
    /// A JSON-RPC `error` object (protocol-level, not a tool failure).
    #[error("engram-mcp rpc error {code}: {message}")]
    Rpc { code: i64, message: String },
    /// A `tools/call` ran and reported `isError: true` - e.g. `why`/`forget`
    /// on an unknown memory id. Never collapsed into an empty success.
    #[error("engram-mcp tool error: {message}")]
    Tool { message: String },
    /// No answer within the deadline, or the server's stdout closed first.
    #[error("engram-mcp timed out after {seconds}s")]
    Timeout { seconds: f64 },
}

impl From<ConnMcpError> for MemoryError {
    fn from(e: ConnMcpError) -> Self {
        match e {
            ConnMcpError::Spawn { command, source } => MemoryError::Spawn {
                reason: format!("{command}: {source}"),
            },
            ConnMcpError::Io(source) => MemoryError::Io {
                reason: source.to_string(),
            },
            ConnMcpError::Protocol(reason) => MemoryError::Protocol { reason },
            ConnMcpError::Rpc { code, message } => MemoryError::Rpc { code, message },
            ConnMcpError::Tool(message) => MemoryError::Tool { message },
            ConnMcpError::Timeout(duration) => MemoryError::Timeout {
                seconds: duration.as_secs_f64(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engram_counts_record_carries_procedural_zero_verbatim() {
        let conn = ConnEngramCounts {
            episodic: 5,
            semantic: 2,
            procedural: 0,
        };
        let record = EngramCountsRecord::from(&conn);
        assert_eq!(record.episodic, 5);
        assert_eq!(record.semantic, 2);
        assert_eq!(record.procedural, 0);
    }

    #[test]
    fn engram_stats_record_keeps_db_size_bytes_as_a_real_option() {
        let conn = ConnEngramStats {
            agent_id: None,
            counts: ConnEngramCounts {
                episodic: 1,
                semantic: 0,
                procedural: 0,
            },
            vector_index_size: 1,
            facts_total: 0,
            facts_active: 0,
            facts_superseded: 0,
            entities: 0,
            reflections: 0,
            db_path: ":memory:".to_string(),
            db_size_bytes: None,
        };
        let record = EngramStatsRecord::from(&conn);
        assert!(record.agent_id.is_none());
        assert!(record.db_size_bytes.is_none());
        assert_eq!(record.db_path, ":memory:");
    }

    #[test]
    fn engram_memory_record_mirrors_every_field() {
        let conn = ConnEngramMemory {
            id: "m1".to_string(),
            content: "paid invoice".to_string(),
            score: 0.91,
            importance: 0.5,
            timestamp: "2026-07-17T10:00:00+00:00".to_string(),
            actors: vec!["alice".to_string()],
            tags: vec!["billing".to_string()],
        };
        let record = EngramMemoryRecord::from(&conn);
        assert_eq!(record.id, "m1");
        assert_eq!(record.score, 0.91);
        assert_eq!(record.tags, vec!["billing".to_string()]);
    }

    #[test]
    fn engram_provenance_record_discriminates_semantic_and_episodic() {
        let semantic = ConnEngramProvenance::Semantic {
            id: "f1".to_string(),
            subject: "acme".to_string(),
            predicate: "owes".to_string(),
            object: "$100".to_string(),
            confidence: 0.9,
            valid_from: "2026-01-01T00:00:00+00:00".to_string(),
            valid_to: None,
            recorded_at: "2026-07-17T10:00:00+00:00".to_string(),
            extracted_from: Some("ep-3".to_string()),
            extracted_by_reflection_run: None,
            extraction_model: Some("claude-sonnet-5".to_string()),
        };
        match EngramProvenanceRecord::from(&semantic) {
            EngramProvenanceRecord::Semantic {
                subject,
                extraction_model,
                ..
            } => {
                assert_eq!(subject, "acme");
                assert_eq!(extraction_model.as_deref(), Some("claude-sonnet-5"));
            }
            other => panic!("expected Semantic, got {other:?}"),
        }

        let episodic = ConnEngramProvenance::Episodic {
            id: "e1".to_string(),
            content: "observed a login".to_string(),
            timestamp: "2026-07-17T10:00:00+00:00".to_string(),
            actors: vec!["bot".to_string()],
            tags: vec![],
            salience: Some(0.4),
            emotional_valence: None,
            importance_score: None,
            summary_of: vec!["ep-1".to_string()],
            agent_id: None,
            access_count: 3,
            last_accessed: None,
            note: "raw observation".to_string(),
        };
        match EngramProvenanceRecord::from(&episodic) {
            EngramProvenanceRecord::Episodic {
                access_count,
                content,
                summary_of,
                ..
            } => {
                assert_eq!(access_count, 3);
                assert_eq!(summary_of, vec!["ep-1".to_string()]);
                assert_eq!(content, "observed a login");
            }
            other => panic!("expected Episodic, got {other:?}"),
        }
    }

    #[test]
    fn engram_forget_result_record_mirrors_every_field() {
        let conn = ConnEngramForgetResult {
            id: "m1".to_string(),
            kind: "episodic".to_string(),
            deleted: true,
        };
        let record = EngramForgetResultRecord::from(&conn);
        assert_eq!(record.id, "m1");
        assert_eq!(record.kind, "episodic");
        assert!(record.deleted);
    }

    #[test]
    fn mcp_tool_error_maps_to_memory_error_tool_with_message() {
        let err = ConnMcpError::Tool("memory not found: 'x'".to_string());
        match MemoryError::from(err) {
            MemoryError::Tool { message } => assert!(message.contains("memory not found")),
            other => panic!("expected MemoryError::Tool, got {other:?}"),
        }
    }

    #[test]
    fn mcp_rpc_error_maps_to_memory_error_rpc_with_code_and_message() {
        let err = ConnMcpError::Rpc {
            code: -32601,
            message: "method not found".to_string(),
        };
        match MemoryError::from(err) {
            MemoryError::Rpc { code, message } => {
                assert_eq!(code, -32601);
                assert_eq!(message, "method not found");
            }
            other => panic!("expected MemoryError::Rpc, got {other:?}"),
        }
    }
}
