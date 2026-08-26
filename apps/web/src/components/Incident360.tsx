import { useEffect, useMemo, useState, type ReactNode } from "react";
import { SeverityBadge } from "./SeverityBadge";
import { UserCard } from "./UserCard";
import { UnitCard } from "./UnitCard";
import { usePopover } from "../lib/popover";
import { fetchRuns } from "../lib/money";
import { fetchAgentRecord, type AgentRecord } from "../lib/agentRecord";
import { fetchRecentEvents } from "../lib/recentEvents";
import { formatUsd } from "../lib/format";
import { shortAgentLabel } from "../lib/graph";
import { sevColor } from "../lib/dashData";
import {
  incidentData,
  incidentDelegation,
  incidentRunId,
  incidentSubject,
  INCIDENT_SOURCE_LABEL,
  type UnifiedIncident,
} from "../lib/incidents";
import type { Run } from "../moneyTypes";
import type { UiEvent } from "../types";
import type { ViewId } from "../lib/views";

/** The same window every other bus read in this console uses. */
const BUS_FETCH_LIMIT = 500;

/**
 * Incident 360: everything this console can say about one anomaly.
 *
 * `@yurii` 2026-08-26, after the first attempt opened an agent card instead:
 * "має бути така сама детальна картка аномалії чи інцидент 360. Також
 * максимально зі всіх боків має бути освітлено... а не просто, хто її зробив,
 * і наскільки це серйозно."
 *
 * The shape is Agent 360's, deliberately: a wide card in the overlay layer,
 * closable, section-headed, opening other cards beside itself rather than
 * navigating away. An operator who has learned one has learned both.
 *
 * # The seven questions, and where each answer comes from
 *
 *   what happened      the producer's own type and `data`
 *   which agent        the event's subject, opening Agent 360 beside this card
 *   who answers for it `AgentRecord.owner`, its team and its allowed behaviour
 *   who asked          `on_behalf_of`, root first, where a `user://` sits when
 *                      a person started the work
 *   was it stopped     three separate facts, kept separate: whether THIS event
 *                      was a refusal, whether the RUN was killed, and whether
 *                      the AGENT is blocked now
 *   what it cost       the run's spend against its budget, its calls, its steps
 *   what led to it     every other event in the same run, in time order, with
 *                      this one marked
 *
 * # The last section is the one worth having and the one with a limit
 *
 * "What led to it" is as close as this estate gets to "what was it doing
 * before it went wrong", and it is a genuine answer: the run's own events, in
 * order, are what the agent did. It is NOT the prompts. No plane in this
 * estate stores prompt content: the gateway's trace carries model, tokens,
 * cost and decision and no text, the envelope carries none, and heraldyx says
 * in its own mail that it sends identifiers and numbers and never the content
 * of a call. So this section answers "what did it do" and cannot answer "what
 * was it told". That limit is printed in the section rather than left for
 * somebody to assume the timeline is complete.
 *
 * # Every section renders even when empty
 *
 * genaryx invariant 4, and this card is where it is easiest to lose: it asks
 * seven questions and a real box will not answer all seven for every row. A
 * posture finding has no run, an identity alert has no delegation chain, a
 * fleet-wide event has no subject. A section that vanished would read as a
 * question nobody asked, and the point of this card is that they are all asked
 * every time.
 */
export function Incident360({
  row,
  onClose,
  onOpenAgent,
  onNavigate,
}: {
  row: UnifiedIncident;
  onClose: () => void;
  onOpenAgent: (agentId: string) => void;
  onNavigate?: (view: ViewId) => void;
}) {
  const { open } = usePopover();
  const subject = incidentSubject(row);
  const chain = incidentDelegation(row);
  const data = incidentData(row);
  const runId = incidentRunId(row);

  const [run, setRun] = useState<Run | null>(null);
  const [record, setRecord] = useState<AgentRecord | null>(null);
  const [busEvents, setBusEvents] = useState<UiEvent[] | null>(null);
  // "Still reading" and "asked, and there is none" are different answers and
  // the card says which. A blank section that might fill in a moment teaches
  // an operator to distrust the blank ones that will not.
  const [runAsked, setRunAsked] = useState(false);
  const [recordAsked, setRecordAsked] = useState(false);

  useEffect(() => {
    if (!runId) {
      setRunAsked(true);
      return;
    }
    let cancelled = false;
    void fetchRuns()
      .then((runs) => !cancelled && setRun(runs.find((r) => r.run_id === runId) ?? null))
      .catch(() => undefined)
      .finally(() => !cancelled && setRunAsked(true));
    return () => {
      cancelled = true;
    };
  }, [runId]);

  useEffect(() => {
    if (!subject) {
      setRecordAsked(true);
      return;
    }
    let cancelled = false;
    void fetchAgentRecord(subject)
      .then((r) => !cancelled && setRecord(r))
      .catch(() => undefined)
      .finally(() => !cancelled && setRecordAsked(true));
    return () => {
      cancelled = true;
    };
  }, [subject]);

  useEffect(() => {
    if (!runId) return;
    let cancelled = false;
    void fetchRecentEvents(BUS_FETCH_LIMIT)
      .then((res) => !cancelled && setBusEvents(res.events))
      .catch(() => !cancelled && setBusEvents([]));
    return () => {
      cancelled = true;
    };
  }, [runId]);

  /** This run's own events, oldest first, so the row above the incident is
   * what came before it rather than after. */
  const timeline = useMemo(() => {
    if (!runId || busEvents === null) return null;
    return busEvents
      .filter((e) => e.run_id === runId)
      .slice()
      .sort((a, b) => Date.parse(a.ts) - Date.parse(b.ts));
  }, [busEvents, runId]);

  /** Every other agent that appears in this run or in its delegation chain.
   * "Who else this touches", which for a delegated run is rarely just one. */
  const alsoTouched = useMemo(() => {
    const out = new Set<string>();
    for (const id of chain) if (id.startsWith("agent://") && id !== subject) out.add(id);
    for (const e of timeline ?? []) if (e.agent_id && e.agent_id !== subject) out.add(e.agent_id);
    return [...out].sort();
  }, [chain, timeline, subject]);

  const thisEventId = row.source === "bus" || row.source === "verdryx" ? row.raw.id : null;

  /** What THIS event says it cost, as opposed to what the run cost.
   *
   * `@yurii` asked for the split: "на якій події вона була зупинена, і саме ця
   * подія скільки забрала коштів". tokenfuse writes `spent_usd` and
   * `budget_usd` onto a refusal, so the answer is on the event itself and the
   * run total is a different number about a longer window. Showing only the
   * run total answered "what did this agent cost today" to somebody asking
   * "what did THIS cost". */
  const eventMoney = useMemo(() => moneyFrom(data), [data]);

  /** The first refusal in the run, which is the point the agent got no
   * further than, and not necessarily the row somebody clicked. */
  const firstRefusal = useMemo(() => {
    for (const e of timeline ?? []) {
      const d = e.data && typeof e.data === "object" ? (e.data as Record<string, unknown>) : null;
      if (refusalFrom(d)) return { event: e, money: moneyFrom(d) };
    }
    return null;
  }, [timeline]);

  return (
    <div
      className="flex flex-col min-h-0"
      style={{
        width: "min(720px, 94vw)",
        maxHeight: "100%",
        background: "var(--panel)",
        border: "1px solid var(--line)",
        borderRadius: 8,
        overflow: "hidden",
      }}
    >
      <header
        className="flex items-start gap-3"
        style={{ padding: "14px 16px 10px", borderBottom: "1px solid var(--line)" }}
      >
        <div className="flex-1 min-w-0">
          <div className="mono" style={{ fontSize: 9.5, color: "var(--faint)", letterSpacing: "0.06em" }}>
            INCIDENT 360 ·{" "}
            {row.source === "bus"
              ? `VIA ${((row.raw as { source?: string }).source ?? "BUS").toUpperCase()}`
              : INCIDENT_SOURCE_LABEL[row.source].toUpperCase()}
          </div>
          <div style={{ fontSize: 15, fontWeight: 600 }}>{row.title}</div>
          <div className="flex items-center gap-2 flex-wrap" style={{ marginTop: 6 }}>
            <SeverityBadge severity={row.severity} />
            {row.ts && (
              <span className="mono text-[10.5px]" style={{ color: "var(--faint)" }}>
                {row.ts}
              </span>
            )}
            {row.occurrences !== undefined && row.occurrences > 1 && (
              <span className="mono text-[10.5px]" style={{ color: "var(--faint)" }}>
                {row.occurrences}× in this window
              </span>
            )}
          </div>
        </div>
        <button type="button" className="icon-btn" onClick={onClose} aria-label="Close">
          ×
        </button>
      </header>

      <div className="flex-1 min-h-0 overflow-y-auto thin-scroll" style={{ padding: "10px 0 16px" }}>
        <Q label="What happened">{row.detail}</Q>

        <Q label="Which agent">
          {subject ? (
            <button
              type="button"
              className="mono"
              style={{ cursor: "pointer", color: "var(--iris)", fontSize: 11.5, textAlign: "left" }}
              title="Open this agent beside the incident"
              onClick={() => onOpenAgent(subject)}
            >
              {subject}
            </button>
          ) : null}
        </Q>

        <Q label="Who answers for it">
          {!recordAsked ? undefined : record ? (
            <span className="flex items-center gap-1.5 flex-wrap">
              {/* Both open their own card rather than reading as text. An
                  operator who has just been told who answers for an agent
                  wants that person's other agents and their spend, and the
                  console already holds both; a plain string made them go and
                  look for it by hand. */}
              {record.owner ? (
                <button
                  type="button"
                  className="chip"
                  style={{ cursor: "pointer" }}
                  title="Open this owner"
                  onClick={() => open(<UserCard handle={record.owner} onOpenFullAgent={onOpenAgent} />)}
                >
                  {record.owner}
                </button>
              ) : (
                <span className="chip">no owner recorded</span>
              )}
              {record.team ? (
                <button
                  type="button"
                  className="chip"
                  style={{ cursor: "pointer" }}
                  title="Open this unit"
                  onClick={() => open(<UnitCard team={record.team} onOpenFullAgent={onOpenAgent} />)}
                >
                  {record.team}
                </button>
              ) : null}
              {record.allowed?.length ? (
                <span className="mono" style={{ fontSize: 10.5, color: "var(--faint)" }}>
                  allowed: {record.allowed.join(", ")}
                </span>
              ) : null}
            </span>
          ) : null}
        </Q>

        <Q
          label="Who asked for the work"
          note={
            chain.length === 0 && (row.source === "money" || row.source === "idryx")
              ? "this source carries no delegation chain; only bus events do"
              : undefined
          }
        >
          {chain.length > 0 ? (
            <span className="flex items-center gap-1.5 flex-wrap">
              {chain.map((id, i) => (
                <span key={`${id}:${i}`} className="flex items-center gap-1.5">
                  {i > 0 && <span style={{ color: "var(--faint)" }}>-&gt;</span>}
                  {id.startsWith("agent://") ? (
                    <button
                      type="button"
                      className="chip"
                      style={{ cursor: "pointer" }}
                      onClick={() => onOpenAgent(id)}
                    >
                      {shortAgentLabel(id)}
                    </button>
                  ) : (
                    <span className="chip">{id}</span>
                  )}
                </span>
              ))}
            </span>
          ) : null}
        </Q>

        <Q label="Was it stopped">{stoppedAnswer(run, record, data, runAsked, recordAsked)}</Q>

        <Q
          label="What this event cost"
          note={
            "the amounts the producer recorded ON THIS EVENT, which is what the " +
            "run had spent at the moment it was refused. Not the run's total: " +
            "see the section below."
          }
        >
          {eventMoney ? (
            <span>
              {eventMoney.spent !== null ? `${formatUsd(eventMoney.spent)} spent by this point` : ""}
              {eventMoney.budget !== null ? ` of ${formatUsd(eventMoney.budget)}` : ""}
              {eventMoney.overBy !== null ? ` · over by ${formatUsd(eventMoney.overBy)}` : ""}
              {eventMoney.reason ? ` · ${eventMoney.reason}` : ""}
            </span>
          ) : null}
        </Q>

        <Q label="What the whole run cost">
          {!runAsked ? undefined : run ? (
            <span>
              {formatUsd(run.spent_usd)} spent
              {run.budget_usd !== null ? ` of ${formatUsd(run.budget_usd)}` : " (no budget set)"}
              {` · ${run.calls} call(s) · ${run.steps} step(s)`}
              <br />
              <span className="mono" style={{ fontSize: 10.5, color: "var(--faint)" }}>
                model {run.model}
                {run.unit ? ` · unit ${run.unit}` : " · no unit resolved"}
              </span>
            </span>
          ) : null}
        </Q>

        <Q
          label="Where it was stopped"
          note={
            "the first refusal in this run, which is the event the agent got no " +
            "further than. It is not always the one above: a run can be refused " +
            "long before the incident somebody opened."
          }
        >
          {timeline === null ? undefined : firstRefusal ? (
            <span>
              <span className="chip">{firstRefusal.event.source}</span>{" "}
              {firstRefusal.event.type.replace(/_/g, " ")} at{" "}
              <span className="mono">{firstRefusal.event.ts.slice(11, 19)}</span>
              {firstRefusal.money?.spent !== null && firstRefusal.money !== null
                ? ` · ${formatUsd(firstRefusal.money.spent as number)} spent by then`
                : ""}
              {firstRefusal.event.id === thisEventId ? " · this is the one you opened" : ""}
            </span>
          ) : null}
        </Q>

        <Q
          label="What led to it"
          note={
            "this run's own events, oldest first. It is what the agent DID; no plane in this " +
            "estate stores what it was told, so the prompts are not here and are not missing " +
            "by accident."
          }
        >
          {timeline === null ? undefined : timeline.length > 0 ? (
            <div className="flex flex-col" style={{ gap: 3, marginTop: 4 }}>
              {timeline.map((e) => (
                <div
                  key={e.id}
                  className="flex items-center gap-2"
                  style={{
                    fontSize: 11,
                    opacity: thisEventId !== null && e.id === thisEventId ? 1 : 0.72,
                    fontWeight: thisEventId !== null && e.id === thisEventId ? 600 : 400,
                  }}
                >
                  <span
                    className="dot"
                    aria-hidden="true"
                    style={{ background: sevColor(e.severity ?? "info"), flex: "0 0 auto" }}
                  />
                  <span className="mono" style={{ fontSize: 10, color: "var(--faint)" }}>
                    {e.ts.slice(11, 19)}
                  </span>
                  <span className="chip">{e.source}</span>
                  <span className="truncate">{e.type.replace(/_/g, " ")}</span>
                  {thisEventId !== null && e.id === thisEventId && (
                    <span className="mono" style={{ fontSize: 9.5, color: "var(--iris)" }}>
                      THIS ONE
                    </span>
                  )}
                </div>
              ))}
            </div>
          ) : null}
        </Q>

        <Q label="Who else this touches">
          {alsoTouched.length > 0 ? (
            <span className="flex items-center gap-1.5 flex-wrap">
              {alsoTouched.map((id) => (
                <button
                  key={id}
                  type="button"
                  className="chip"
                  style={{ cursor: "pointer" }}
                  onClick={() => onOpenAgent(id)}
                >
                  {shortAgentLabel(id)}
                </button>
              ))}
            </span>
          ) : null}
        </Q>

        <Q label="What the producer recorded">
          {data && Object.keys(data).length > 0 ? (
            <div className="flex flex-col" style={{ gap: 2 }}>
              {Object.entries(data).map(([k, v]) => (
                <div key={k} className="mono" style={{ fontSize: 10.5 }}>
                  <span style={{ color: "var(--faint)" }}>{k}</span>{" "}
                  {typeof v === "object" ? JSON.stringify(v) : String(v)}
                </div>
              ))}
            </div>
          ) : null}
        </Q>

        {onNavigate && (
          <div style={{ padding: "6px 16px 0" }}>
            <button
              type="button"
              className="chip"
              style={{ cursor: "pointer" }}
              onClick={() => onNavigate("bus")}
            >
              open the raw stream
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

/** One question, its answer, and the fact when there is none.
 *
 * `undefined` means still reading; `null` or empty means asked and absent.
 * Two different sentences, because a card that says "not recorded" while a
 * fetch is in flight is lying for as long as the fetch takes. */
function Q({ label, note, children }: { label: string; note?: string; children?: ReactNode }) {
  const reading = children === undefined;
  const empty = children === null || children === "";
  return (
    <div style={{ padding: "0 16px 11px" }}>
      <div className="mono" style={{ fontSize: 9.5, color: "var(--faint)", letterSpacing: "0.06em" }}>
        {label.toUpperCase()}
      </div>
      <div style={{ fontSize: 12, lineHeight: 1.45, color: reading || empty ? "var(--faint)" : undefined }}>
        {reading ? "reading…" : empty ? "not recorded for this incident" : children}
      </div>
      {note && (
        <div className="mono" style={{ fontSize: 9.5, color: "var(--faint)", marginTop: 3, lineHeight: 1.4 }}>
          {note}
        </div>
      )}
    </div>
  );
}

/**
 * Whether anything stopped this, in the words of what actually stopped it.
 *
 * Three separate facts, deliberately not merged into one badge: THIS EVENT may
 * itself have been a refusal, the RUN may have been killed, and the AGENT may
 * be blocked now. An operator asking "was it stopped before it did the
 * anomalous thing" is asking the first; answering with the third would tell
 * them the agent is quiet today about a run that spent a budget yesterday.
 */
function stoppedAnswer(
  run: Run | null,
  record: AgentRecord | null,
  data: Record<string, unknown> | null,
  runAsked: boolean,
  recordAsked: boolean,
): ReactNode {
  if (!runAsked || !recordAsked) return undefined;
  const said: string[] = [];
  const refusal = refusalFrom(data);
  if (refusal) said.push(`this event is itself a refusal (${refusal})`);
  if (run?.killed) said.push("the run was killed");
  if (record?.blocked) said.push("the agent is blocked now");
  if (said.length > 0) return said.join(" · ");
  if (run !== null || record !== null) {
    return "nothing stopped it: this event is not a refusal, the run was not killed, and the agent is not blocked";
  }
  return null;
}


/** The money a producer recorded on one event, where it recorded any.
 *
 * `budget_usd` and `spent_usd` are what tokenfuse writes onto a refusal
 * (`crates/gateway/src/proxy.rs`, the breaker emit). Read best-effort and only
 * as numbers: a producer that writes a string there is reporting something
 * this console does not understand, and rendering it as money would be the
 * console asserting a figure it did not read. */
function moneyFrom(
  data: Record<string, unknown> | null,
): { spent: number | null; budget: number | null; overBy: number | null; reason: string | null } | null {
  if (!data) return null;
  const num = (k: string) => (typeof data[k] === "number" ? (data[k] as number) : null);
  const spent = num("spent_usd");
  const budget = num("budget_usd");
  if (spent === null && budget === null) return null;
  const reason = typeof data.reason === "string" ? (data.reason as string) : null;
  return {
    spent,
    budget,
    overBy: spent !== null && budget !== null && spent > budget ? spent - budget : null,
    reason,
  };
}

/** The members a producer uses to say "this was refused, and here is why".
 * Read best-effort and never turned into a claim of this console's own: it
 * reports what the producer wrote. */
function refusalFrom(data: Record<string, unknown> | null): string | null {
  if (!data) return null;
  for (const key of ["decision", "effect", "verdict", "reason"]) {
    const v = data[key];
    if (typeof v === "string" && v) return `${key}: ${v}`;
  }
  return null;
}
