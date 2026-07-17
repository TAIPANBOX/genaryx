import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { describeIdentityError, fetchAlerts, fetchIdentities, fetchRemediations, rescan } from "../lib/identity";
import { useIdentityStatus } from "../lib/useIdentityStatus";
import { ATTESTATION_DETECTORS } from "../identityTypes";
import type { IdentityError, IdentityStatus, IdryxAlert, IdryxIdentity, IdryxRecommendation } from "../identityTypes";
import { IdentityAlerts } from "./IdentityAlerts";
import { IdentityList } from "./IdentityList";

function SectionHeader({ title }: { title: string }) {
  return (
    <span className="mono" style={{ fontSize: 11, letterSpacing: "0.1em", textTransform: "uppercase", color: "var(--faint)" }}>
      {title}
    </span>
  );
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
export function IdentityView() {
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

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-6">
      <div className="flex flex-wrap items-center gap-2">
        <span className="chip" style={cssVar("dot", "var(--src-qryx)")}>
          <span className="dot" aria-hidden="true" />
          taipan up &middot; {status.source.name} &middot; {status.idryx_url}
        </span>
        <span className="chip" style={cssVar("dot", "var(--faint)")}>
          <span className="dot" aria-hidden="true" />
          as of load{asOfMs !== null ? ` · fetched ${new Date(asOfMs).toLocaleTimeString()}` : ""}
        </span>
        <div className="flex-1" />
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 10px", fontSize: 11 }}
          onClick={() => void load()}
        >
          Refresh
        </button>
      </div>

      <span className="text-[11px]" style={{ color: "var(--faint)" }}>
        idryx serve loads once at startup and never reloads on its own; Refresh re-reads that same snapshot, Rescan
        runs a fresh detect pass over the current bus files.
      </span>

      {error && (
        <div className="panel px-3 py-2 mono text-[11.5px]" style={{ background: "var(--panel-2)", color: "var(--sev-high)" }}>
          {describeIdentityError(error)}
        </div>
      )}

      <div className="panel px-3 py-2.5 flex items-center gap-3" style={{ background: "var(--panel-2)" }}>
        <span className="text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.6 }}>
          Attestation is not a clean field on an identity (idryx has none) - it surfaces only via{" "}
          <span className="mono" style={{ color: "var(--fg)" }}>attestation_missing</span> /{" "}
          <span className="mono" style={{ color: "var(--fg)" }}>bom_incomplete</span> alerts, below.
        </span>
        <div className="flex-1" />
        <span
          className="badge"
          style={cssVar("tone", attestationAlerts.length > 0 ? "var(--sev-high)" : "var(--sev-low)")}
        >
          {attestationAlerts.length} found
        </span>
      </div>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Identities" />
        {identities === null ? <Loading /> : <IdentityList identities={identities} />}
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Alerts" />
        {alerts === null ? (
          <Loading />
        ) : (
          <IdentityAlerts
            alerts={alerts}
            onRescan={() => void handleRescan()}
            rescanning={rescanning}
            rescanAvailable={status.rescan_available}
          />
        )}
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Remediations" />
        {remediations === null ? (
          <Loading />
        ) : remediations.length === 0 ? (
          <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
            no remediations in this snapshot.
          </div>
        ) : (
          <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
            {remediations.map((r, idx) => (
              <div
                key={`${r.identity}-${r.kind}-${idx}`}
                className="grid items-center gap-3 px-4 py-2.5 bus-row"
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
      </section>
    </div>
  );
}
