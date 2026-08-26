import { usePopover } from "./popover";
import { AgentDetailCard } from "../components/AgentDetailCard";
import { EventDetailCard } from "../components/EventDetailCard";
import { IncidentTextCard } from "../components/IncidentTextCard";
import type { UnifiedIncident } from "./incidents";

/**
 * Opening one incident row beside itself, in one implementation.
 *
 * Two surfaces show these rows now, the Overview card and the Anomalies view,
 * and a drill written twice is a drill that diverges: one of them grows a
 * source's detail card and the other keeps sending that source nowhere, and
 * nobody notices because each is correct on its own screen. This estate has
 * been bitten by that shape often enough to name it, so the rule lives here
 * and both callers are thin.
 *
 * Rendering JSX is why this is a `.tsx` under `lib/` rather than sitting in
 * `lib/incidents.ts`, which is deliberately framework-free and stays that way:
 * the aggregation is testable without a DOM and must not acquire React on the
 * way to giving a row a click handler.
 */
export function useIncidentDrill(onOpenAgentFull?: (agentId: string) => void) {
  const { open, close } = usePopover();

  const openAgent = (agentId: string, rect: DOMRect) =>
    open(<AgentDetailCard agentId={agentId} onOpenFull={onOpenAgentFull} />, { anchor: rect });

  /**
   * Each source opens the most specific record this console actually holds.
   *
   *   bus, verdryx  the raw envelope, through `EventDetailCard`, which already
   *                 renders a `UiEvent` with its severity, its producer, its
   *                 time and its whole `data` object. That is the answer to
   *                 "what happened" for anything off the bus, and it needed no
   *                 new component.
   *   money         the agent the incident is about. Money incidents carry a
   *                 run and an agent, and the agent card is the one that leads
   *                 somewhere: its own "open full" goes to Agent 360.
   *   idryx         the identity the detector fired about, which is the subject
   *                 of an identity alert the way an agent is the subject of a
   *                 money incident.
   *   posture       a computed state rather than a stored record, so it opens
   *                 its own text and says so. See `IncidentTextCard`.
   */
  const openIncident = (row: UnifiedIncident, rect: DOMRect) => {
    if (row.source === "bus" || row.source === "verdryx") {
      // `close(id)` and not `close()`: the second closes every open window, so
      // an operator who opened an agent card and then an event beside it would
      // lose both from one dismiss.
      const id = open(
        <EventDetailCard event={row.raw} onClose={() => close(id)} onOpenAgent={openAgent} />,
        { anchor: rect },
      );
      return;
    }
    if (row.source === "money" && row.raw.agent_id) {
      openAgent(row.raw.agent_id, rect);
      return;
    }
    if (row.source === "idryx" && row.raw.identity) {
      openAgent(row.raw.identity, rect);
      return;
    }
    const id = open(<IncidentTextCard row={row} onClose={() => close(id)} />, { anchor: rect });
  };

  return { openIncident, openAgent };
}
