import type { CSSProperties, ReactNode } from "react";
import { useEffect, useState } from "react";
import type { AgentSlice } from "../graphTypes";
import { ATTESTATION_DETECTORS } from "../identityTypes";
import type { IdentityError, IdryxAlert, IdryxIdentity } from "../identityTypes";
import { cssVar } from "../lib/cssVars";
import { sevRank, spendSeries } from "../lib/dashData";
import { formatTimestamp, formatUsd } from "../lib/format";
import { fetchAgentEvents, fetchAgentSlice, shortAgentLabel } from "../lib/graph";
import { describeIdentityError, fetchAlerts, fetchIdentities } from "../lib/identity";
import { isQualityDriftEvent } from "../lib/incidents";
import { describeMoneyError, fetchRuns, killRun } from "../lib/money";
import { FreezeToggleButton, KillRunButton, runBlockedState, StateBadge } from "../lib/lifecycle";
import { useLifecycleBlocks } from "../lib/lifecycleBlocks";
import { useConsoleStateVersion } from "../lib/consoleState";
import { blockAgent } from "../lib/agentActions";
import { describePolicyError, fetchApprovals, fetchPolicies } from "../lib/policy";
import { effectiveOverlay, matchedPolicies, mcpReachForAgent, mcpServerIdentities, permissionRollup, shadowServerIds } from "../lib/access";
import { useIdentityStatus } from "../lib/useIdentityStatus";
import { useMoneyStatus } from "../lib/useMoneyStatus";
import { usePolicyStatus } from "../lib/usePolicyStatus";
import { usePopover } from "../lib/popover";
import { prettyUnit, unitForTeam } from "../lib/views";
import type { ViewId } from "../lib/views";
import type { MoneyError, Run } from "../moneyTypes";
import type { Approval, PolicyError, PolicyRecord, PolicyStatus } from "../policyTypes";
import type { UiEvent } from "../types";
import { DelegationGraphView } from "./DelegationGraphView";
import { Sparkline } from "./dash";
import { SeverityBadge } from "./SeverityBadge";
import { SourceChip } from "./SourceChip";
import { AgentDetailCard } from "./AgentDetailCard";
import { UnitCard } from "./UnitCard";
import { WatchToggleButton } from "./WatchDock";

/** How many events/policy rows this compact card shows inline before
 * pointing at the fuller panel instead of growing without bound. */
const EVENTS_LIMIT = 50;
const EVENTS_SHOWN = 12;
const POLICY_EVENTS_SHOWN = 8;
const IDENTITY_ALERTS_SHOWN = 6;
/** I11 "Drift" section: how many older drift checks show below the latest,
 * and how many behavior-anomaly alerts show before pointing at the Identity
 * panel instead - same "compact card, not a full panel" budget as the
 * constants above. */
const DRIFT_CHECKS_SHOWN = 6;
const BEHAVIOR_ALERTS_SHOWN = 6;

/** Best-effort read of one field out of an event's untyped `data` payload -
 * never assumes the shape, never throws on a missing/malformed field (the
 * core keeps `data` deliberately untyped end to end). Duplicated rather than
 * imported from `QualityDriftStream.tsx`'s identical pair (and
 * `lib/incidents.ts`'s private copy) ON PURPOSE - the same "two independent
 * literals, not worth a shared dependency" call `lib/incidents.ts`'s own doc
 * comment already makes for this exact pair. */
function dataNumber(data: unknown, key: string): number | null {
  if (data && typeof data === "object" && key in (data as Record<string, unknown>)) {
    const value = (data as Record<string, unknown>)[key];
    if (typeof value === "number") return value;
  }
  return null;
}

function dataString(data: unknown, key: string): string | null {
  if (data && typeof data === "object" && key in (data as Record<string, unknown>)) {
    const value = (data as Record<string, unknown>)[key];
    if (typeof value === "string") return value;
  }
  return null;
}

/** One `quality_drift` bus event's fields, pulled out of its untyped `data`
 * once so the render below reads plain properties instead of re-parsing
 * `data` per field per row. `baselineN`/`tStatistic`/`ciLow`/`ciHigh` are
 * `null` together whenever the event predates them or the field is
 * malformed - never assumed present, matching this whole card's tolerance
 * for an honestly-incomplete upstream payload. */
interface DriftReading {
  id: number;
  ts: string;
  verdict: string | null;
  meanScore: number | null;
  delta: number | null;
  baselineId: string | null;
  baselineN: number | null;
  tStatistic: number | null;
  ciLow: number | null;
  ciHigh: number | null;
}

function driftReading(e: UiEvent): DriftReading {
  return {
    id: e.id,
    ts: e.ts,
    verdict: dataString(e.data, "verdict"),
    meanScore: dataNumber(e.data, "mean_score"),
    delta: dataNumber(e.data, "delta"),
    baselineId: dataString(e.data, "baseline_id"),
    baselineN: dataNumber(e.data, "baseline_n"),
    tStatistic: dataNumber(e.data, "t_statistic"),
    ciLow: dataNumber(e.data, "ci_low"),
    ciHigh: dataNumber(e.data, "ci_high"),
  };
}

/** on-track -> calm/mint, regressed -> warn/ember, anything else (an
 * unrecognized verdict string) -> the same neutral "faint" tone
 * `SeverityBadge` falls back to for an unrecognized severity - never looks
 * more assured than the data actually is. */
function verdictTone(verdict: string | null): string {
  if (verdict === "regressed") return "var(--ember)";
  if (verdict === "on-track") return "var(--mint)";
  return "var(--faint)";
}

function SectionHeader({ title }: { title: string }) {
  return (
    <span
      className="mono"
      style={{ fontSize: 10.5, letterSpacing: "0.1em", textTransform: "uppercase", color: "var(--faint)" }}
    >
      {title}
    </span>
  );
}

/** Compact, honest "this plane has nothing to show" line - mirrors the tone
 * of `MoneyEmptyState`/`PolicyEmptyState`/`IdentityEmptyState` but inline
 * (a full panel-sized empty state per section would overwhelm a card this
 * dense with five sections). */
function PlaneNote({ children }: { children: ReactNode }) {
  return (
    <span className="text-[11px]" style={{ color: "var(--faint)" }}>
      {children}
    </span>
  );
}

/** Plain-text-looking link button for the "business unit" row below - same
 * look AgentDetailCard's own `linkStyle` uses for its unit/owner rows, so the
 * affordance reads the same wherever it appears. */
const linkStyle: CSSProperties = {
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

function Field({ label, value }: { label: string; value: string }) {
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

/** A parent/child delegation chip - the deep-link's own recursion point:
 * clicking one re-focuses Agent 360 on THAT agent (`Agent360` remounts via
 * `key={agentId}` at the call site in `AppShell.tsx`, so every section
 * refetches cleanly for the new subject). */
function AgentChip({ id, onOpen }: { id: string; onOpen: (agentId: string) => void }) {
  return (
    <button type="button" className="chip" style={{ cursor: "pointer" }} onClick={() => onOpen(id)} title={id}>
      {shortAgentLabel(id)}
    </button>
  );
}

function OpenPanelButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      type="button"
      className="icon-btn self-start"
      style={{ width: "auto", padding: "0 10px", fontSize: 11 }}
      onClick={onClick}
    >
      {label} &rarr;
    </button>
  );
}

/**
 * The "Access" section's body (I5), once this agent's idryx record is
 * confirmed to exist - split out from the ternary chain at its call site
 * purely so the permission/MCP/policy computations below get a real
 * `IdryxIdentity`, not an `IdryxIdentity | null | undefined` that would need
 * a non-null assertion at every call site. Entirely derived by
 * `lib/access.ts` from data already fetched by the Identity/Policy sections
 * above (`identity.permissions`, `allIdentities`/`allAlerts` for MCP reach,
 * `policies` for the Wardryx overlay) - no new fetch of its own. Read-only:
 * no action here mutates anything, matching this whole card's own rule (see
 * the footer note at the bottom of `Agent360`).
 */
function AccessSectionBody({
  identity,
  mcpServers,
  shadowIds,
  policyStatus,
  policyError,
  policies,
  onOpenAgent,
  onNavigate,
}: {
  identity: IdryxIdentity;
  mcpServers: IdryxIdentity[];
  shadowIds: ReadonlySet<string>;
  policyStatus: PolicyStatus | null;
  policyError: PolicyError | null;
  policies: PolicyRecord[] | null;
  onOpenAgent: (agentId: string) => void;
  onNavigate: (view: ViewId) => void;
}) {
  const policyReady = policyStatus?.state === "ready";
  const rollup = permissionRollup(identity.permissions);
  const reach = mcpReachForAgent(
    identity.permissions.map((p) => p.name),
    mcpServers,
    shadowIds,
  );
  const matched = policyReady && policies !== null ? matchedPolicies(identity.id, policies) : null;
  const overlay = matched !== null ? effectiveOverlay(matched) : null;

  return (
    <div className="flex flex-col gap-2">
      {rollup.granted === 0 ? (
        <PlaneNote>this identity carries no permissions in the current idryx snapshot.</PlaneNote>
      ) : (
        <div className="flex flex-col gap-1">
          {identity.permissions.map((p) => (
            <div key={p.name} className="flex items-center gap-2 min-w-0">
              <span className="mono text-[11.5px] truncate" style={{ color: "var(--fg)" }}>
                {p.name}
              </span>
              {p.admin && (
                <span className="badge" style={cssVar("tone", "var(--sev-medium)")}>
                  admin
                </span>
              )}
              {!rollup.hasUsageSignal ? (
                <span
                  className="badge"
                  style={cssVar("tone", "var(--faint)")}
                  title="idryx has recorded no usage signal for this identity's permissions"
                >
                  no usage signal
                </span>
              ) : p.used ? (
                <span className="badge" style={cssVar("tone", "var(--mint)")}>
                  used
                </span>
              ) : (
                <span className="badge" style={cssVar("tone", p.admin ? "var(--sev-high)" : "var(--sev-medium)")}>
                  unused{p.admin ? " · admin" : ""}
                </span>
              )}
            </div>
          ))}
        </div>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <span className="text-[11px]" style={{ color: "var(--dim)" }}>
          MCP reach:
        </span>
        <span className="badge" style={cssVar("tone", "var(--sev-info)")}>
          {reach.sanctionedTools.length} sanctioned
        </span>
        <span className="badge" style={cssVar("tone", reach.shadowTools.length > 0 ? "var(--sev-high)" : "var(--faint)")}>
          {reach.shadowTools.length} shadow
        </span>
      </div>
      {(reach.sanctionedServers.length > 0 || reach.shadowServers.length > 0) && (
        <div className="flex flex-col gap-1">
          {[...reach.shadowServers, ...reach.sanctionedServers].map((s) => (
            <div key={s.serverId} className="flex items-center gap-2 min-w-0">
              <AgentChip id={s.serverId} onOpen={onOpenAgent} />
              <span className="mono text-[11px] truncate" style={{ color: "var(--dim)" }} title={s.tools.join(", ")}>
                {s.tools.join(", ")}
              </span>
            </div>
          ))}
        </div>
      )}

      {!policyReady ? (
        <PlaneNote>
          {!policyStatus || policyStatus.state === "bootstrapping"
            ? "connecting to the policy plane..."
            : "policy plane not connected (wardryx overlay unavailable)."}
        </PlaneNote>
      ) : policyError ? (
        <PlaneNote>{describePolicyError(policyError)}</PlaneNote>
      ) : matched === null ? (
        <PlaneNote>loading...</PlaneNote>
      ) : matched.length === 0 ? (
        <PlaneNote>no wardryx policy targets this agent.</PlaneNote>
      ) : (
        overlay && (
          <div className="flex flex-col gap-1.5">
            <div className="flex flex-wrap items-center gap-1.5">
              {matched.map((p) => (
                <span key={p.id} className="chip" style={cssVar("dot", "var(--src-wardryx)")} title={`target ${p.target}`}>
                  <span className="dot" aria-hidden="true" />
                  {p.name || p.target}
                </span>
              ))}
            </div>
            <div className="flex flex-wrap gap-x-5 gap-y-1.5">
              <Field label="denied tools" value={overlay.deniedTools.length > 0 ? overlay.deniedTools.join(", ") : "none"} />
              <Field
                label="domains"
                value={
                  overlay.allowDomains.effective.kind === "unrestricted"
                    ? "unrestricted"
                    : overlay.allowDomains.effective.domains.length > 0
                      ? overlay.allowDomains.effective.domains.join(", ")
                      : "none in common (matched policies contradict)"
                }
              />
              <Field label="max steps" value={overlay.maxSteps !== null ? String(overlay.maxSteps) : "-"} />
              <Field label="human above" value={overlay.requireHumanAboveUsd !== null ? formatUsd(overlay.requireHumanAboveUsd) : "-"} />
              <Field label="deny above" value={overlay.denyAboveUsd !== null ? formatUsd(overlay.denyAboveUsd) : "-"} />
              <Field label="unattested" value={overlay.denyIfUnattested ? "denied" : "allowed"} />
            </div>
          </div>
        )
      )}

      <div className="flex items-center gap-2">
        <OpenPanelButton label="Open Identity panel" onClick={() => onNavigate("identity")} />
        <OpenPanelButton label="Open Policy panel" onClick={() => onNavigate("policy")} />
      </div>
    </div>
  );
}

/**
 * Agent 360 (docs/PHASE3.md W3, position 4): a read-only, cross-plane card
 * for one `agent_id`, assembled from every plane this shell can reach -
 * Delegation (`agent_slice` + a mini-focus of the graph), Events
 * (`agent_events`), Identity (this agent's row + alerts from the W2 Idryx
 * panel), Access (I5: this agent's permission rollup, MCP reach, and
 * Wardryx overlay - `lib/access.ts`), Money (this agent's runs, from the
 * existing `money_runs`), and Policy (this agent's `wardryx.*` decisions,
 * filtered out of the same `agent_events` result, plus its approvals).
 * Every section has its own honest empty/error state - never a shared
 * spinner, never silently blank - mirroring
 * `MoneyEmptyState`/`PolicyEmptyState`/`IdentityEmptyState`'s tone but
 * inline, since a card this dense cannot afford a full panel-sized empty
 * state per section.
 *
 * Deliberately a VIEW, not an actions surface: kill/approve are not
 * reimplemented here (see the footer note) - `onNavigate` links out to the
 * panel that owns the actual mutation, exactly as docs/PHASE3.md's task
 * brief calls for ("Actions may LINK to the existing panels rather than
 * re-implement a mutation here").
 *
 * Rendered as one drawer inside the fixed overlay `AppShell.tsx` owns,
 * regardless of which nav view is active (the deep-link's "from anywhere"
 * requirement). `AppShell` is responsible for the fixed positioning, the
 * shared backdrop, and laying out up to two of these side by side for the
 * compare view (see its `focusedAgentIds` doc comment) - this component's
 * own root is just the drawer itself: fixed width, full height, its own
 * scroll, and the dialog role/label. Each mounted instance still closes on
 * Escape or its own explicit close button; a shared backdrop click (in
 * `AppShell`, not here) dismisses every open card at once.
 */
export function Agent360({
  agentId,
  inCompare = false,
  onClose,
  onOpenAgent,
  onNavigate,
  onOpenReplay,
}: {
  agentId: string;
  /** True when `AppShell.tsx` is rendering this card alongside a SECOND
   * Agent 360 in its compare overlay (`visibleAgentIds.length > 1`) - false
   * for the ordinary one-card case. Gates ONLY the width's vw fallback
   * below (see `AppShell.tsx`'s own overlay-container comment for the full
   * derivation): a solo card's `min(720px, 94vw)` is already pinned flat at
   * the literal 720px cap for every viewport `COMPARE_MIN_WIDTH` (1200px)
   * ever lets this render at, so two solo-style cards side by side would
   * demand a fixed 1440px - overflowing any compare-eligible viewport
   * narrower than that. `inCompare` swaps the fallback to 46vw instead, so
   * two cards together are always <=92vw (never overflows); everything
   * else about the card - the 720px cap itself, height, layout - is
   * identical either way. */
  inCompare?: boolean;
  onClose: () => void;
  onOpenAgent: (agentId: string) => void;
  onNavigate: (view: ViewId) => void;
  /** Phase-3 wave-4 deep link (docs/PHASE3.md W4): opens Run Replay seeded
   * with one of this agent's runs, from the Money section below. */
  onOpenReplay: (runId: string) => void;
}) {
  const { open } = usePopover();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // ---- Delegation + Events: core-only, no plane status to gate on. ----
  const [slice, setSlice] = useState<AgentSlice | null>(null);
  const [events, setEvents] = useState<UiEvent[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    setSlice(null);
    setEvents(null);
    void fetchAgentSlice(agentId).then((s) => {
      if (!cancelled) setSlice(s);
    });
    void fetchAgentEvents(agentId, EVENTS_LIMIT).then((e) => {
      if (!cancelled) setEvents(e);
    });
    return () => {
      cancelled = true;
    };
  }, [agentId]);

  // ---- Identity: this agent's row + its alerts, filtered client-side. ----
  const identityStatus = useIdentityStatus();
  const identityReady = identityStatus?.state === "ready";
  const [identity, setIdentity] = useState<IdryxIdentity | null | undefined>(undefined);
  const [identityAlerts, setIdentityAlerts] = useState<IdryxAlert[]>([]);
  const [identityError, setIdentityError] = useState<IdentityError | null>(null);
  // Access (I5): the SAME identity-plane read above already returns every
  // identity/alert - captured here too (zero extra fetches) so the Access
  // section below can compute this agent's MCP reach (needs every
  // `mcp_server` identity, not just this one) and derive the shadow set
  // (needs every `shadow_mcp` alert, which typically names a DIFFERENT
  // identity than this agent's own filtered `identityAlerts` above).
  const [allIdentities, setAllIdentities] = useState<IdryxIdentity[]>([]);
  const [allAlerts, setAllAlerts] = useState<IdryxAlert[]>([]);

  useEffect(() => {
    if (!identityReady) return;
    let cancelled = false;
    setIdentity(undefined);
    setIdentityError(null);
    void Promise.all([fetchIdentities(), fetchAlerts()])
      .then(([ids, alerts]) => {
        if (cancelled) return;
        setIdentity(ids.find((i) => i.id === agentId) ?? null);
        setIdentityAlerts(alerts.filter((a) => a.identity === agentId));
        setAllIdentities(ids);
        setAllAlerts(alerts);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setIdentity(null);
        setIdentityAlerts([]);
        setAllIdentities([]);
        setAllAlerts([]);
        setIdentityError(err as IdentityError);
      });
    return () => {
      cancelled = true;
    };
  }, [identityReady, agentId]);

  // ---- Money: this agent's runs, out of the already-wired `money_runs`. ----
  const moneyStatus = useMoneyStatus();
  const moneyReady = moneyStatus?.state === "ready";
  const [runs, setRuns] = useState<Run[] | null>(null);
  const [moneyError, setMoneyError] = useState<MoneyError | null>(null);

  // `consoleVersion` is in the deps so a Freeze or a Kill issued from this
  // panel's own header re-reads the runs it derives its state from. Without
  // it the command lands, the box changes, and the header keeps saying
  // "Freeze" at an agent that is already frozen.
  const consoleVersion = useConsoleStateVersion();

  useEffect(() => {
    if (!moneyReady) return;
    let cancelled = false;
    setRuns(null);
    setMoneyError(null);
    void fetchRuns()
      .then((r) => {
        if (!cancelled) setRuns(r.filter((run) => run.agent_id === agentId));
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setRuns([]);
        setMoneyError(err as MoneyError);
      });
    return () => {
      cancelled = true;
    };
  }, [moneyReady, agentId, consoleVersion]);

  // Freeze and kill live HERE as well as on the agent popover, because this
  // panel is where an operator ENDS UP: every deep link, every row of the
  // Statistics table and the popover's own "open full" lead here, and the
  // panel that shows you the runaway was the one panel that could not stop it.
  // Same two controls, same commands, same break-glass ceremony as
  // `AgentDetailCard.tsx`; no new capability, just where the hand already is.
  //
  // The frozen flag comes from the server's own block list rather than from a
  // record, since `agent_record` is preview-only and this panel must answer on
  // a real box. `liveRun` is the run a kill would target: killing needs a run,
  // and `KillRunButton` renders itself disabled when there is none.
  // Two sources, because neither answers everywhere. `lifecycle_blocks` is the
  // real box's durable set and returns empty under the mock by design (see its
  // own module doc); `money_runs` stamps `Run.lifecycle` on a blocked agent's
  // runs and is what the preview has. Reading only the first left the demo's
  // button saying "Freeze" at an agent it had just frozen.
  const serverBlocks = useLifecycleBlocks();
  const frozen =
    serverBlocks.agents.includes(agentId) ||
    (runs ?? []).some((r) => runBlockedState(r) === "frozen");
  const liveRun = (runs ?? []).find((r) => !r.killed) ?? null;

  // ---- Policy: wardryx.* decisions (from `agent_events`) + approvals. ----
  const policyStatus = usePolicyStatus();
  const policyReady = policyStatus?.state === "ready";
  const [approvals, setApprovals] = useState<Approval[] | null>(null);
  const [policyError, setPolicyError] = useState<PolicyError | null>(null);
  // Access (I5): this agent's matched Wardryx policies need the full policy
  // set (matched against `agentId`'s own glob, client-side) - fetched
  // alongside approvals on the same effect/readiness gate below, since both
  // are reads of the same policy plane.
  const [policies, setPolicies] = useState<PolicyRecord[] | null>(null);

  useEffect(() => {
    if (!policyReady) return;
    let cancelled = false;
    setApprovals(null);
    setPolicyError(null);
    void Promise.all([fetchApprovals(), fetchPolicies()])
      .then(([a, p]) => {
        if (cancelled) return;
        setApprovals(a.filter((ap) => ap.agent_id === agentId));
        setPolicies(p);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setApprovals([]);
        setPolicies(null);
        setPolicyError(err as PolicyError);
      });
    return () => {
      cancelled = true;
    };
  }, [policyReady, agentId]);

  const policyEvents = (events ?? []).filter((e) => e.source === "wardryx");
  const attestationAlerts = identityAlerts.filter((a) => ATTESTATION_DETECTORS.has(a.detector));
  const totalSpentUsd = (runs ?? []).reduce((sum, r) => sum + r.spent_usd, 0);
  // Access (I5): derived from the SAME identity-plane read the Identity
  // section above already made (`allIdentities`/`allAlerts`) - no new fetch.
  const mcpServers = mcpServerIdentities(allIdentities);
  const shadowIds = shadowServerIds(allAlerts);
  // Drift (I11): assembled entirely from state the sections above already
  // fetched - zero new fetches, zero new commands. Quality drift is `events`
  // (Events section) filtered to verdryx's `quality_drift` bus event with
  // the SAME predicate `QualityDriftStream.tsx`/`lib/incidents.ts` use
  // elsewhere, newest first (sorted here rather than trusted from the
  // transport, so this reads correctly regardless of fetch order). Spend
  // trend buckets `runs` (Money section) with the SAME `spendSeries` helper
  // MoneyView/OverviewView already feed their own hero Sparkline from.
  // Behavior anomalies are `identityAlerts` (Identity section) filtered to
  // idryx's `behavior_anomaly` detector - idryx's own LOGIN-behavior
  // baseline, not a quality baseline of any kind: "idryx baselines" on this
  // card is exactly this filter over alerts Agent 360 already held, nothing
  // new fetched from idryx.
  const driftReadings = (events ?? [])
    .filter(isQualityDriftEvent)
    .map(driftReading)
    .sort((a, b) => Date.parse(b.ts) - Date.parse(a.ts));
  const latestDrift = driftReadings[0] ?? null;
  const behaviorAlerts = identityAlerts
    .filter((a) => a.detector === "behavior_anomaly")
    .slice()
    .sort((a, b) => sevRank(b.severity) - sevRank(a.severity) || Date.parse(b.time) - Date.parse(a.time));
  const spendTrend = spendSeries(runs ?? []);
  // Business unit (same derivation AgentDetailCard/UserCard use): the console
  // only ever learns an agent's team from its id path
  // (`agent://org/<team>/<name>`), so the unit shown here comes from that
  // path segment, not from any plane fetch above.
  const teamSeg = /^agent:\/\/[^/]+\/([^/]+)\//.exec(agentId)?.[1] ?? null;
  const unit = teamSeg ? unitForTeam(teamSeg) : null;

  return (
    <div
      className="relative flex flex-col gap-5 thin-scroll overflow-y-auto"
      role="dialog"
      aria-modal="true"
      aria-label={`Agent 360: ${agentId}`}
      style={{
        width: inCompare ? "min(720px, 46vw)" : "min(720px, 94vw)",
        flexShrink: 0,
        flexGrow: 0,
        height: "100%",
        background: "var(--bg)",
        borderLeft: "1px solid var(--line-2)",
        padding: "20px 22px 28px",
        boxShadow: "-24px 0 48px color-mix(in srgb, var(--ink) 35%, transparent)",
      }}
    >
      <div className="flex items-start gap-3">
        <div className="flex flex-col gap-1 min-w-0">
          <span
            className="mono text-[10px] uppercase tracking-wider"
            style={{ color: "var(--faint)" }}
            title="Click another agent to open it beside this one for comparison."
          >
            Agent 360
          </span>
          <span className="mono truncate text-[15px]" style={{ color: "var(--fg)" }} title={agentId}>
            {agentId}
          </span>
        </div>
        <div className="flex-1" />
        <FreezeToggleButton frozen={frozen} onToggle={() => blockAgent(agentId, !frozen).then(() => {})} />
        <KillRunButton
          run={liveRun}
          detail={liveRun ? `run ${liveRun.run_id} · spent ${formatUsd(liveRun.spent_usd)}` : undefined}
          onKill={(runId, reason) => killRun(runId, reason).then(() => {})}
        />
        <WatchToggleButton kind="agent" id={agentId} label={shortAgentLabel(agentId)} />
        <button type="button" className="icon-btn" aria-label="Close Agent 360" onClick={onClose}>
          &times;
        </button>
      </div>

      {unit && (
        <div className="flex items-baseline gap-2 min-w-0">
          <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
            business unit
          </span>
          <button
            type="button"
            style={linkStyle}
            onClick={(e) => open(<UnitCard team={unit} onOpenFullAgent={onOpenAgent} />, { anchor: e.currentTarget.getBoundingClientRect() })}
          >
            {prettyUnit(unit)} &rsaquo;
          </button>
        </div>
      )}

      {/* ---- Delegation ---- */}
      <section className="flex flex-col gap-2">
        <SectionHeader title="Delegation" />
        {slice === null ? (
          <PlaneNote>loading...</PlaneNote>
        ) : (
          <div className="flex flex-col gap-2">
            {slice.node === null && slice.parents.length === 0 && slice.children.length === 0 ? (
              <PlaneNote>this agent has never been seen on the delegation graph.</PlaneNote>
            ) : (
              <div className="flex flex-wrap gap-x-5 gap-y-1.5">
                <Field label="events" value={slice.node ? String(slice.node.event_count) : "0"} />
                <Field label="last seen" value={slice.node?.last_ts ? formatTimestamp(slice.node.last_ts) : "-"} />
                <Field label="kind" value={slice.node?.kind ?? "(chain-only)"} />
                {/* The way BACK, and it sits HERE rather than up in the header
                    (Yurii, 2026-08-10) because it belongs to this line: the
                    header is the panel's own controls (watch, freeze, kill,
                    close), and this is one more thing to open about the agent,
                    the same shape and the same affordance as the delegation
                    chips right below it.
                    `AgentDetailCard` has had an "open full" into this panel
                    since it was written and nothing went the other way, so an
                    operator who arrived by a link and then wanted the
                    owned-thing view had to close this, find the agent
                    elsewhere, and open the popover from there. */}
                <button
                  type="button"
                  className="chip self-center"
                  style={{ cursor: "pointer" }}
                  title="The agent's owned-thing card: owner, unit, budget, behaviour and lifecycle"
                  onClick={(e) =>
                    open(<AgentDetailCard agentId={agentId} onOpenFull={onOpenAgent} />, {
                      anchor: e.currentTarget.getBoundingClientRect(),
                    })
                  }
                >
                  Agent card &rsaquo;
                </button>
              </div>
            )}
            {slice.parents.length > 0 && (
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
                  delegated by
                </span>
                {slice.parents.map((p) => (
                  <AgentChip key={p.id} id={p.id} onOpen={onOpenAgent} />
                ))}
              </div>
            )}
            {slice.children.length > 0 && (
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
                  delegates to
                </span>
                {slice.children.map((c) => (
                  <AgentChip key={c.id} id={c.id} onOpen={onOpenAgent} />
                ))}
              </div>
            )}
            <DelegationGraphView key={agentId} focusAgentId={agentId} onOpenAgent={onOpenAgent} height={220} compact />
            <OpenPanelButton label="Open full graph" onClick={() => onNavigate("graph")} />
          </div>
        )}
      </section>

      {/* ---- Events ---- */}
      <section className="flex flex-col gap-2">
        <SectionHeader title="Events" />
        {events === null ? (
          <PlaneNote>loading...</PlaneNote>
        ) : events.length === 0 ? (
          <PlaneNote>no events for this agent yet.</PlaneNote>
        ) : (
          <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
            {events.slice(0, EVENTS_SHOWN).map((e) => (
              <div
                key={e.id}
                className="grid items-center gap-3 px-3 py-1.5 bus-row"
                style={{ gridTemplateColumns: "76px 92px 1fr 140px" }}
              >
                <SeverityBadge severity={e.severity} />
                <SourceChip source={e.source} />
                <span className="mono truncate text-[11.5px]" style={{ color: "var(--fg)" }} title={e.type}>
                  {e.type}
                </span>
                <span className="mono tabular text-[10.5px] text-right" style={{ color: "var(--faint)" }}>
                  {formatTimestamp(e.ts)}
                </span>
              </div>
            ))}
            {events.length > EVENTS_SHOWN && (
              <div className="px-3 py-1.5 mono text-[10.5px]" style={{ color: "var(--faint)" }}>
                + {events.length - EVENTS_SHOWN} more &middot; open the Bus Explorer for the full list
              </div>
            )}
          </div>
        )}
      </section>

      {/* ---- Identity ---- */}
      <section className="flex flex-col gap-2">
        <SectionHeader title="Identity" />
        {!identityReady ? (
          <PlaneNote>
            {!identityStatus || identityStatus.state === "bootstrapping"
              ? "connecting to the identity plane..."
              : "identity plane not connected."}
          </PlaneNote>
        ) : identityError ? (
          <PlaneNote>{describeIdentityError(identityError)}</PlaneNote>
        ) : identity === undefined ? (
          <PlaneNote>loading...</PlaneNote>
        ) : (
          <div className="flex flex-col gap-2">
            {identity ? (
              <div className="flex flex-wrap gap-x-5 gap-y-1.5">
                <Field label="type" value={identity.type} />
                <Field label="source" value={identity.source} />
                <Field label="privileged" value={identity.privileged ? "yes" : "no"} />
                <Field label="owner" value={identity.owner || "-"} />
              </div>
            ) : (
              <PlaneNote>no idryx identity record for this agent (as of the last load/Rescan).</PlaneNote>
            )}
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-[11px]" style={{ color: "var(--dim)" }}>
                attestation:
              </span>
              {attestationAlerts.length === 0 ? (
                <span className="badge" style={cssVar("tone", "var(--sev-low)")}>
                  none flagged
                </span>
              ) : (
                attestationAlerts.map((a, idx) => (
                  <span key={idx} className="badge" style={cssVar("tone", "var(--sev-high)")} title={a.summary}>
                    {a.detector}
                  </span>
                ))
              )}
            </div>
            {identityAlerts.length > 0 && (
              <div className="flex flex-col gap-1">
                {identityAlerts.slice(0, IDENTITY_ALERTS_SHOWN).map((a, idx) => (
                  <div key={idx} className="flex items-center gap-2 min-w-0">
                    <SeverityBadge severity={a.severity} />
                    <span className="mono text-[11px]" style={{ color: "var(--fg)" }}>
                      {a.detector}
                    </span>
                    <span className="text-[11px] truncate" style={{ color: "var(--dim)" }} title={a.summary}>
                      {a.summary}
                    </span>
                  </div>
                ))}
              </div>
            )}
            <OpenPanelButton label="Open Identity panel" onClick={() => onNavigate("identity")} />
          </div>
        )}
      </section>

      {/* ---- Access (I5: agent access matrix) ---- */}
      <section className="flex flex-col gap-2">
        <SectionHeader title="Access" />
        {!identityReady ? (
          <PlaneNote>
            {!identityStatus || identityStatus.state === "bootstrapping"
              ? "connecting to the identity plane..."
              : "identity plane not connected."}
          </PlaneNote>
        ) : identityError ? (
          <PlaneNote>{describeIdentityError(identityError)}</PlaneNote>
        ) : identity === undefined ? (
          <PlaneNote>loading...</PlaneNote>
        ) : identity === null ? (
          <PlaneNote>no idryx identity record for this agent - nothing to assemble an access matrix from.</PlaneNote>
        ) : (
          <AccessSectionBody
            identity={identity}
            mcpServers={mcpServers}
            shadowIds={shadowIds}
            policyStatus={policyStatus}
            policyError={policyError}
            policies={policies}
            onOpenAgent={onOpenAgent}
            onNavigate={onNavigate}
          />
        )}
      </section>

      {/* ---- Money ---- */}
      <section className="flex flex-col gap-2">
        <SectionHeader title="Money" />
        {!moneyReady ? (
          <PlaneNote>
            {!moneyStatus || moneyStatus.state === "bootstrapping"
              ? "connecting to the money plane..."
              : "money plane not connected."}
          </PlaneNote>
        ) : moneyError ? (
          <PlaneNote>{describeMoneyError(moneyError)}</PlaneNote>
        ) : runs === null ? (
          <PlaneNote>loading...</PlaneNote>
        ) : runs.length === 0 ? (
          <PlaneNote>no runs for this agent yet.</PlaneNote>
        ) : (
          <div className="flex flex-col gap-2">
            <div className="flex flex-wrap gap-x-5 gap-y-1.5">
              <Field label="total spent" value={formatUsd(totalSpentUsd)} />
              <Field label="runs" value={String(runs.length)} />
            </div>
            <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
              {runs.map((r) => (
                <div
                  key={r.run_id}
                  className="grid items-center gap-3 px-3 py-1.5 bus-row"
                  style={{ gridTemplateColumns: "1fr 90px 90px 64px 60px" }}
                >
                  <span className="mono truncate text-[11.5px]" style={{ color: "var(--fg)" }} title={r.run_id}>
                    {r.run_id}
                  </span>
                  <span className="mono tabular text-[11.5px]" style={{ color: "var(--fg)" }}>
                    {formatUsd(r.spent_usd)}
                  </span>
                  <span className="mono tabular text-[11.5px]" style={{ color: "var(--dim)" }}>
                    {r.budget_usd !== null ? formatUsd(r.budget_usd) : "-"}
                  </span>
                  <span>
                    {(() => {
                      const blocked = runBlockedState(r);
                      return blocked ? <StateBadge state={blocked} /> : null;
                    })()}
                  </span>
                  <span className="flex justify-end">
                    <button
                      type="button"
                      className="icon-btn"
                      style={{ width: "auto", padding: "0 8px", fontSize: 10.5 }}
                      title={`Replay run ${r.run_id}`}
                      onClick={() => onOpenReplay(r.run_id)}
                    >
                      Replay
                    </button>
                  </span>
                </div>
              ))}
            </div>
          </div>
        )}
        {moneyReady && <OpenPanelButton label="Open Money panel" onClick={() => onNavigate("money")} />}
      </section>

      {/* ---- Drift (I11: quality drift + spend trend + behavior anomalies) ---- */}
      <section className="flex flex-col gap-3">
        <SectionHeader title="Drift" />

        {/* Quality drift: verdryx's quality_drift bus events for this
            agent, out of the SAME `events` the Events/Policy sections
            above already hold. */}
        <div className="flex flex-col gap-2">
          <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
            quality drift
          </span>
          {events === null ? (
            <PlaneNote>loading...</PlaneNote>
          ) : latestDrift === null ? (
            <PlaneNote>
              no quality-drift signal for this agent yet - drift arrives on the bus from verdryx only when it
              flags a regression against a baseline.
            </PlaneNote>
          ) : (
            <div className="flex flex-col gap-2">
              <div className="flex flex-wrap items-center gap-2">
                <span className="badge" style={cssVar("tone", verdictTone(latestDrift.verdict))}>
                  {latestDrift.verdict ?? "unknown verdict"}
                </span>
                <span className="mono tabular text-[10.5px]" style={{ color: "var(--faint)" }}>
                  {formatTimestamp(latestDrift.ts)}
                </span>
              </div>
              <div className="flex flex-wrap gap-x-5 gap-y-1.5">
                <Field label="mean score" value={latestDrift.meanScore !== null ? latestDrift.meanScore.toFixed(3) : "-"} />
                <Field
                  label="delta"
                  value={
                    latestDrift.delta !== null ? `${latestDrift.delta >= 0 ? "+" : ""}${latestDrift.delta.toFixed(3)}` : "-"
                  }
                />
                {latestDrift.baselineId && <Field label="baseline" value={latestDrift.baselineId} />}
                {latestDrift.baselineN !== null && latestDrift.baselineN > 0 && (
                  <>
                    <Field label="t-statistic" value={latestDrift.tStatistic !== null ? latestDrift.tStatistic.toFixed(2) : "-"} />
                    <Field
                      label="95% CI"
                      value={
                        latestDrift.ciLow !== null && latestDrift.ciHigh !== null
                          ? `[${latestDrift.ciLow.toFixed(3)}, ${latestDrift.ciHigh.toFixed(3)}]`
                          : "-"
                      }
                    />
                    <Field label="baseline n" value={String(latestDrift.baselineN)} />
                  </>
                )}
              </div>
              {driftReadings.length > 1 && (
                <div className="flex flex-col gap-1">
                  <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
                    recent checks
                  </span>
                  {driftReadings.slice(1, DRIFT_CHECKS_SHOWN).map((d) => (
                    <div key={d.id} className="flex items-center gap-2 min-w-0">
                      <span className="badge" style={cssVar("tone", verdictTone(d.verdict))}>
                        {d.verdict ?? "-"}
                      </span>
                      <span className="mono tabular text-[11px]" style={{ color: "var(--dim)" }}>
                        {d.delta !== null ? `${d.delta >= 0 ? "+" : ""}${d.delta.toFixed(3)}` : "-"}
                      </span>
                      <span className="mono tabular text-[10.5px]" style={{ color: "var(--faint)" }}>
                        {formatTimestamp(d.ts)}
                      </span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
          <OpenPanelButton label="Open Quality panel" onClick={() => onNavigate("quality")} />
        </div>

        {/* Spend trend: this agent's own runs (Money section above),
            bucketed with the SAME `spendSeries` helper MoneyView/
            OverviewView already feed their own hero Sparkline from. */}
        <div className="flex flex-col gap-2">
          <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
            spend trend
          </span>
          {!moneyReady ? (
            <PlaneNote>
              {!moneyStatus || moneyStatus.state === "bootstrapping"
                ? "connecting to the money plane..."
                : "money plane not connected."}
            </PlaneNote>
          ) : moneyError ? (
            <PlaneNote>{describeMoneyError(moneyError)}</PlaneNote>
          ) : runs === null ? (
            <PlaneNote>loading...</PlaneNote>
          ) : runs.length === 0 ? (
            <PlaneNote>no runs for this agent yet.</PlaneNote>
          ) : spendTrend.length === 0 ? (
            <PlaneNote>not enough distinct run timestamps yet to draw a trend (needs at least two).</PlaneNote>
          ) : (
            <div className="flex flex-col gap-1">
              <Sparkline values={spendTrend} height={56} />
              <span className="text-[11px]" style={{ color: "var(--dim)" }}>
                spend over recent runs &middot; {formatUsd(totalSpentUsd)} total across {runs.length} run
                {runs.length === 1 ? "" : "s"}
              </span>
            </div>
          )}
        </div>

        {/* Behavior anomalies: idryx's behavior_anomaly alerts for this
            agent, out of the SAME `identityAlerts` the Identity section
            above already fetched - idryx's login-behavior baseline, not a
            quality one. */}
        <div className="flex flex-col gap-2">
          <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
            behavior anomalies
          </span>
          {!identityReady ? (
            <PlaneNote>
              {!identityStatus || identityStatus.state === "bootstrapping"
                ? "connecting to the identity plane..."
                : "identity plane not connected."}
            </PlaneNote>
          ) : identityError ? (
            <PlaneNote>{describeIdentityError(identityError)}</PlaneNote>
          ) : identity === undefined ? (
            <PlaneNote>loading...</PlaneNote>
          ) : behaviorAlerts.length === 0 ? (
            <PlaneNote>no behavior-anomaly alerts for this agent.</PlaneNote>
          ) : (
            <div className="flex flex-col gap-1">
              {behaviorAlerts.slice(0, BEHAVIOR_ALERTS_SHOWN).map((a, idx) => (
                <div key={idx} className="flex items-center gap-2 min-w-0">
                  <SeverityBadge severity={a.severity} />
                  <span className="text-[11px] truncate" style={{ color: "var(--dim)" }} title={a.summary}>
                    {a.summary}
                  </span>
                  <span className="mono tabular text-[10.5px]" style={{ color: "var(--faint)" }}>
                    {formatTimestamp(a.time)}
                  </span>
                </div>
              ))}
            </div>
          )}
          <OpenPanelButton label="Open Identity panel" onClick={() => onNavigate("identity")} />
        </div>
      </section>

      {/* ---- Policy ---- */}
      <section className="flex flex-col gap-2">
        <SectionHeader title="Policy" />
        {policyEvents.length === 0 ? (
          <PlaneNote>no wardryx decisions for this agent yet.</PlaneNote>
        ) : (
          <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
            {policyEvents.slice(0, POLICY_EVENTS_SHOWN).map((e) => (
              <div
                key={e.id}
                className="grid items-center gap-3 px-3 py-1.5 bus-row"
                style={{ gridTemplateColumns: "76px 1fr 140px" }}
              >
                <SeverityBadge severity={e.severity} />
                <span className="mono truncate text-[11.5px]" style={{ color: "var(--fg)" }}>
                  {e.type}
                </span>
                <span className="mono tabular text-[10.5px] text-right" style={{ color: "var(--faint)" }}>
                  {formatTimestamp(e.ts)}
                </span>
              </div>
            ))}
          </div>
        )}

        {!policyReady ? (
          <PlaneNote>
            {!policyStatus || policyStatus.state === "bootstrapping"
              ? "connecting to the policy plane..."
              : "policy plane not connected (no approvals to show)."}
          </PlaneNote>
        ) : policyError ? (
          <PlaneNote>{describePolicyError(policyError)}</PlaneNote>
        ) : (
          approvals !== null &&
          approvals.length > 0 && (
            <div className="flex flex-col gap-1">
              <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
                approvals
              </span>
              {approvals.map((a) => (
                <div key={a.approval_id} className="flex items-center gap-2 min-w-0">
                  <span
                    className="badge"
                    style={cssVar(
                      "tone",
                      a.pending ? "var(--sev-medium)" : a.decision === "grant" ? "var(--sev-low)" : "var(--sev-critical)",
                    )}
                  >
                    {a.pending ? "pending" : (a.decision ?? "decided")}
                  </span>
                  <span className="mono truncate text-[11px]" style={{ color: "var(--dim)" }}>
                    {a.approval_id}
                  </span>
                  {a.est_cost_usd !== null && (
                    <span className="mono tabular text-[11px]" style={{ color: "var(--faint)" }}>
                      {formatUsd(a.est_cost_usd)}
                    </span>
                  )}
                </div>
              ))}
            </div>
          )
        )}
        {policyReady && <OpenPanelButton label="Open Policy panel" onClick={() => onNavigate("policy")} />}
      </section>

      <span className="text-[10.5px]" style={{ color: "var(--faint)" }}>
        This card is read-only. Killing a run, setting a budget, or granting/denying an approval happens in the
        Money and Policy panels linked above, not here.
      </span>
    </div>
  );
}
