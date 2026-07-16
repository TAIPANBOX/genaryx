//! genaryx-core: all Genaryx logic lives here; the SwiftUI and Tauri shells are thin.
//!
//! Design principle 06 §0.9 (one core, two shells): every feature is implemented
//! once in this crate and merely rendered by the shells. Shells collect intents;
//! they contain no domain logic.
//!
//! Phase 0 status: [`event`] and [`conform`] are implemented (the heart of the
//! ingest path). [`store`], [`ingest`], and [`demo`] are scaffolded stubs with
//! stable signatures, delegated to Sonnet tracks (see `../../docs/PHASE0.md`).

pub mod conform;
pub mod demo;
pub mod error;
pub mod event;
pub mod ingest;
pub mod store;

pub use conform::{ConformReport, Conformer};
pub use error::{Error, Result};
pub use event::{AgentEvent, ConsoleEvent, Provenance, SchemaVersion, Severity};
pub use ingest::{EventSource, FileTail, IngestService, IngestStats, RawRecord};
pub use store::StoredEvent;
