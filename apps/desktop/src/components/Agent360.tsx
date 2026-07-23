import type { ReactNode } from "react";
import { useEffect, useState } from "react";
import type { AgentSlice } from "../graphTypes";
import { ATTESTATION_DETECTORS } from "../identityTypes";
import type { IdentityError, IdryxAlert, IdryxIdentity } from "../identityTypes";
import { cssVar } from "../lib/cssVars";
import { formatTimestamp, formatUsd } from "../lib/format";
import { fetchAgentEvents, fetchAgentSlice, shortAgentLabel } from "../lib/graph";
import { describeIdentityError, fetchAlerts, fetchIdentities } from "../lib/identity";
import { describeMoneyError, fetchRuns } from "../lib/money";
import { describePolicyError, fetchApprovals, fetchPolicies } from "../lib/policy";
import { effectiveOverlay, matchedPolicies, mcpReachForAgent, mcpServerIdentities, permissionRollup, shadowServerIds } from "../lib/access";
import { useIdentityStatus } from "../lib/useIdentityStatus";
import { useMoneyStatus } from "../lib/useMoneyStatus";
import { usePolicyStatus } from "../lib/usePolicyStatus";
import type { ViewId } from "../lib/views";
import type { MoneyError, Run } from "../moneyTypes";
import type { Approval, PolicyError, PolicyRecord, PolicyStatus } from "../policyTypes";
import type { UiEvent } from "../types";
import { DelegationGraphView } from "./DelegationGraphView";
import { SeverityBadge } from "./SeverityBadge";
import { SourceChip } from "./SourceChip";

/** How many events/policy rows this compact card shows inline before
 * pointing at the fuller panel instead of growing without bound. */
const EVENTS_LIMIT = 50;
const EVENTS_SHOWN = 12;
const POLICY_EVENTS_SHOWN = 8;
const IDENTITY_ALERTS_SHOWN = 6;

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
 * Rendered as a fixed overlay from `AppShell.tsx` regardless of which nav
 * view is active - the deep-link's "from anywhere" requirement - and closes
 * on Escape, a backdrop click, or the explicit close button.
 */
export function Agent360({
  agentId,
  onClose,
  onOpenAgent,
  onNavigate,
  onOpenReplay,
}: {
  agentId: string;
  onClose: () => void;
  onOpenAgent: (agentId: string) => void;
  onNavigate: (view: ViewId) => void;
  /** Phase-3 wave-4 deep link (docs/PHASE3.md W4): opens Run Replay seeded
   * with one of this agent's runs, from the Money section below. */
  onOpenReplay: (runId: string) => void;
}) {
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
  }, [moneyReady, agentId]);

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

  return (
    <div className="fixed inset-0 z-50 flex justify-end" role="dialog" aria-modal="true" aria-label={`Agent 360: ${agentId}`}>
      <button
        type="button"
        aria-label="Close Agent 360"
        className="absolute inset-0"
        style={{ background: "color-mix(in srgb, var(--ink) 55%, transparent)", cursor: "default" }}
        onClick={onClose}
      />
      <div
        className="relative flex flex-col gap-5 thin-scroll overflow-y-auto"
        style={{
          width: "min(760px, 94vw)",
          height: "100%",
          background: "var(--bg)",
          borderLeft: "1px solid var(--line-2)",
          padding: "20px 22px 28px",
          boxShadow: "-24px 0 48px color-mix(in srgb, var(--ink) 35%, transparent)",
        }}
      >
        <div className="flex items-start gap-3">
          <div className="flex flex-col gap-1 min-w-0">
            <span className="mono text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
              Agent 360
            </span>
            <span className="mono truncate text-[15px]" style={{ color: "var(--fg)" }} title={agentId}>
              {agentId}
            </span>
          </div>
          <div className="flex-1" />
          <button type="button" className="icon-btn" aria-label="Close Agent 360" onClick={onClose}>
            &times;
          </button>
        </div>

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
                      {r.killed && (
                        <span className="badge" style={cssVar("tone", "var(--faint)")}>
                          killed
                        </span>
                      )}
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
    </div>
  );
}
