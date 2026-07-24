import type { UiEvent } from "../types";
import { shortAgentLabel } from "../lib/graph";
import { PopoverHeader } from "../lib/popover";
import { JsonPreview } from "./JsonPreview";
import { SeverityBadge } from "./SeverityBadge";
import { SourceChip } from "./SourceChip";

/**
 * The detail behind one bus event, shown beside the row that was pinned in the
 * Bus Explorer (or any stream). Everything the inline expander used to show,
 * now floating so the row it belongs to can stay put while the feed keeps
 * moving underneath: severity, source, type, the acting agent and the human it
 * acted for, the provenance (env/schema/run/file/offset/prev_hash), the parsed
 * data, and the raw NDJSON line. The agent is a link into its own card.
 */

function Meta({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline gap-2 min-w-0">
      <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
        {label}
      </span>
      <span className="mono tabular truncate text-[11.5px]" style={{ color: "var(--dim)" }} title={value}>
        {value}
      </span>
    </div>
  );
}

export function EventDetailCard({
  event,
  onClose,
  onOpenAgent,
}: {
  event: UiEvent;
  onClose: () => void;
  onOpenAgent?: (agentId: string, rect: DOMRect) => void;
}) {
  return (
    <div className="flex flex-col">
      <PopoverHeader kicker="Bus event" title={event.type} onClose={onClose} />

      <div className="flex items-center gap-2" style={{ padding: "0 16px 10px" }}>
        <SeverityBadge severity={event.severity} />
        <SourceChip source={event.source} />
        <span className="mono text-[10.5px]" style={{ color: "var(--faint)" }}>
          {event.ts}
        </span>
      </div>

      <div style={{ padding: "10px 16px", borderTop: "1px solid var(--line)" }}>
        <div className="flex items-center gap-2 min-w-0">
          <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
            agent
          </span>
          {event.agent_id ? (
            <button
              type="button"
              className="mono text-[12px] truncate"
              style={{ color: "var(--fg)", background: "none", cursor: onOpenAgent ? "pointer" : "default", textAlign: "left" }}
              title={event.agent_id}
              onClick={onOpenAgent ? (e) => onOpenAgent(event.agent_id, e.currentTarget.getBoundingClientRect()) : undefined}
            >
              {shortAgentLabel(event.agent_id)}
              {onOpenAgent && <span style={{ color: "var(--faint)" }}> &rsaquo;</span>}
            </button>
          ) : (
            <span className="mono text-[12px]" style={{ color: "var(--dim)" }}>
              -
            </span>
          )}
        </div>
        {event.on_behalf_of.length > 0 && (
          <div className="flex items-baseline gap-2 min-w-0" style={{ paddingTop: 4 }}>
            <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
              on behalf of
            </span>
            <span className="mono text-[11.5px] truncate" style={{ color: "var(--dim)" }}>
              {event.on_behalf_of.map((u) => shortAgentLabel(u)).join(" -> ")}
            </span>
          </div>
        )}
      </div>

      <div className="flex flex-wrap gap-x-5 gap-y-1.5" style={{ padding: "10px 16px", borderTop: "1px solid var(--line)" }}>
        <Meta label="env" value={event.env} />
        <Meta label="run" value={event.run_id ?? "-"} />
        <Meta label="schema" value={event.schema} />
        <Meta label="offset" value={event.off !== null ? String(event.off) : "-"} />
        <Meta label="prev_hash" value={event.prev_hash ?? "none"} />
      </div>

      <div style={{ padding: "10px 16px 14px", borderTop: "1px solid var(--line)" }}>
        <div className="text-[10px] uppercase tracking-wider mb-1" style={{ color: "var(--faint)" }}>
          data
        </div>
        <JsonPreview value={event.data} />
      </div>
    </div>
  );
}
