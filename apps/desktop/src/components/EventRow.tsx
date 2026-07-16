import type { KeyboardEvent } from "react";
import type { UiEvent } from "../types";
import { JsonPreview } from "./JsonPreview";
import { SeverityBadge } from "./SeverityBadge";
import { SourceChip } from "./SourceChip";

function formatClock(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  const ss = String(d.getSeconds()).padStart(2, "0");
  const ms = String(d.getMilliseconds()).padStart(3, "0");
  return `${hh}:${mm}:${ss}.${ms}`;
}

/** A `key: value` provenance chip in the expand panel: label dim, value in
 * mono so ids/hashes/paths line up. */
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

export function EventRow({
  event,
  expanded,
  onToggle,
}: {
  event: UiEvent;
  expanded: boolean;
  onToggle: () => void;
}) {
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onToggle();
    }
  };

  return (
    <div className={`bus-row${expanded ? " expanded" : ""}`}>
      <div
        role="button"
        tabIndex={0}
        aria-expanded={expanded}
        onClick={onToggle}
        onKeyDown={onKeyDown}
        className="grid items-center gap-3 px-4 py-2 cursor-pointer select-none"
        style={{ gridTemplateColumns: "84px 108px 190px 1fr 108px 24px" }}
      >
        <SeverityBadge severity={event.severity} />
        <SourceChip source={event.source} />
        <span className="mono truncate text-[12px]" style={{ color: "var(--fg)" }} title={event.type}>
          {event.type}
        </span>
        <span
          className="mono tabular truncate text-[12px]"
          style={{ color: "var(--dim)" }}
          title={event.agent_id}
        >
          {event.agent_id}
        </span>
        <span
          className="mono tabular text-[11.5px] text-right"
          style={{ color: "var(--faint)" }}
          title={event.ts}
        >
          {formatClock(event.ts)}
        </span>
        <svg
          viewBox="0 0 24 24"
          width="13"
          height="13"
          fill="none"
          aria-hidden="true"
          style={{
            color: "var(--faint)",
            transform: expanded ? "rotate(90deg)" : "none",
            transition: "transform 0.15s ease",
            justifySelf: "end",
          }}
        >
          <path d="M9 5l7 7-7 7" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </div>

      {expanded && (
        <div className="px-4 pb-3 pt-1 flex flex-col gap-3">
          <div className="flex flex-wrap gap-x-5 gap-y-1.5 panel px-3 py-2" style={{ background: "var(--panel-2)" }}>
            <Meta label="env" value={event.env} />
            <Meta label="schema" value={event.schema} />
            <Meta label="run" value={event.run_id ?? "-"} />
            <Meta label="file" value={event.file ?? "-"} />
            <Meta label="offset" value={event.off !== null ? String(event.off) : "-"} />
            <Meta label="prev_hash" value={event.prev_hash ?? "none"} />
            {event.on_behalf_of.length > 0 && (
              <Meta label="on_behalf_of" value={event.on_behalf_of.join(" -> ")} />
            )}
          </div>

          <div className="grid gap-3" style={{ gridTemplateColumns: "1fr 1fr" }}>
            <div className="min-w-0">
              <div className="text-[10px] uppercase tracking-wider mb-1" style={{ color: "var(--faint)" }}>
                data
              </div>
              <JsonPreview value={event.data} />
            </div>
            <div className="min-w-0">
              <div className="text-[10px] uppercase tracking-wider mb-1" style={{ color: "var(--faint)" }}>
                raw NDJSON line
              </div>
              <pre className="json-pre mono thin-scroll" style={{ whiteSpace: "pre-wrap", wordBreak: "break-all" }}>
                {event.raw}
              </pre>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
