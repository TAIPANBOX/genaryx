import { hasBackend, invokeBackend } from "./transport";
import type { AgentSlice, LayoutView } from "../graphTypes";
import type { UiEvent } from "../types";

/** A clean empty graph - the same honest "nothing yet" shape
 * `crates/api/src/graph.rs`'s `agent_graph` returns for a missing/failed
 * Store. Used as this module's own no-backend fallback too (a plain
 * `vite build`/browser preview): unlike the Bus Explorer's `mockData.ts`,
 * there is no plausible mock delegation graph to fabricate, and unlike
 * `lib/money.ts`/`lib/identity.ts` there is also no failure state worth
 * reporting here - the graph commands never error in the first place, so
 * "no backend" and "no Store yet" read as the exact same honest empty
 * result rather than two different ones. */
const EMPTY_GRAPH: LayoutView = { nodes: [], edges: [], width: 1000, height: 1000 };

const EMPTY_SLICE: AgentSlice = { node: null, parents: [], children: [] };

/** The whole delegation graph, laid out and ready to draw
 * (`DelegationGraphView.tsx`). Never throws: with no backend, or on any
 * transport failure, this resolves to [`EMPTY_GRAPH`] rather than an error - matching
 * `agent_graph`'s own "never an Err the UI traps on" contract on the Rust
 * side. */
export async function fetchAgentGraph(): Promise<LayoutView> {
  if (!hasBackend()) return EMPTY_GRAPH;
  try {
    return await invokeBackend<LayoutView>("agent_graph");
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("agent_graph invoke failed, rendering an empty graph:", err);
    return EMPTY_GRAPH;
  }
}

/** One agent's immediate delegation neighborhood (Agent 360's Delegation
 * section). Same never-throws contract as [`fetchAgentGraph`]. */
export async function fetchAgentSlice(agentId: string): Promise<AgentSlice> {
  if (!hasBackend()) return EMPTY_SLICE;
  try {
    return await invokeBackend<AgentSlice>("agent_slice", { agent_id: agentId });
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error(`agent_slice invoke failed for ${agentId}, rendering an empty slice:`, err);
    return EMPTY_SLICE;
  }
}

/** This agent's most recent `limit` events, newest first (Agent 360's
 * Events section, and - filtered client-side to `source === "wardryx"` - its
 * Policy section). Same never-throws contract as [`fetchAgentGraph`]. */
export async function fetchAgentEvents(agentId: string, limit: number): Promise<UiEvent[]> {
  if (!hasBackend()) return [];
  try {
    return await invokeBackend<UiEvent[]>("agent_events", { agent_id: agentId, limit });
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error(`agent_events invoke failed for ${agentId}, rendering no events:`, err);
    return [];
  }
}

/** Short-form label for a node/agent id in the graph and every deep-link
 * chip: the last path segment of a `user://`/`agent://` URI (e.g.
 * `agent://taipanbox.dev/demo/orchestrator` -> `orchestrator`,
 * `user://taipanbox.dev/j.doe` -> `j.doe`). Falls back to the full id
 * unchanged for anything that does not look like one of those two schemes -
 * same "an unrecognized shape still renders, just without special handling"
 * tolerance every other id-shaped field in this app follows. */
/** Whether an id names a PERSON rather than an agent.
 *
 * The delegation chain carries both. `on_behalf_of` bottoms out in a human
 * (`user://meridian.io/n.foster`), and the delegation graph gives those their
 * own node kind, so every surface that lets you click a principal will be
 * handed one sooner or later.
 *
 * This exists because for a while none of them checked. Every chip and every
 * graph node routed straight to Agent 360, so clicking a person opened an agent
 * card about a person: "this agent has never been seen on the delegation
 * graph", "no idryx identity record for this agent", "no runs for this agent
 * yet". Each sentence true, all of them nonsense, and the card looked like it
 * was working because `shortAgentLabel` already handled `user://` and rendered
 * the name correctly. The label knew; the click did not.
 */
export function isUserId(id: string): boolean {
  return id.startsWith("user://");
}

export function shortAgentLabel(id: string): string {
  const match = /^(?:user|agent):\/\/[^/]+\/(.+)$/.exec(id);
  return match ? match[1] : id;
}
