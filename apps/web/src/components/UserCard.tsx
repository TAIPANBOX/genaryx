import { useEffect, useState } from "react";
import { userHandle } from "../lib/agentRecord";
import { spendByAgent } from "../lib/dashData";
import { blockUser, fetchUserRecord, type EntityAgent, type UserRecord } from "../lib/entityRecords";
import { formatUsd } from "../lib/format";
import { fetchIdentities } from "../lib/identity";
import { fetchRuns } from "../lib/money";
import { entityAgentState, isUserStopped, StateBadge, StopStartButton } from "../lib/lifecycle";
import { useLifecycleBlocks } from "../lib/lifecycleBlocks";
import { useConsoleStateVersion } from "../lib/consoleState";
import { usePopover, PopoverHeader } from "../lib/popover";
import { cssVar } from "../lib/cssVars";
import { prettyUnit, unitForTeam } from "../lib/views";
import { AgentDetailCard } from "./AgentDetailCard";
import { UnitCard } from "./UnitCard";
import { WatchToggleButton } from "./WatchDock";

/** Build a real user record for THIS box from the live money runs + the idryx
 * identity list (which carries each agent's owner), since `user_record` is a
 * mock-only command with no handler here. The user's agents are the ones the
 * identity plane says this handle owns; their runs give spend/calls/teams.
 * Returns null when this handle owns nothing that shows up (render the honest
 * empty state, never a fabricated row). */
function buildUserRecord(handle: string, runs: Parameters<typeof spendByAgent>[0], identities: { id: string; type: string; owner: string }[]): UserRecord | null {
  const ownerOf = new Map<string, string>();
  for (const idn of identities) {
    if (idn.type === "agent" || idn.id.startsWith("agent://")) {
      const h = userHandle(idn.owner ?? "").trim();
      if (h) ownerOf.set(idn.id, h);
    }
  }
  const agents: EntityAgent[] = spendByAgent(runs)
    .filter((a) => ownerOf.get(a.agent) === handle)
    .map((a) => ({
      agentId: a.agent,
      name: a.name,
      team: a.team,
      owner: handle,
      model: "",
      spentUsd: a.spent,
      calls: a.calls,
      closed: false,
      blocked: false,
      current: true,
    }));
  if (agents.length === 0) return null;
  return {
    handle,
    agents,
    teams: [...new Set(agents.map((a) => a.team))],
    totalSpentUsd: agents.reduce((s, a) => s + a.spentUsd, 0),
    totalCalls: agents.reduce((s, a) => s + a.calls, 0),
  };
}

/**
 * The human owner behind an agent: every agent they own, what those agents
 * spend, and which units they span. Opened from an agent card's owner field so
 * "who is this person and what else do they run" is one tap away, and each
 * agent here opens its own card in turn.
 */
export function UserCard({ handle, onOpenFullAgent }: { handle: string; onOpenFullAgent?: (agentId: string) => void }) {
  const { open } = usePopover();
  const [rec, setRec] = useState<UserRecord | null | undefined>(undefined);
  const consoleVersion = useConsoleStateVersion();

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const mock = await fetchUserRecord(handle);
      if (mock) {
        if (!cancelled) setRec(mock);
        return;
      }
      // Real box: user_record has no handler, so assemble from live runs +
      // the identity list (owner per agent).
      const [runs, identities] = await Promise.all([
        fetchRuns().catch(() => []),
        fetchIdentities().catch(() => []),
      ]);
      if (!cancelled) setRec(buildUserRecord(handle, runs, identities));
    })();
    return () => {
      cancelled = true;
    };
  }, [handle, consoleVersion]);

  // Same as the unit card: the record is preview-only, the box's own block
  // list is the real-box source.
  const currentAgents = (rec?.agents ?? []).filter((x) => x.current);
  const pastAgents = (rec?.agents ?? []).filter((x) => !x.current);
  const serverBlocks = useLifecycleBlocks();
  const stopped = isUserStopped(rec) || serverBlocks.users.includes(handle);

  return (
    <div className="flex flex-col">
      <PopoverHeader kicker="User" title={rec?.handle ?? handle} />
      <div className="flex items-center gap-2" style={{ padding: "0 16px 8px" }}>
        <WatchToggleButton kind="user" id={handle} label={handle} />
        {rec && <StateBadge state={stopped ? "stopped" : "live"} />}
      </div>
      {rec === undefined ? (
        <div className="text-[12px]" style={{ color: "var(--faint)", padding: "8px 16px 16px" }}>
          loading...
        </div>
      ) : rec === null ? (
        <div className="text-[11.5px]" style={{ color: "var(--faint)", padding: "8px 16px 16px" }}>
          owner records need a store, which this box does not keep.
        </div>
      ) : (
        <>
          <div className="flex flex-wrap gap-x-6 gap-y-1" style={{ padding: "6px 16px 12px" }}>
            {/* Agents this user owns NOW. One they used to own still appears
                in the list below, badged, because their spend for that period
                is theirs; counting it here would disagree with the Statistics
                table, with "calls" on this same line, and with any sensible
                reading of "how many agents does this person run". */}
            <Stat label="agents" value={String(currentAgents.length)} />
            {pastAgents.length > 0 && (
              <Stat
                label="past"
                value={String(pastAgents.length)}
                title="Agents this user used to own. Listed below, badged, and still counted in total spend for the period they owned them."
              />
            )}
            <Stat
              label="total spend"
              value={formatUsd(rec.totalSpentUsd)}
              title={
                pastAgents.length > 0
                  ? "This user's share across time: each agent's spend for the period they owned it, which is why it does not equal their current agents' totals."
                  : undefined
              }
            />
            <Stat label="calls" value={rec.totalCalls.toLocaleString("en-US")} />
          </div>
          <div className="flex flex-wrap gap-1.5" style={{ padding: "0 16px 12px" }}>
            {[...new Set(rec.teams.map(unitForTeam))].map((unit) => (
              <button
                key={unit}
                type="button"
                className="chip text-[11px]"
                style={{ color: "var(--dim)", cursor: "pointer" }}
                onClick={(e) => open(<UnitCard team={unit} onOpenFullAgent={onOpenFullAgent} />, { anchor: e.currentTarget.getBoundingClientRect() })}
              >
                {prettyUnit(unit)} &rsaquo;
              </button>
            ))}
          </div>
          <div className="flex items-center gap-2" style={{ padding: "0 16px 12px" }}>
            <StopStartButton
              stopped={stopped}
              onToggle={() => blockUser(rec.handle, !stopped).then((x) => { if (x) setRec(x); })}
            />
            <span className="text-[10.5px]" style={{ color: "var(--faint)" }}>
              {stopped ? "starts every agent this user owns" : "stops every agent this user owns"}
            </span>
          </div>
          <div style={{ padding: "10px 16px 14px", borderTop: "1px solid var(--line)" }}>
            <div className="mono text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)", paddingBottom: 6 }}>
              agents owned
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
                    {a.name} <span style={{ color: "var(--faint)" }}>· {prettyUnit(unitForTeam(a.team))}</span>
                  </span>
                  {(() => {
                    const st = entityAgentState(a);
                    return st === "live" ? null : <StateBadge state={st} />;
                  })()}
                  {!a.current && (
                    <span className="badge" style={cssVar("tone", "var(--faint)")} title="owned in the past; spend shown is this user's share">
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

function Stat({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <div className="flex flex-col" title={title}>
      <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
        {label}
      </span>
      <span className="mono tabular text-[15px]" style={{ color: "var(--fg)" }}>
        {value}
      </span>
    </div>
  );
}
