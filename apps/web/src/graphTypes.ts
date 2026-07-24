/**
 * Delegation graph + Agent 360 wire types (docs/PHASE3.md W3). Unlike
 * `identityTypes.ts`'s `Idryx*` interfaces, these mirror `genaryx-core` types
 * directly (`genaryx_core::{LayoutView, PositionedNode, GraphEdge, NodeKind,
 * AgentSlice, GraphNode}` - `crates/core/src/{graph,layout}.rs`), not a
 * connector: every one of them already derives `Serialize` and crosses the
 * genaryx-web JSON boundary unmirrored (`crates/api/src/graph.rs`'s module doc), so
 * this file exists only to give the frontend names/types for that same wire
 * shape, exactly the same "no UI-facing mirror struct needed" convention
 * `identityTypes.ts` documents for the Idryx DTOs.
 */

/** Mirrors `genaryx_core::graph::NodeKind` (`#[serde(rename_all = "lowercase")]`).
 * `"other"` is not dead weight: any `on_behalf_of`/`agent_id` URI scheme
 * outside `user://`/`agent://` still renders as a node, just uncategorized -
 * same "an unrecognized value must still render" tolerance `IdryxIdentity.type`
 * and `UiEvent.severity` already follow. */
export type NodeKind = "user" | "agent" | "other";

/** Mirrors `genaryx_core::graph::GraphNode`. */
export interface GraphNode {
  id: string;
  kind: NodeKind;
  /** Events where this node was the acting `agent_id`, deduped on the
   * natural key. A pure delegator that never itself acted stays at 0. */
  event_count: number;
  /** Most recent `ts` this node was seen acting (RFC 3339); `""` for a node
   * only ever seen inside another's delegation chain. */
  last_ts: string;
}

/** Mirrors `genaryx_core::graph::GraphEdge` - one directed "delegates_to"
 * edge: `from` acted on behalf of `to`. */
export interface GraphEdge {
  from: string;
  to: string;
}

/** Mirrors `genaryx_core::graph::AgentSlice` - one agent's immediate
 * delegation neighborhood (the Agent 360 card's Delegation section).
 * `node: null` means this agent has never been seen on the bus - a real,
 * honest state (e.g. a pure human root, or an id typed into a deep link that
 * never actually acted), never an error. */
export interface AgentSlice {
  node: GraphNode | null;
  parents: GraphNode[];
  children: GraphNode[];
}

/** Mirrors `genaryx_core::layout::PositionedNode` - one graph node with a
 * computed Canvas2D position (PHASE3 §5.3: layout is computed once in core,
 * both shells just draw it). */
export interface PositionedNode {
  id: string;
  kind: NodeKind;
  event_count: number;
  x: number;
  y: number;
}

/** Mirrors `genaryx_core::layout::LayoutView` - the full laid-out graph a
 * Canvas2D renderer draws directly: positioned nodes, the edges, and the
 * canvas bounds the positions live in. An empty graph (`nodes: []`) is a
 * real, honest state (no delegation activity yet / no Store), never an
 * error - see `crates/api/src/graph.rs`'s `agent_graph` doc comment. */
export interface LayoutView {
  nodes: PositionedNode[];
  edges: GraphEdge[];
  width: number;
  height: number;
}
