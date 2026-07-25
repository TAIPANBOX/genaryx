import { useEffect, useState } from "react";
import { fetchAgentRecord, userHandle, type AgentRecord } from "../lib/agentRecord";
import {
  blockAgent,
  fetchOrgDirectory,
  reassignAgentUnit,
  setAgentBehaviour,
  setAgentBudget,
  transferAgentOwner,
  type OrgDirectory,
} from "../lib/agentActions";
import { shortAgentLabel } from "../lib/graph";
import { fetchIdentities } from "../lib/identity";
import { fetchRuns, killRun } from "../lib/money";
import { agentStateFromRecord, FreezeToggleButton, KillRunButton, StateBadge } from "../lib/lifecycle";
import { useLifecycleBlocks } from "../lib/lifecycleBlocks";
import { useConsoleStateVersion } from "../lib/consoleState";
import { formatTimestamp, formatUsd } from "../lib/format";
import { cssVar } from "../lib/cssVars";
import { usePopover, useWindowControls, PopoverHeader } from "../lib/popover";
import { prettyUnit, unitForTeam } from "../lib/views";
import type { IdryxIdentity } from "../identityTypes";
import type { Run } from "../moneyTypes";
import { UserCard } from "./UserCard";
import { UnitCard } from "./UnitCard";

/**
 * The agent detail card, shown in the floating popover layer beside whatever
 * was clicked (a spend bar, a run row, a graph node, a bus row's agent). It
 * answers "who is this, what does it do, whose is it, and how is it behaving"
 * in one place: business unit and owner, model, budget vs spend, allowed
 * behaviour, and, when the backend keeps an owned record (the preview does),
 * the lifecycle - launched, owned, transferred, and, for a caught runaway,
 * closed and why.
 *
 * Read-only by design at this layer; heavier actions (transfer, edit budget)
 * live in their own sub-forms so a glance never risks a mutation. Sections
 * whose data the backend does not keep are simply omitted, never faked.
 */

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-baseline gap-3 min-w-0" style={{ padding: "3px 0" }}>
      <span className="text-[10.5px] uppercase tracking-wider shrink-0" style={{ color: "var(--faint)", width: 92 }}>
        {label}
      </span>
      <span className="text-[12px] min-w-0" style={{ color: "var(--dim)" }}>
        {children}
      </span>
    </div>
  );
}

function Section({ title, children, right }: { title: string; children: React.ReactNode; right?: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1" style={{ padding: "10px 16px", borderTop: "1px solid var(--line)" }}>
      <div className="flex items-center gap-2">
        <span className="mono text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
          {title}
        </span>
        <div className="flex-1" />
        {right}
      </div>
      {children}
    </div>
  );
}

const linkStyle: React.CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  cursor: "pointer",
  color: "var(--fg)",
  font: "inherit",
  textDecoration: "underline",
  textDecorationColor: "var(--line-2)",
  textUnderlineOffset: 2,
};

const LIFECYCLE_TONE: Record<string, string> = {
  launched: "var(--src-engram)",
  owned: "var(--src-qryx)",
  transferred: "var(--iris)",
  budget_set: "var(--amber)",
  closed: "var(--sev-critical)",
};

const fieldStyle: React.CSSProperties = {
  width: "100%",
  background: "var(--panel)",
  border: "1px solid var(--line-2)",
  borderRadius: 8,
  padding: "7px 9px",
  fontSize: 12,
  color: "var(--fg)",
};

function ApplyRow({ onApply, onCancel }: { onApply: () => void; onCancel: () => void }) {
  return (
    <div className="flex items-center gap-2" style={{ marginTop: 8 }}>
      <button type="button" onClick={onApply} style={{ padding: "5px 12px", borderRadius: 7, cursor: "pointer", border: "1px solid var(--iris)", background: "color-mix(in srgb, var(--iris) 18%, transparent)", color: "var(--fg)", fontSize: 12 }}>
        Apply
      </button>
      <button type="button" onClick={onCancel} style={{ padding: "5px 10px", borderRadius: 7, cursor: "pointer", border: "1px solid var(--line-2)", background: "var(--panel)", color: "var(--dim)", fontSize: 12 }}>
        Cancel
      </button>
    </div>
  );
}

function SelectForm({
  options,
  current,
  labelFor,
  onApply,
  onCancel,
}: {
  options: string[];
  current: string;
  /** Optional display-text mapping - the "Reassign unit" picker passes
   * `prettyUnit` so the dropdown shows business-unit names instead of raw
   * slugs; every other caller (owner handles) omits it and gets the raw
   * value back, unchanged. The submitted/current VALUE is always the raw
   * option string either way - only the rendered `<option>` text changes. */
  labelFor?: (value: string) => string;
  onApply: (v: string) => void;
  onCancel: () => void;
}) {
  const [v, setV] = useState(current);
  return (
    <div>
      <select style={fieldStyle} value={v} onChange={(e) => setV(e.target.value)}>
        {options.map((o) => (
          <option key={o} value={o}>
            {labelFor ? labelFor(o) : o}
          </option>
        ))}
      </select>
      <ApplyRow onApply={() => onApply(v)} onCancel={onCancel} />
    </div>
  );
}

function NumberForm({ current, onApply, onCancel }: { current: number; onApply: (v: number) => void; onCancel: () => void }) {
  const [v, setV] = useState(String(current));
  return (
    <div>
      <input style={fieldStyle} value={v} inputMode="decimal" onChange={(e) => setV(e.target.value)} />
      <ApplyRow onApply={() => onApply(Number(v) || current)} onCancel={onCancel} />
    </div>
  );
}

function BehaviourForm({ current, onApply, onCancel }: { current: string[]; onApply: (v: string[]) => void; onCancel: () => void }) {
  const [v, setV] = useState(current.join("\n"));
  return (
    <div>
      <textarea
        style={{ ...fieldStyle, minHeight: 92, resize: "vertical", fontFamily: "inherit" }}
        value={v}
        onChange={(e) => setV(e.target.value)}
      />
      <div className="text-[10.5px]" style={{ color: "var(--faint)", marginTop: 3 }}>
        one rule per line
      </div>
      <ApplyRow onApply={() => onApply(v.split("\n").map((s) => s.trim()).filter(Boolean))} onCancel={onCancel} />
    </div>
  );
}

const ACTION_LABELS: [string, string][] = [
  ["owner", "Transfer owner"],
  ["unit", "Reassign unit"],
  ["budget", "Edit budget"],
  ["behaviour", "Edit behaviour"],
];

export function AgentDetailCard({
  agentId,
  onOpenFull,
  onReplay,
}: {
  agentId: string;
  onOpenFull?: (agentId: string) => void;
  onReplay?: (runId: string) => void;
}) {
  const { open } = usePopover();
  const win = useWindowControls();
  const [record, setRecord] = useState<AgentRecord | null | undefined>(undefined);
  const [runs, setRuns] = useState<Run[]>([]);
  const [action, setAction] = useState<null | "owner" | "unit" | "budget" | "behaviour">(null);
  const [dir, setDir] = useState<OrgDirectory | null>(null);
  const [identities, setIdentities] = useState<IdryxIdentity[]>([]);
  // Bumps whenever any lifecycle action lands anywhere, so this card re-reads
  // (a unit/user stop from the dock reflects here even while it is open).
  const consoleVersion = useConsoleStateVersion();

  useEffect(() => {
    void fetchOrgDirectory().then(setDir);
  }, []);

  const applyRecord = (rec: AgentRecord | null) => {
    if (rec) setRecord(rec);
    setAction(null);
  };

  useEffect(() => {
    let cancelled = false;
    void fetchAgentRecord(agentId).then((r) => !cancelled && setRecord(r));
    void fetchRuns().then((r) => !cancelled && setRuns(r.filter((x) => x.agent_id === agentId))).catch(() => {});
    void fetchIdentities().then((i) => !cancelled && setIdentities(i)).catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [agentId, consoleVersion]);

  const run = runs[0];
  const spent = record?.spentUsd ?? runs.reduce((s, r) => s + r.spent_usd, 0);
  // Prefer the run's window budget so utilisation here agrees with the Money
  // tab; the per-run ceiling (the record's own budget) is a separate line.
  const budget = run?.budget_usd ?? record?.budgetUsd ?? null;
  const perRunCap = record?.budgetUsd ?? null;
  const util = budget && budget > 0 ? spent / budget : null;
  const closed = Boolean(record?.closed);
  // The effective lifecycle state (LIVE/STOPPED/FROZEN/KILLED) and the live run
  // the Kill button targets, from the same shared derivation every surface uses.
  const serverBlocks = useLifecycleBlocks();
  const serverFrozen = serverBlocks.agents.includes(agentId);
  const state = serverFrozen ? "frozen" : agentStateFromRecord(record ?? null);
  const frozen = state === "frozen";
  const liveRun = runs.find((r) => !r.killed) ?? null;
  // The HUMAN owner (a person, with a surname) is this agent's own idryx
  // identity owner, NOT a delegation parent: a parent is another AGENT, so
  // slice.parents[0] wrongly surfaced an agent id (e.g. cashflow-forecaster)
  // in the "owner" row. Fall back to the mock record's owner in preview.
  const identityOwner = identities.find((i) => i.id === agentId)?.owner;
  const owner = record
    ? userHandle(`user://x/${record.owner}`)
    : identityOwner
      ? userHandle(identityOwner)
      : null;
  const teamSeg = record?.team ?? (/^agent:\/\/[^/]+\/([^/]+)\//.exec(agentId)?.[1] ?? null);
  const unit = teamSeg ? unitForTeam(teamSeg) : null;

  return (
    <div className="flex flex-col">
      <PopoverHeader kicker="Agent" title={shortAgentLabel(agentId)} />
      <div style={{ padding: "0 16px 8px" }}>
        <span className="mono text-[10.5px] break-all" style={{ color: "var(--faint)" }}>
          {agentId}
        </span>
      </div>

      {/* Who and what - unit and owner drill into their own cards */}
      <Section title="Ownership">
        {unit && (
          <Row label="business unit">
            <button
              type="button"
              style={linkStyle}
              onClick={(e) => open(<UnitCard team={unit} onOpenFullAgent={onOpenFull} />, { anchor: e.currentTarget.getBoundingClientRect() })}
            >
              {prettyUnit(unit)} &rsaquo;
            </button>
          </Row>
        )}
        {owner && (
          <Row label="owner">
            <button
              type="button"
              style={linkStyle}
              onClick={(e) => open(<UserCard handle={owner} onOpenFullAgent={onOpenFull} />, { anchor: e.currentTarget.getBoundingClientRect() })}
            >
              {owner} &rsaquo;
            </button>
          </Row>
        )}
        {record && <Row label="model">{record.model}</Row>}
        <Row label="status">
          <StateBadge state={state} />
        </Row>
      </Section>

      {/* Money */}
      <Section title="Spend">
        <Row label="spent">{formatUsd(spent)}</Row>
        {budget !== null && <Row label="budget">{formatUsd(budget)}</Row>}
        {perRunCap !== null && perRunCap !== budget && <Row label="per-run cap">{formatUsd(perRunCap)}</Row>}
        {util !== null && (
          <Row label="utilisation">
            <span style={{ color: util >= 1 ? "var(--sev-critical)" : util >= 0.8 ? "var(--amber)" : "var(--mint)" }}>
              {Math.round(util * 100)}%
            </span>
          </Row>
        )}
        {record && <Row label="calls">{record.calls.toLocaleString("en-US")}</Row>}
      </Section>

      {/* Spend attribution: how the total splits across ownership periods */}
      {record && record.segments && record.segments.length > 1 && (
        <Section title="Spend by owner">
          <div className="flex flex-col gap-1" style={{ paddingTop: 2 }}>
            {record.segments.map((s, i) => (
              <div key={i} className="flex items-center gap-2 min-w-0">
                <span className="text-[11.5px] truncate" style={{ color: "var(--fg)", flex: 1 }}>
                  {s.owner} <span style={{ color: "var(--faint)" }}>· {prettyUnit(unitForTeam(s.team))}</span>
                </span>
                {s.to === null && (
                  <span className="badge" style={cssVar("tone", "var(--sev-low)")}>
                    current
                  </span>
                )}
                <span className="mono tabular text-[11.5px]" style={{ color: "var(--dim)" }}>
                  {formatUsd(s.spentUsd)}
                </span>
              </div>
            ))}
          </div>
          <div className="text-[10px]" style={{ color: "var(--faint)", paddingTop: 4 }}>
            total {formatUsd(spent)} across {record.segments.length} ownership periods
          </div>
        </Section>
      )}

      {/* Allowed behaviour envelope */}
      {record && record.allowed.length > 0 && (
        <Section title="Allowed behaviour">
          <div className="flex flex-wrap gap-1.5">
            {record.allowed.map((a, i) => (
              <span key={i} className="chip text-[11px]" style={{ color: "var(--dim)" }}>
                {a}
              </span>
            ))}
          </div>
        </Section>
      )}

      {/* Why it was closed, if it was */}
      {record?.closed && (
        <Section title="Closure">
          <Row label="closed by">{record.closed.by}</Row>
          <Row label="reason">{record.closed.reason}</Row>
          <div className="text-[11.5px]" style={{ color: "var(--dim)", paddingTop: 2 }}>
            {record.closed.wrongdoing}
          </div>
        </Section>
      )}

      {/* Lifecycle timeline */}
      {record && record.history.length > 0 && (
        <Section title="Lifecycle">
          <div className="flex flex-col gap-1.5" style={{ paddingTop: 2 }}>
            {[...record.history]
              .sort((a, b) => a.ts.localeCompare(b.ts))
              .map((h, i) => (
                <div key={i} className="flex items-start gap-2 min-w-0">
                  <span
                    aria-hidden="true"
                    style={{
                      width: 7,
                      height: 7,
                      borderRadius: "50%",
                      marginTop: 4,
                      background: LIFECYCLE_TONE[h.kind] ?? "var(--faint)",
                      flexShrink: 0,
                    }}
                  />
                  <div className="flex flex-col min-w-0">
                    <span className="text-[11.5px]" style={{ color: "var(--fg)" }}>
                      {h.detail}
                    </span>
                    <span className="mono text-[10px]" style={{ color: "var(--faint)" }}>
                      {h.kind} · {h.actor} · {formatTimestamp(h.ts)}
                    </span>
                  </div>
                </div>
              ))}
          </div>
        </Section>
      )}

      {/* Governance actions: reassign, transfer, edit budget/behaviour */}
      {record && (
        <Section title="Manage">
          {!closed && (
            <div className="flex items-center gap-2" style={{ paddingBottom: 8 }}>
              <FreezeToggleButton
                frozen={frozen}
                onToggle={() => blockAgent(agentId, !frozen).then(applyRecord)}
              />
              <KillRunButton
                run={liveRun}
                detail={liveRun ? `run ${liveRun.run_id} · spent ${formatUsd(liveRun.spent_usd)}` : undefined}
                onKill={(runId, reason) => killRun(runId, reason).then(() => {})}
              />
            </div>
          )}
          <div className="flex flex-wrap gap-1.5">
            {ACTION_LABELS.map(([k, label]) => {
              const on = action === k;
              return (
                <button
                  key={k}
                  type="button"
                  onClick={() => setAction(on ? null : (k as typeof action))}
                  className="text-[11px]"
                  style={{
                    padding: "4px 10px",
                    borderRadius: 6,
                    cursor: "pointer",
                    border: `1px solid ${on ? "var(--iris)" : "var(--line-2)"}`,
                    background: on ? "color-mix(in srgb, var(--iris) 14%, transparent)" : "var(--panel)",
                    color: on ? "var(--fg)" : "var(--dim)",
                  }}
                >
                  {label}
                </button>
              );
            })}
          </div>
          {action && (
            <div style={{ marginTop: 8 }}>
              {action === "owner" && (
                <SelectForm
                  options={dir ? dir.users.map((u) => u.handle) : [record.owner]}
                  current={record.owner}
                  onCancel={() => setAction(null)}
                  onApply={(v) => void transferAgentOwner(agentId, v).then(applyRecord)}
                />
              )}
              {action === "unit" && (
                <SelectForm
                  options={dir ? dir.teams.map((t) => t.team) : [record.team]}
                  current={record.team}
                  labelFor={prettyUnit}
                  onCancel={() => setAction(null)}
                  onApply={(v) => void reassignAgentUnit(agentId, v).then(applyRecord)}
                />
              )}
              {action === "budget" && (
                <NumberForm current={record.budgetUsd} onCancel={() => setAction(null)} onApply={(v) => void setAgentBudget(agentId, v).then(applyRecord)} />
              )}
              {action === "behaviour" && (
                <BehaviourForm current={record.allowed} onCancel={() => setAction(null)} onApply={(v) => void setAgentBehaviour(agentId, v).then(applyRecord)} />
              )}
            </div>
          )}
          {record === null && (
            <span className="text-[10.5px]" style={{ color: "var(--faint)" }}>
              editing needs a records store, which this box does not keep.
            </span>
          )}
        </Section>
      )}

      {/* Runs + links out */}
      <Section title="Runs" right={<span className="mono text-[10px]" style={{ color: "var(--faint)" }}>{runs.length}</span>}>
        {runs.length === 0 ? (
          <span className="text-[11.5px]" style={{ color: "var(--faint)" }}>
            no runs for this agent yet.
          </span>
        ) : (
          <div className="flex flex-col gap-1">
            {runs.slice(0, 4).map((r) => (
              <div key={r.run_id} className="flex items-center gap-2 min-w-0">
                <span className="mono text-[11px] truncate" style={{ color: "var(--dim)", flex: 1 }} title={r.run_id}>
                  {r.run_id}
                </span>
                <span className="mono tabular text-[11px]" style={{ color: "var(--fg)" }}>
                  {formatUsd(r.spent_usd)}
                </span>
                {onReplay && (
                  <button
                    type="button"
                    className="icon-btn"
                    style={{ width: "auto", padding: "0 8px", fontSize: 10.5 }}
                    onClick={() => {
                      onReplay(r.run_id);
                      win?.close();
                    }}
                  >
                    Replay
                  </button>
                )}
              </div>
            ))}
          </div>
        )}
      </Section>

      <div className="flex items-center gap-2" style={{ padding: "10px 16px 14px", borderTop: "1px solid var(--line)" }}>
        {onOpenFull && (
          <button
            type="button"
            className="icon-btn"
            style={{ width: "auto", padding: "0 12px", fontSize: 11.5 }}
            onClick={() => {
              onOpenFull(agentId);
              win?.close();
            }}
          >
            Open full 360 &rarr;
          </button>
        )}
        {record === null && (
          <span className="text-[10.5px]" style={{ color: "var(--faint)" }}>
            ownership + lifecycle need a records store (not on this box)
          </span>
        )}
      </div>
    </div>
  );
}
