import { useEffect, useState } from "react";
import { blockUnit, fetchUnitRecord, type EntityAgent, type UnitRecord } from "../lib/entityRecords";
import { spendByAgent } from "../lib/dashData";
import { formatUsd } from "../lib/format";
import { fetchIdentities } from "../lib/identity";
import { fetchRuns } from "../lib/money";
import { entityAgentState, isUnitStopped, StateBadge, StopStartButton } from "../lib/lifecycle";
import { useLifecycleBlocks } from "../lib/lifecycleBlocks";
import { useConsoleStateVersion } from "../lib/consoleState";
import { usePopover, PopoverHeader } from "../lib/popover";
import { cssVar } from "../lib/cssVars";
import { prettyUnit, unitForTeam } from "../lib/views";
import { AgentDetailCard } from "./AgentDetailCard";
import { UserCard } from "./UserCard";
import { WatchToggleButton } from "./WatchDock";

/** Strip a `user://<org>/` prefix so an identity's owner reads as the plain
 * handle the rest of the app (agent-card owner, user card) uses. */
function ownerHandle(raw: string): string {
  return (raw ?? "").replace(/^user:\/\/[^/]+\//, "").trim();
}

/** Build a real unit record for THIS box from the live money runs + the idryx
 * identity list (which carries each agent's owner), since `unit_record` is a
 * mock-only command with no handler here. Agents are the ones whose team rolls
 * up to this unit; owners are the distinct humans behind them. Returns null
 * when nothing rolls up (render the honest empty state, never a fake row). */
function buildUnitRecord(team: string, runs: Parameters<typeof spendByAgent>[0], identities: { id: string; type: string; owner: string }[]): UnitRecord | null {
  const ownerOf = new Map<string, string>();
  for (const idn of identities) {
    if (idn.type === "agent" || idn.id.startsWith("agent://")) {
      const h = ownerHandle(idn.owner);
      if (h) ownerOf.set(idn.id, h);
    }
  }
  const agents: EntityAgent[] = spendByAgent(runs)
    .filter((a) => unitForTeam(a.team) === team)
    .map((a) => ({
      agentId: a.agent,
      name: a.name,
      team: a.team,
      owner: ownerOf.get(a.agent) ?? "unassigned",
      model: "",
      spentUsd: a.spent,
      calls: a.calls,
      closed: false,
      blocked: false,
      current: true,
    }));
  if (agents.length === 0) return null;
  const owners = [...new Set(agents.map((a) => a.owner).filter((o) => o && o !== "unassigned"))];
  return {
    team,
    agents,
    owners,
    totalSpentUsd: agents.reduce((s, a) => s + a.spentUsd, 0),
    totalCalls: agents.reduce((s, a) => s + a.calls, 0),
  };
}

/**
 * A business unit's fleet: every agent in the unit, the owners (users) running
 * them, and the unit's spend. Opened from an agent card's business-unit field,
 * so "what else does this unit run, and who owns it" is one tap away; agents
 * open their own card, owners open theirs.
 */
export function UnitCard({ team, onOpenFullAgent }: { team: string; onOpenFullAgent?: (agentId: string) => void }) {
  const { open } = usePopover();
  const [rec, setRec] = useState<UnitRecord | null | undefined>(undefined);
  const consoleVersion = useConsoleStateVersion();

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const mock = await fetchUnitRecord(team);
      if (mock) {
        if (!cancelled) setRec(mock);
        return;
      }
      // Real box: unit_record has no handler, so assemble the record from the
      // live money runs + the identity list (owner per agent).
      const [runs, identities] = await Promise.all([
        fetchRuns().catch(() => []),
        fetchIdentities().catch(() => []),
      ]);
      if (!cancelled) setRec(buildUnitRecord(team, runs, identities));
    })();
    return () => {
      cancelled = true;
    };
  }, [team, consoleVersion]);

  // The record answers on the preview; a real box has no unit record, so its
  // own block list is what keeps this button pointing the right way.
  const serverBlocks = useLifecycleBlocks();
  const stopped = isUnitStopped(rec) || serverBlocks.units.includes(team);

  return (
    <div className="flex flex-col">
      <PopoverHeader kicker="Business unit" title={prettyUnit(rec?.team ?? team)} />
      <div className="flex items-center gap-2" style={{ padding: "0 16px 8px" }}>
        <WatchToggleButton kind="unit" id={team} label={prettyUnit(team)} />
        {rec && <StateBadge state={stopped ? "stopped" : "live"} />}
      </div>
      {rec === undefined ? (
        <div className="text-[12px]" style={{ color: "var(--faint)", padding: "8px 16px 16px" }}>
          loading...
        </div>
      ) : rec === null ? (
        <div className="text-[11.5px]" style={{ color: "var(--faint)", padding: "8px 16px 16px" }}>
          no agents roll up to this unit yet.
        </div>
      ) : (
        <>
          <div className="flex flex-wrap gap-x-6 gap-y-1" style={{ padding: "6px 16px 12px" }}>
            <Stat label="agents" value={String(rec.agents.length)} />
            <Stat label="users" value={String(rec.owners.length)} />
            <Stat label="total spend" value={formatUsd(rec.totalSpentUsd)} />
            <Stat label="calls" value={rec.totalCalls.toLocaleString("en-US")} />
          </div>

          <div style={{ padding: "0 16px 12px" }}>
            <div className="mono text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)", paddingBottom: 6 }}>
              users in unit ({rec.owners.length})
            </div>
            <div className="flex flex-wrap gap-1.5">
              {rec.owners.length === 0 ? (
                <span className="text-[11px]" style={{ color: "var(--faint)" }}>
                  no owners resolved for this unit
                </span>
              ) : (
                rec.owners.map((o) => (
                  <button
                    key={o}
                    type="button"
                    className="chip text-[11px]"
                    style={{ color: "var(--dim)", cursor: "pointer" }}
                    onClick={(e) => open(<UserCard handle={o} onOpenFullAgent={onOpenFullAgent} />, { anchor: e.currentTarget.getBoundingClientRect() })}
                  >
                    {o} &rsaquo;
                  </button>
                ))
              )}
            </div>
          </div>

          <div className="flex items-center gap-2" style={{ padding: "0 16px 12px" }}>
            <StopStartButton
              stopped={stopped}
              onToggle={() => blockUnit(rec.team, !stopped).then((x) => { if (x) setRec(x); })}
            />
            <span className="text-[10.5px]" style={{ color: "var(--faint)" }}>
              {stopped ? "starts every agent in this unit" : "stops every agent in this unit"}
            </span>
          </div>

          <div style={{ padding: "10px 16px 14px", borderTop: "1px solid var(--line)" }}>
            <div className="mono text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)", paddingBottom: 6 }}>
              agents in unit ({rec.agents.length})
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
                  {(() => {
                    const st = entityAgentState(a);
                    return st === "live" ? null : <StateBadge state={st} />;
                  })()}
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
