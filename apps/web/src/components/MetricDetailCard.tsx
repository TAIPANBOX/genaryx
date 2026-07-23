import type { ReactNode } from "react";
import { usePopover, PopoverHeader } from "../lib/popover";
import { AgentDetailCard } from "./AgentDetailCard";

/**
 * The card behind a clicked number. Every headline figure on the dashboards
 * (Active runs, Governed saved, Open incidents, Model calls, ...) opens one of
 * these: the number itself, a one-line plain explanation of what it counts,
 * and the breakdown rows that make it up, each of which can drill further (an
 * agent row opens that agent's card). This is what turns a static KPI tile
 * into something you can actually inspect.
 */

export interface MetricRow {
  key: string;
  label: ReactNode;
  value?: ReactNode;
  /** When set, the row is itself clickable and opens this agent's card. */
  agentId?: string;
  /** Optional colour for the value (severity, tone). */
  valueColor?: string;
}

export function MetricDetailCard({
  kicker,
  title,
  value,
  valueTone,
  description,
  rows,
  rowsTitle,
  onOpenFullAgent,
}: {
  kicker?: string;
  title: string;
  value: ReactNode;
  valueTone?: string;
  description: ReactNode;
  rows?: MetricRow[];
  rowsTitle?: string;
  onOpenFullAgent?: (agentId: string) => void;
}) {
  const { open } = usePopover();

  return (
    <div className="flex flex-col">
      <PopoverHeader kicker={kicker ?? "Metric"} title={title} />
      <div style={{ padding: "0 16px 12px" }}>
        <div className="d-heronum" style={{ fontSize: 34, color: valueTone ?? "var(--fg)", lineHeight: 1.1 }}>
          {value}
        </div>
        <div className="text-[12px]" style={{ color: "var(--dim)", paddingTop: 6 }}>
          {description}
        </div>
      </div>

      {rows && rows.length > 0 && (
        <div style={{ padding: "10px 16px 14px", borderTop: "1px solid var(--line)" }}>
          {rowsTitle && (
            <div className="mono text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)", paddingBottom: 6 }}>
              {rowsTitle}
            </div>
          )}
          <div className="flex flex-col">
            {rows.map((r) => {
              const clickable = Boolean(r.agentId);
              return (
                <div
                  key={r.key}
                  className="flex items-center gap-3 min-w-0"
                  style={{
                    padding: "5px 6px",
                    borderRadius: 6,
                    cursor: clickable ? "pointer" : undefined,
                  }}
                  role={clickable ? "button" : undefined}
                  tabIndex={clickable ? 0 : undefined}
                  onClick={
                    clickable
                      ? (e) =>
                          open(<AgentDetailCard agentId={r.agentId!} onOpenFull={onOpenFullAgent} />, {
                            anchor: e.currentTarget.getBoundingClientRect(),
                          })
                      : undefined
                  }
                  onMouseEnter={clickable ? (e) => (e.currentTarget.style.background = "var(--panel-2)") : undefined}
                  onMouseLeave={clickable ? (e) => (e.currentTarget.style.background = "transparent") : undefined}
                >
                  <span className="text-[12px] min-w-0 truncate" style={{ color: "var(--fg)", flex: 1 }}>
                    {r.label}
                  </span>
                  {r.value != null && (
                    <span className="mono tabular text-[11.5px]" style={{ color: r.valueColor ?? "var(--dim)" }}>
                      {r.value}
                    </span>
                  )}
                  {clickable && (
                    <span className="text-[11px]" style={{ color: "var(--faint)" }}>
                      &rsaquo;
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
