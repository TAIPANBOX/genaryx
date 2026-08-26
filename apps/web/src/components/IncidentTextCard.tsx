import { PopoverHeader } from "../lib/popover";
import { SeverityBadge } from "./SeverityBadge";
import { INCIDENT_SOURCE_LABEL, type UnifiedIncident } from "../lib/incidents";

/**
 * The detail card for an incident this console holds no deeper record about.
 *
 * Posture is the case it exists for. A posture finding is a computed state
 * rather than a stored event: it has no id in a store, no subject to open, and
 * no history behind it, so there is nothing more specific to show than what the
 * finding already says. It is also the fallback for a money incident carrying
 * neither an agent nor a run, and for any future source added to
 * `UnifiedIncident` before it has a detail surface.
 *
 * It shows what the row shows, and nothing else. That sounds like a card not
 * worth opening and it is the point: before 2026-08-26 these rows did not open
 * at all, and a row that silently ignores a click teaches an operator that the
 * panel is not interactive, which cost every OTHER row its drill-in too. A card
 * that says "this is all there is" is a different message from no response, and
 * it is the honest one here.
 *
 * What it deliberately does NOT do is invent a fuller-looking detail view by
 * restating the same two fields in more words. genaryx invariant 4 is about
 * fabricated rows; this is the same instinct applied to a card.
 */
export function IncidentTextCard({
  row,
  onClose,
}: {
  row: UnifiedIncident;
  onClose: () => void;
}) {
  return (
    <div className="flex flex-col" style={{ maxWidth: 420 }}>
      <PopoverHeader kicker={INCIDENT_SOURCE_LABEL[row.source]} title={row.title} onClose={onClose} />
      <div className="flex items-center gap-2" style={{ padding: "0 16px 10px" }}>
        <SeverityBadge severity={row.severity} />
        {row.ts && (
          <span className="mono text-[10.5px]" style={{ color: "var(--faint)" }}>
            {row.ts}
          </span>
        )}
        {row.occurrences !== undefined && row.occurrences > 1 && (
          <span className="mono text-[10.5px]" style={{ color: "var(--faint)" }}>
            {row.occurrences}×
          </span>
        )}
      </div>
      <div style={{ padding: "0 16px 14px", fontSize: 12, lineHeight: 1.5 }}>{row.detail}</div>
      <div
        className="mono"
        style={{ padding: "0 16px 14px", fontSize: 10, color: "var(--faint)" }}
      >
        this console holds no record behind this row beyond what is shown
      </div>
    </div>
  );
}
