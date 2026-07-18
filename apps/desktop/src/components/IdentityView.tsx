import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { describeIdentityError, fetchAlerts, fetchIdentities, fetchRemediations, rescan } from "../lib/identity";
import { useIdentityStatus } from "../lib/useIdentityStatus";
import { formatHm } from "../lib/format";
import { ATTESTATION_DETECTORS } from "../identityTypes";
import type { IdentityError, IdentityStatus, IdryxAlert, IdryxIdentity, IdryxRecommendation } from "../identityTypes";
import { IdentityAlerts } from "./IdentityAlerts";
import { IdentityList } from "./IdentityList";
import { FreshBadge } from "./FreshBadge";
import { Hero, HeroBand, KpiTile, Section } from "./dash";

const SEVERITY_ORDER = ["critical", "high", "medium", "low", "info", "none"] as const;

/** "2 critical · 5 high" style breakdown for the hero's Alerts tile - empty
 * only when there are truly no alerts (an honest empty state, never a
 * fabricated "0 critical · 0 high · ..." line). */
function severityBreakdown(alerts: IdryxAlert[]): string {
  const parts = SEVERITY_ORDER.map((sev) => ({ sev, count: alerts.filter((a) => a.severity === sev).length })).filter(
    (x) => x.count > 0,
  );
  return parts.length > 0 ? parts.map((p) => `${p.count} ${p.sev}`).join(" · ") : "none";
}

function Loading() {
  return (
    <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
      loading...
    </div>
  );
}

/**
 * Shared "not ready yet" rendering for the Identity view - mirrors
 * `PolicyView.tsx`'s local `PolicyEmptyState`'s three honest, distinct
 * states (never a generic spinner-forever or error toast), Idryx-flavored:
 * still connecting, no identity plane configured, or a resolved
 * environment whose `GET /healthz` check failed.
 */
function IdentityEmptyState({ status }: { status: IdentityStatus | null }) {
  if (!status || status.state === "bootstrapping") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center">
        <div className="mono text-[12px]" style={{ color: "var(--faint)" }}>
          connecting to an Idryx identity plane...
        </div>
      </div>
    );
  }

  if (status.state === "no_environment") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center px-6">
        <div className="panel px-5 py-4 flex flex-col gap-2" style={{ background: "var(--panel-2)", maxWidth: 480 }}>
          <span style={{ fontSize: 13, color: "var(--fg)" }}>No identity plane found</span>
          <span className="mono text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.7 }}>
            Run <span style={{ color: "var(--fg)" }}>taipan up --with idryx</span> to start one. Idryx has no bearer
            key of its own, so there is no environment-variable fallback here - only a discovered{" "}
            <span style={{ color: "var(--fg)" }}>taipan up</span> descriptor.
          </span>
        </div>
      </div>
    );
  }

  if (status.state === "unreachable") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center px-6">
        <div className="panel px-5 py-4 flex flex-col gap-2" style={{ background: "var(--panel-2)", maxWidth: 480 }}>
          <span style={{ fontSize: 13, color: "var(--sev-high)" }}>Could not reach idryx</span>
          <span className="mono text-[11.5px]" style={{ color: "var(--dim)" }}>
            {status.idryx_url || "(no idryx URL resolved)"}
          </span>
          <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
            {status.reason}
          </span>
        </div>
      </div>
    );
  }

  // `status.state === "ready"`: callers only render this component when NOT
  // ready, so this branch is unreachable in practice.
  return null;
}

/**
 * The Identity panel (docs/PHASE3.md W2): an Identities list (type
 * filters), an Alerts stream (severity + detector filters, plus Rescan),
 * and a Remediations list, over a read-only Idryx connection. Mirrors
 * `PolicyView.tsx`'s overall shape (status hook, empty state, section
 * layout) but deliberately does NOT mirror its 20s periodic auto-refresh:
 * `idryx serve` is a load-once snapshot (docs/PHASE3.md - "Polling /api/*
 * returns byte-identical data for the process lifetime"), so a timer here
 * would either be a no-op or, worse, silently overwrite a fresher Rescan
 * result with the older REST snapshot. Data only ever changes via the
 * explicit Refresh button (re-reads the REST snapshot, e.g. after idryx was
 * restarted externally) or Rescan (recomputes and replaces the alerts
 * list only - idryx's CLI batch output is alerts-only, it can never refresh
 * the identities/remediations lists).
 */
export function IdentityView({ onOpenAgent }: { onOpenAgent: (agentId: string) => void }) {
  const status = useIdentityStatus();
  const ready = status?.state === "ready";

  const [identities, setIdentities] = useState<IdryxIdentity[] | null>(null);
  const [alerts, setAlerts] = useState<IdryxAlert[] | null>(null);
  const [remediations, setRemediations] = useState<IdryxRecommendation[] | null>(null);
  const [error, setError] = useState<IdentityError | null>(null);
  const [asOfMs, setAsOfMs] = useState<number | null>(null);
  const [rescanning, setRescanning] = useState(false);

  const load = useCallback(async () => {
    if (!ready) return;
    try {
      const [i, a, r] = await Promise.all([fetchIdentities(), fetchAlerts(), fetchRemediations()]);
      setIdentities(i);
      setAlerts(a);
      setRemediations(r);
      setAsOfMs(Date.now());
      setError(null);
    } catch (err) {
      setError(err as IdentityError);
    }
  }, [ready]);

  // Fetch once when the panel becomes ready - see this component's doc
  // comment for why there is no periodic re-poll here.
  useEffect(() => {
    void load();
  }, [load]);

  const handleRescan = useCallback(async () => {
    setRescanning(true);
    try {
      const fresh = await rescan();
      setAlerts(fresh);
      setAsOfMs(Date.now());
      setError(null);
    } catch (err) {
      setError(err as IdentityError);
    } finally {
      setRescanning(false);
    }
  }, []);

  if (!ready) {
    return <IdentityEmptyState status={status} />;
  }

  const attestationAlerts = (alerts ?? []).filter((a) => ATTESTATION_DETECTORS.has(a.detector));
  const privilegedCount = (identities ?? []).filter((i) => i.privileged).length;
  const hhmm = asOfMs !== null ? formatHm(asOfMs) : undefined;

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <span className="chip" style={cssVar("dot", "var(--src-qryx)")}>
          <span className="dot" aria-hidden="true" />
          taipan up &middot; {status.source.name} &middot; {status.idryx_url}
        </span>
        <FreshBadge variant="snapshot" detail={hhmm} title="idryx serve loads once at startup; this is when the console last read that snapshot" />
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 10px", fontSize: 11 }}
          onClick={() => void load()}
        >
          Refresh
        </button>
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 10px", fontSize: 11 }}
          onClick={() => void handleRescan()}
          disabled={!status.rescan_available || rescanning}
          title={
            status.rescan_available
              ? "Recompute the 21 detectors now (idryx detect)"
              : "Rescan needs the idryx binary at ~/.taipan/bin/idryx, which was not found"
          }
        >
          {rescanning ? "Rescanning..." : "Rescan"}
        </button>
        <div className="flex-1" />
      </div>

      <span className="text-[11px]" style={{ color: "var(--faint)" }}>
        idryx serve loads once at startup and never reloads on its own; Refresh re-reads that same snapshot, Rescan
        runs a fresh detect pass over the current bus files.
      </span>

      {error && (
        <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
          {describeIdentityError(error)}
        </div>
      )}

      {identities === null || alerts === null ? (
        <div className="mono" style={{ fontSize: 12, color: "var(--faint)" }}>
          loading identity snapshot...
        </div>
      ) : (
        <HeroBand
          hero={
            <Hero
              cap="Identity · idryx snapshot"
              value={identities.length.toLocaleString("en-US")}
              sub={<>{privilegedCount} privileged</>}
            />
          }
          tiles={
            <>
              <KpiTile
                label="Privileged"
                value={privilegedCount.toLocaleString("en-US")}
                tone={privilegedCount > 0 ? "var(--sev-high)" : undefined}
                sub={`of ${identities.length.toLocaleString("en-US")} identities`}
              />
              <KpiTile
                label="Alerts"
                value={alerts.length.toLocaleString("en-US")}
                tone={alerts.length > 0 ? "var(--sev-high)" : undefined}
                sub={severityBreakdown(alerts)}
              />
              <KpiTile
                label="Attestation gaps"
                value={attestationAlerts.length.toLocaleString("en-US")}
                tone={attestationAlerts.length > 0 ? "var(--sev-high)" : "var(--mint)"}
                sub="attestation_missing + bom_incomplete"
              />
            </>
          }
        />
      )}

      <Section title="Identities" right={<FreshBadge variant="snapshot" detail={hhmm} />}>
        {identities === null ? <Loading /> : <IdentityList identities={identities} onOpenAgent={onOpenAgent} />}
      </Section>

      <Section title="Alerts" right={<FreshBadge variant="snapshot" detail={hhmm} />}>
        {alerts === null ? <Loading /> : <IdentityAlerts alerts={alerts} onOpenAgent={onOpenAgent} />}
      </Section>

      <Section title="Remediations" right={<FreshBadge variant="snapshot" detail={hhmm} />}>
        {remediations === null ? (
          <Loading />
        ) : remediations.length === 0 ? (
          <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
            no remediations in this snapshot.
          </div>
        ) : (
          <div style={{ overflowX: "auto" }}>
            {remediations.map((r, idx) => (
              <div
                key={`${r.identity}-${r.kind}-${idx}`}
                className="grid items-center gap-3 px-5 py-2.5 bus-row"
                style={{ gridTemplateColumns: "1fr 110px 1fr" }}
              >
                <span className="mono truncate text-[11.5px]" title={r.identity} style={{ color: "var(--fg)" }}>
                  {r.identity}
                </span>
                <span
                  className="badge"
                  style={cssVar("tone", r.kind === "rotation" ? "var(--sev-medium)" : "var(--sev-info)")}
                >
                  {r.kind}
                </span>
                <span className="truncate text-[11.5px]" title={r.explanation} style={{ color: "var(--dim)" }}>
                  {r.explanation}
                </span>
              </div>
            ))}
          </div>
        )}
      </Section>
    </div>
  );
}
