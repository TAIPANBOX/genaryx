//! The live `on_behalf_of` delegation graph, built in core from the bus.
//!
//! Architecture position (PHASE3 §5.1, decided): the live delegation graph is
//! genaryx-core's, NOT Idryx's. `idryx serve` is a load-once immutable snapshot
//! (proven from its source: no file-watch / SIGHUP / reload route / poll / TTL),
//! so a live console cannot stand on it. Every agent-event on the bus carries an
//! `agent_id` and an optional `on_behalf_of` chain (root-first, `user://` or
//! `agent://` URIs, max depth 32 per the envelope schema); this module folds
//! those into a directed "delegates_to" graph incrementally as events arrive,
//! and can also batch-build from the Store for the initial load.
//!
//! ## Dedup (the class-1 review guard: key on the FULL set of defining fields)
//!
//! The bus is append-only and a re-tail (or a batch build over a Store that a
//! live session also fed) can present the same event twice. Nodes and edges are
//! sets, so the graph *structure* is idempotent by construction. Per-node
//! `event_count` is NOT idempotent, so it is guarded by a `seen` set keyed on
//! the full natural key `(agent_id, ts, source, type, run_id)` - the same
//! "decision from the complete set of defining fields" rule the stack's own
//! recurring bug class demands (idryx itself had to add exactly this). A second
//! sighting of an identical event re-asserts nodes/edges (a no-op) and does not
//! double-count.
//!
//! This is fail-closed and panic-free: a malformed chain element (an empty
//! string, or a self-reference) is skipped, never added as a bogus node or a
//! self-edge; nothing here can `unwrap`/`panic`.

use crate::error::Result;
use crate::event::AgentEvent;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// What kind of principal a node is, inferred from its URI scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    /// `user://…` - a human principal (only ever a root, never an actor).
    User,
    /// `agent://…` - an agent principal.
    Agent,
    /// Anything else (kept, not dropped, so an unexpected scheme is still visible).
    Other,
}

impl NodeKind {
    fn of(id: &str) -> Self {
        if let Some(scheme) = id.split_once("://").map(|(s, _)| s) {
            match scheme {
                "user" => Self::User,
                "agent" => Self::Agent,
                _ => Self::Other,
            }
        } else {
            Self::Other
        }
    }
}

/// One node in the delegation graph: a principal URI plus how often it acted
/// (as the event's `agent_id`) and when it was last seen acting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub kind: NodeKind,
    /// Events where this node was the acting `agent_id`, deduped on the natural
    /// key. A pure delegator that never itself acted stays at 0.
    pub event_count: u64,
    /// The most recent `ts` this node was seen acting (lexical max over RFC 3339
    /// strings; `""` for a node only ever seen inside another's chain).
    pub last_ts: String,
}

/// One directed "delegates_to" edge: `from` acted on behalf of `to` one step
/// down the chain (and the last chain element delegates to the acting agent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
}

/// A serializable snapshot of the graph, the shape that crosses to the shells
/// (Tauri IPC / SwiftUI FFI) and feeds the layout engine.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphView {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// Natural key identifying one event, so a re-tail cannot double-count. The
/// FULL set of defining fields, not a subset (the recurring stack bug class).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct EventKey {
    agent_id: String,
    ts: String,
    source: String,
    event_type: String,
    run_id: String,
}

/// The live delegation graph. Feed it with [`DelegationGraph::add_event`] as
/// bus events arrive, or batch-build with [`DelegationGraph::from_store`].
#[derive(Debug, Default)]
pub struct DelegationGraph {
    nodes: BTreeMap<String, GraphNode>,
    edges: BTreeSet<(String, String)>,
    seen: BTreeSet<EventKey>,
}

impl DelegationGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Batch-build from every event in the Store (oldest-first). Idempotent
    /// with the live path: the shared `seen` dedup means feeding the same event
    /// through both `from_store` and later `add_event` counts it once.
    pub fn from_store(store: &Store) -> Result<Self> {
        let mut g = Self::new();
        for row in store.delegation_events()? {
            let key = EventKey {
                agent_id: row.agent_id.clone(),
                ts: row.ts.clone(),
                source: row.source,
                event_type: row.type_,
                run_id: row.run_id.unwrap_or_default(),
            };
            g.ingest(&row.agent_id, &row.on_behalf_of, key, &row.ts);
        }
        Ok(g)
    }

    /// Fold one bus event into the graph. Returns `true` if this was a newly
    /// counted event (its natural key had not been seen), `false` if it was a
    /// duplicate (structure re-asserted, count not bumped).
    pub fn add_event(&mut self, ev: &AgentEvent) -> bool {
        let key = EventKey {
            agent_id: ev.agent_id.clone(),
            ts: ev.ts.clone(),
            source: ev.source.clone(),
            event_type: ev.event_type.clone(),
            run_id: ev.run_id.clone().unwrap_or_default(),
        };
        self.ingest(&ev.agent_id, &ev.on_behalf_of, key, &ev.ts)
    }

    /// Core fold shared by the live and batch paths.
    fn ingest(&mut self, agent_id: &str, chain: &[String], key: EventKey, ts: &str) -> bool {
        let newly = self.seen.insert(key);

        // The acting agent is always a node; count it only on a first sighting.
        if !agent_id.is_empty() {
            let node = self.ensure_node(agent_id);
            if newly {
                node.event_count += 1;
                if ts > node.last_ts.as_str() {
                    node.last_ts = ts.to_string();
                }
            }
        }

        // Chain elements are nodes; consecutive pairs (and last -> agent) are
        // edges. Skip empty elements and any self-edge defensively; a set makes
        // re-adding an edge a no-op regardless of `newly`.
        let mut prev: Option<&str> = None;
        for item in chain {
            if item.is_empty() {
                continue;
            }
            self.ensure_node(item);
            if let Some(p) = prev {
                self.add_edge(p, item);
            }
            prev = Some(item);
        }
        if let Some(p) = prev
            && !agent_id.is_empty()
        {
            self.add_edge(p, agent_id);
        }

        newly
    }

    fn ensure_node(&mut self, id: &str) -> &mut GraphNode {
        self.nodes
            .entry(id.to_string())
            .or_insert_with(|| GraphNode {
                id: id.to_string(),
                kind: NodeKind::of(id),
                event_count: 0,
                last_ts: String::new(),
            })
    }

    fn add_edge(&mut self, from: &str, to: &str) {
        if from != to {
            self.edges.insert((from.to_string(), to.to_string()));
        }
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn node(&self, id: &str) -> Option<&GraphNode> {
        self.nodes.get(id)
    }

    /// Direct delegators of `id` (the `from` of every edge whose `to` is `id`).
    pub fn parents(&self, id: &str) -> Vec<&str> {
        self.edges
            .iter()
            .filter(|(_, to)| to == id)
            .map(|(from, _)| from.as_str())
            .collect()
    }

    /// Direct delegatees of `id` (the `to` of every edge whose `from` is `id`).
    pub fn children(&self, id: &str) -> Vec<&str> {
        self.edges
            .iter()
            .filter(|(from, _)| from == id)
            .map(|(_, to)| to.as_str())
            .collect()
    }

    /// Nodes with no incoming edge: the ultimate principals (usually `user://`).
    pub fn roots(&self) -> Vec<&str> {
        self.nodes
            .keys()
            .filter(|id| !self.edges.iter().any(|(_, to)| to == *id))
            .map(String::as_str)
            .collect()
    }

    /// A serializable snapshot for the shells and the layout engine.
    pub fn view(&self) -> GraphView {
        GraphView {
            nodes: self.nodes.values().cloned().collect(),
            edges: self
                .edges
                .iter()
                .map(|(from, to)| GraphEdge {
                    from: from.clone(),
                    to: to.clone(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::AgentEvent;
    use serde_json::Map;

    fn ev(agent_id: &str, ts: &str, obo: &[&str], run: Option<&str>) -> AgentEvent {
        AgentEvent {
            schema: crate::event::SchemaVersion::SCHEMA_V0_2.to_string(),
            ts: ts.to_string(),
            source: "wardryx".to_string(),
            event_type: "policy_allow".to_string(),
            agent_id: agent_id.to_string(),
            severity: None,
            run_id: run.map(str::to_string),
            on_behalf_of: obo.iter().map(|s| s.to_string()).collect(),
            data: None,
            prev_hash: None,
            extra: Map::new(),
        }
    }

    #[test]
    fn builds_chain_edges_root_first() {
        // chain [user, orchestrator] acting agent = worker:
        //   user -> orchestrator -> worker
        let mut g = DelegationGraph::new();
        assert!(g.add_event(&ev(
            "agent://acme/worker",
            "2026-01-02T00:00:01Z",
            &["user://acme/alice", "agent://acme/orchestrator"],
            Some("run-1"),
        )));
        assert_eq!(g.node_count(), 3);
        assert_eq!(g.edge_count(), 2);
        assert_eq!(
            g.parents("agent://acme/worker"),
            ["agent://acme/orchestrator"]
        );
        assert_eq!(
            g.parents("agent://acme/orchestrator"),
            ["user://acme/alice"]
        );
        assert_eq!(g.roots(), ["user://acme/alice"]);
        assert_eq!(g.node("user://acme/alice").unwrap().kind, NodeKind::User);
        assert_eq!(g.node("agent://acme/worker").unwrap().kind, NodeKind::Agent);
        // the human root never acted, so its event_count is 0
        assert_eq!(g.node("user://acme/alice").unwrap().event_count, 0);
        assert_eq!(g.node("agent://acme/worker").unwrap().event_count, 1);
    }

    #[test]
    fn dedup_on_full_natural_key() {
        let mut g = DelegationGraph::new();
        let e = ev(
            "agent://acme/w",
            "2026-01-02T00:00:01Z",
            &["user://acme/a"],
            Some("r1"),
        );
        assert!(g.add_event(&e)); // first sighting -> counted
        assert!(!g.add_event(&e)); // identical -> dedup, not counted
        assert_eq!(g.node("agent://acme/w").unwrap().event_count, 1);
        assert_eq!(g.edge_count(), 1); // edge idempotent

        // a DIFFERENT run_id is a different event -> counts again, same edge
        let e2 = ev(
            "agent://acme/w",
            "2026-01-02T00:00:01Z",
            &["user://acme/a"],
            Some("r2"),
        );
        assert!(g.add_event(&e2));
        assert_eq!(g.node("agent://acme/w").unwrap().event_count, 2);
        assert_eq!(g.edge_count(), 1);
    }

    #[test]
    fn no_chain_is_a_root_actor_with_no_edges() {
        let mut g = DelegationGraph::new();
        g.add_event(&ev("agent://acme/solo", "2026-01-02T00:00:05Z", &[], None));
        assert_eq!(g.node_count(), 1);
        assert_eq!(g.edge_count(), 0);
        assert_eq!(g.roots(), ["agent://acme/solo"]);
        assert_eq!(
            g.node("agent://acme/solo").unwrap().last_ts,
            "2026-01-02T00:00:05Z"
        );
    }

    #[test]
    fn skips_empty_and_self_edges() {
        let mut g = DelegationGraph::new();
        // an empty chain element and a self-reference must not create bogus
        // nodes or a self-edge.
        g.add_event(&ev(
            "agent://acme/x",
            "2026-01-02T00:00:01Z",
            &["", "agent://acme/x"],
            None,
        ));
        assert_eq!(g.node_count(), 1); // only agent://acme/x, not ""
        assert_eq!(g.edge_count(), 0); // no self-edge x -> x
        assert!(g.node("").is_none());
    }

    #[test]
    fn view_round_trips_json() {
        let mut g = DelegationGraph::new();
        g.add_event(&ev(
            "agent://acme/w",
            "2026-01-02T00:00:01Z",
            &["user://acme/a", "agent://acme/o"],
            None,
        ));
        let view = g.view();
        assert_eq!(view.nodes.len(), 3);
        assert_eq!(view.edges.len(), 2);
        let json = serde_json::to_string(&view).unwrap();
        let back: GraphView = serde_json::from_str(&json).unwrap();
        assert_eq!(view, back);
    }
}
