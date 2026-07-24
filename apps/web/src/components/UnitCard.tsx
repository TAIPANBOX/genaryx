import { useEffect, useState } from "react";
import { blockUnit, fetchUnitRecord, type UnitRecord } from "../lib/entityRecords";
import { formatUsd } from "../lib/format";
import { usePopover, PopoverHeader } from "../lib/popover";
import { cssVar } from "../lib/cssVars";
import { AgentDetailCard } from "./AgentDetailCard";
import { UserCard } from "./UserCard";
import { WatchToggleButton } from "./WatchDock";

/**
 * A business unit's fleet: every agent in the unit, the owners running them,
 * and the unit's spend. Opened from an agent card's business-unit field, so
 * "what else does treasury run, and who owns it" is one tap away; agents open
 * their own card, owners open theirs.
 */
export function UnitCard({ team, onOpenFullAgent }: { team: string; onOpenFullAgent?: (agentId: string) => void }) {
  const { open } = usePopover();
  const [rec, setRec] = useState<UnitRecord | null | undefined>(undefined);

  useEffect(() => {
    let cancelled = false;
    void fetchUnitRecord(team).then((r) => !cancelled && setRec(r));
    return () => {
      cancelled = true;
    };
  }, [team]);

  return (
    <div className="flex flex-col">
      <PopoverHeader kicker="Business unit" title={rec?.team ?? team} />
      <div className="flex items-center" style={{ padding: "0 16px 8px" }}>
        <WatchToggleButton kind="unit" id={team} label={team} />
      </div>
      {rec === undefined ? (
        <div className="text-[12px]" style={{ color: "var(--faint)", padding: "8px 16px 16px" }}>
          loading...
        </div>
      ) : rec === null ? (
        <div className="text-[11.5px]" style={{ color: "var(--faint)", padding: "8px 16px 16px" }}>
          unit records need a store, which this box does not keep.
        </div>
      ) : (
        <>
          <div className="flex flex-wrap gap-x-6 gap-y-1" style={{ padding: "6px 16px 12px" }}>
            <Stat label="agents" value={String(rec.agents.length)} />
            <Stat label="total spend" value={formatUsd(rec.totalSpentUsd)} />
            <Stat label="calls" value={rec.totalCalls.toLocaleString("en-US")} />
          </div>

          <div style={{ padding: "0 16px 12px" }}>
            <div className="mono text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)", paddingBottom: 6 }}>
              owners
            </div>
            <div className="flex flex-wrap gap-1.5">
              {rec.owners.map((o) => (
                <button
                  key={o}
                  type="button"
                  className="chip text-[11px]"
                  style={{ color: "var(--dim)", cursor: "pointer" }}
                  onClick={(e) => open(<UserCard handle={o} onOpenFullAgent={onOpenFullAgent} />, { anchor: e.currentTarget.getBoundingClientRect() })}
                >
                  {o} &rsaquo;
                </button>
              ))}
            </div>
          </div>

          <div className="flex items-center gap-2" style={{ padding: "0 16px 12px" }}>
            {(() => {
              const allBlocked = rec.agents.length > 0 && rec.agents.every((a) => a.blocked);
              return (
                <button
                  type="button"
                  onClick={() => void blockUnit(rec.team, !allBlocked).then((x) => x && setRec(x))}
                  style={{
                    padding: "5px 12px",
                    borderRadius: 7,
                    cursor: "pointer",
                    border: `1px solid ${allBlocked ? "var(--mint)" : "var(--sev-high)"}`,
                    background: allBlocked ? "color-mix(in srgb, var(--mint) 14%, transparent)" : "color-mix(in srgb, var(--sev-high) 14%, transparent)",
                    color: "var(--fg)",
                    fontSize: 12,
                  }}
                >
                  {allBlocked ? "Enable all agents" : "Disable all agents"}
                </button>
              );
            })()}
            <span className="text-[10.5px]" style={{ color: "var(--faint)" }}>
              blocks every agent in this unit
            </span>
          </div>

          <div style={{ padding: "10px 16px 14px", borderTop: "1px solid var(--line)" }}>
            <div className="mono text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)", paddingBottom: 6 }}>
              agents in unit
            </div>
            <div className="flex flex-col">
              {rec.agents.map((a) => (
                <button
                  key={a.agentId}
                  type="button"
                  className="flex items-center gap-3 min-w-0"
                  style={{ padding: "5px 6px", borderRadius: 6, cursor: "pointer", background: "none", textAlign: "left" }}
                  onClick={(e) =>
                    open(<AgentDetailCard agentId={a.agentId} onOpenFull={onOpenFullAgent} />, {
                      anchor: e.currentTarget.getBoundingClientRect(),
                    })
                  }
                  onMouseEnter={(e) => (e.currentTarget.style.background = "var(--panel-2)")}
                  onMouseLeave={(e) => (e.currentTarget.style.background = "none")}
                >
                  <span className="text-[12px] truncate" style={{ color: "var(--fg)", flex: 1 }}>
                    {a.name} <span style={{ color: "var(--faint)" }}>· {a.owner}</span>
                  </span>
                  {a.closed && (
                    <span className="badge" style={cssVar("tone", "var(--sev-critical)")}>
                      closed
                    </span>
                  )}
                  {a.blocked && !a.closed && (
                    <span className="badge" style={cssVar("tone", "var(--amber)")}>
                      disabled
                    </span>
                  )}
                  {!a.current && (
                    <span className="badge" style={cssVar("tone", "var(--faint)")} title="was in this unit in the past; spend shown is this unit's share">
                      past
                    </span>
                  )}
                  <span className="mono tabular text-[11.5px]" style={{ color: "var(--dim)" }}>
                    {formatUsd(a.spentUsd)}
                  </span>
                  <span className="text-[11px]" style={{ color: "var(--faint)" }}>
                    &rsaquo;
                  </span>
                </button>
              ))}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col">
      <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
        {label}
      </span>
      <span className="mono tabular text-[15px]" style={{ color: "var(--fg)" }}>
        {value}
      </span>
    </div>
  );
}
