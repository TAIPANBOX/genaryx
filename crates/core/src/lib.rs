//! genaryx-core: all Genaryx logic lives here; every shell stays thin - the
//! web shell today, the SwiftUI and Tauri shells before the desktop apps were
//! removed.
//!
//! Design principle 06 §0.9 (one core, two shells): every feature is implemented
//! once in this crate and merely rendered by the shells. Shells collect intents;
//! they contain no domain logic.
//!
//! Phase 0 status: [`event`] and [`conform`] are implemented (the heart of the
//! ingest path). [`store`], [`ingest`], and [`demo`] are scaffolded stubs with
//! stable signatures, delegated to Sonnet tracks (see `../../docs/PHASE0.md`).

pub mod bus;
pub mod command;
pub mod conform;
pub mod demo;
pub mod error;
pub mod event;
pub mod evidence;
pub mod graph;
pub mod ingest;
pub mod layout;
pub mod store;
pub mod taipan_home;

pub use bus::{ResolvedBus, discover as discover_bus};
pub use command::{CommandRecord, console_command_line, record};
pub use conform::{ConformReport, Conformer};
pub use error::{Error, Result};
pub use event::{AgentEvent, ConsoleEvent, Provenance, SchemaVersion, Severity};
pub use graph::{AgentSlice, DelegationGraph, GraphEdge, GraphNode, GraphView, NodeKind};
pub use ingest::{EventSource, FileTail, IngestService, IngestStats, RawRecord};
pub use layout::{LayoutConfig, LayoutView, PositionedNode, layout, layout_view};
pub use store::{DelegationRow, StoredEvent};
