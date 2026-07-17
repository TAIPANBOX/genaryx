import { cssVar } from "../lib/cssVars";
import { formatTimestamp } from "../lib/format";
import type { Incident } from "../moneyTypes";
import { ConfirmButton } from "./ConfirmButton";
import { SeverityBadge } from "./SeverityBadge";

export function IncidentsList({
  incidents,
  onAck,
}: {
  incidents: Incident[];
  onAck: (id: string) => Promise<void>;
}) {
  if (incidents.length === 0) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        no incidents.
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      {incidents.map((inc) => (
        <div
          key={inc.id}
          className="panel px-3 py-2.5 flex items-center gap-3"
          style={{ background: "var(--panel-2)" }}
        >
          <SeverityBadge severity={inc.severity} />
          <div className="flex flex-col min-w-0 flex-1">
            <span className="mono truncate text-[12px]" title={inc.kind} style={{ color: "var(--fg)" }}>
              {inc.kind}
            </span>
            <span className="mono truncate text-[11px]" style={{ color: "var(--faint)" }}>
              {inc.run_id ?? "no run"} &middot; {inc.occurrences} occurrence{inc.occurrences === 1 ? "" : "s"} &middot;
              last {formatTimestamp(inc.last_seen)}
            </span>
          </div>
          {inc.acknowledged ? (
            <span className="badge" style={cssVar("tone", "var(--sev-low)")}>
              acked
            </span>
          ) : (
            <ConfirmButton label="Ack" confirmLabel="Confirm ack" tone="var(--sev-medium)" onConfirm={() => onAck(inc.id)} />
          )}
        </div>
      ))}
    </div>
  );
}
