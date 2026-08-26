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
import { CredentialsKeysTable } from "./CredentialsKeysTable";
import { useCredentialsStatus } from "../lib/useCredentialsStatus";
import {
  deriveKeyStatus,
  describeCredentialsError,
  fetchCredentialsKeys,
  isKeyIssue,
  type CredentialsError,
  type CredentialsStatus,
  type GatewayKeysReport,
} from "../lib/credentials";
import { buildAccessRows } from "../lib/access";
import { AccessMatrixTable } from "./AccessMatrixTable";
import { describePolicyError, fetchPolicies } from "../lib/policy";
import { usePolicyStatus } from "../lib/usePolicyStatus";
import type { PolicyError, PolicyRecord, PolicyStatus } from "../policyTypes";
import { downloadCsv, downloadJson, type ExportMeta } from "../lib/download";
import {
  ACCESS_COLUMNS,
  ALERT_COLUMNS,
  IDENTITY_COLUMNS,
  KEY_COLUMNS,
  REMEDIATION_COLUMNS,
  accessExportMeta,
  accessExportRows,
  alertExportMeta,
  alertExportRows,
  gatewayEnvironment,
  identityExportMeta,
  identityExportRows,
  idryxEnvironment,
  keyExportMeta,
  keyExportRows,
  remediationExportMeta,
  remediationExportRows,
  type ExportColumn,
  type SnapshotMetaInput,
} from "../lib/identityExport";

/** The browser this console is being read in. Only ever half of an export's
 * `environment`: the other half, the plane's own URL, is the part a reader of
 * the file days later cannot recover. */
function consoleHost(): string {
  return window.location.host;
}

function isoOrNull(ms: number | null): string | null {
  return ms === null ? null : new Date(ms).toISOString();
}

/**
 * CSV/JSON for one table, in the shape `StatsView.tsx` already established.
 *
 * `meta` is a thunk rather than a value: it is evaluated at CLICK time so that
 * `takenAt` is when the file was actually saved, not when this component last
 * rendered. Disabled on an empty table, because a provenance block over zero
 * rows is a document that says nothing while looking like a record.
 */
function ExportButtons<T>({
  name,
  columns,
  rows,
  meta,
}: {
  name: string;
  columns: ExportColumn<T>[];
  rows: T[] | null;
  meta: () => ExportMeta;
}) {
  const ready = rows !== null && rows.length > 0;
  const style = {
    background: "var(--panel-2)",
    color: "var(--dim)",
    border: "1px solid var(--line)",
    opacity: ready ? 1 : 0.4,
  };
  return (
    <span className="flex items-center gap-1">
      <button
        type="button"
        className="mono text-[11px] px-2 py-1 rounded"
        style={style}
        disabled={!ready}
        title={ready ? `Save this table as ${name}.csv, with its provenance block` : "nothing to export yet"}
        onClick={() => rows && downloadCsv(`${name}.csv`, columns, rows, meta())}
      >
        CSV
      </button>
      <button
        type="button"
        className="mono text-[11px] px-2 py-1 rounded"
        style={style}
        disabled={!ready}
        title={ready ? `Save this table as ${name}.json, with its provenance block` : "nothing to export yet"}
        onClick={() => rows && downloadJson(`${name}.json`, rows, meta())}
      >
        JSON
      </button>
    </span>
  );
}

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
 * The Credentials card (I15 "key lifecycle health"): its own connection
 * states (mirroring `IdentityEmptyState`'s three-way honest split, gateway-
 * flavored), a hint chip when the environment has no identity map configured
 * (so the "unbound" status can never fire - `lib/credentials.ts`'s
 * `deriveKeyStatus` doc comment), a one-line unauthorized-attempts summary,
 * and the key table itself. Rendered unconditionally by `IdentityView`
 * regardless of the idryx-backed sections' own readiness - see that
 * component's doc comment for why.
 */
function CredentialsSection({
  status,
  report,
  error,
  nowMs,
}: {
  status: CredentialsStatus | null;
  report: GatewayKeysReport | null;
  error: CredentialsError | null;
  nowMs: number;
}) {
  const unauthorized = report?.unauthorized_since_startup;

  return (
    <Section
      title="Credentials"
      right={
        <div className="flex items-center gap-2">
          {/* Rendered only with a report in hand, so the meta thunk never has
              to invent one: an export of a gateway this console never reached
              would carry a provenance block over nothing. */}
          {report !== null && (
            <ExportButtons
              name="genaryx-gateway-keys"
              columns={KEY_COLUMNS}
              rows={keyExportRows(report, nowMs)}
              meta={() =>
                keyExportMeta({
                  environment: gatewayEnvironment(status, consoleHost()),
                  takenAt: new Date().toISOString(),
                  report,
                  nowMillis: nowMs,
                })
              }
            />
          )}
          {report && !report.identity_map_configured && (
            <span
              className="chip"
              style={cssVar("dot", "var(--faint)")}
              title="This environment has no identity map configured, so key-to-unit bindings cannot be checked (tokenfuse docs/20)."
            >
              <span className="dot" aria-hidden="true" />
              identity map off (docs/20)
            </span>
          )}
          <FreshBadge variant="auto" detail="30s" title="Polls the gateway's GET /v1/keys every 30 seconds" />
        </div>
      }
    >
      {error && (
        <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
          {describeCredentialsError(error)}
        </div>
      )}

      {!status || status.state === "bootstrapping" ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          connecting to the gateway...
        </div>
      ) : status.state === "no_environment" ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no gateway in this environment.
        </div>
      ) : status.state === "unreachable" ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--sev-high)", fontSize: 12 }}>
          could not reach the gateway: {status.reason}
        </div>
      ) : report === null ? (
        <Loading />
      ) : report.keys.length === 0 ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no client keys configured.
        </div>
      ) : (
        <>
          {unauthorized && unauthorized.attempts > 0 && (
            <div className="mono px-5 py-2 text-[11.5px]" style={{ color: "var(--sev-high)" }}>
              {unauthorized.attempts} unauthorized attempt{unauthorized.attempts === 1 ? "" : "s"} since gateway
              start
              {unauthorized.last_millis !== null ? `, last at ${formatHm(unauthorized.last_millis)}` : ""}
            </div>
          )}
          <CredentialsKeysTable report={report} nowMs={nowMs} />
        </>
      )}
    </Section>
  );
}

/**
 * The Access matrix (I5): one row per agent identity - permissions granted
 * vs used, MCP reach (sanctioned vs shadow), and the Wardryx overlay -
 * assembled client-side by `lib/access.ts` from the SAME `identities`/
 * `alerts` this view already loaded above, plus an independent one-shot
 * Wardryx `policies` read (own `usePolicyStatus()`/`fetchPolicies()`,
 * mirroring the Credentials section's own "independent plane, own status
 * hook" precedent just above - an environment can have idryx up and
 * wardryx down, or vice versa, and this section's identity-derived columns
 * stay meaningful either way).
 *
 * Rendered unconditionally, like `CredentialsSection` - NOT gated behind
 * this view's own idryx `ready` - so "identity plane not ready" has
 * somewhere to actually say so (nesting it inside the `ready` branch the
 * way Identities/Alerts/Remediations are would mean this section simply
 * never renders when identity is not ready, and the honesty requirement
 * would never be reachable). The wardryx-derived column group (denied
 * tools/domains/flags) degrades independently: `buildAccessRows` is only
 * ever given a real `policies` array once `policyReady && policies !==
 * null`, otherwise `null` - which `AccessMatrixTable` renders as "policy
 * plane unavailable" per cell, never a fabricated zero (see
 * `lib/access.ts`'s `AccessRow.policy` doc comment for why those two must
 * never look the same).
 */
function AccessMatrixSection({
  identityReady,
  identityStatus,
  identities,
  alerts,
  policyStatus,
  policiesError,
  policies,
  hhmm,
  when,
  onOpenAgent,
}: {
  identityReady: boolean;
  identityStatus: IdentityStatus | null;
  identities: IdryxIdentity[] | null;
  alerts: IdryxAlert[] | null;
  policyStatus: PolicyStatus | null;
  policiesError: PolicyError | null;
  policies: PolicyRecord[] | null;
  hhmm: string | undefined;
  /** When each of the two idryx reads happened, for the export's provenance
   * block. Not `hhmm`: an "11:58" in a file read next month names no day. */
  when: { snapshotAt: string | null; rescanAt: string | null };
  onOpenAgent: (agentId: string) => void;
}) {
  const policyReady = policyStatus?.state === "ready";
  const policiesAvailable = policyReady && policies !== null && policiesError === null;

  let policyNote: string | null = null;
  if (!policiesAvailable) {
    if (!policyStatus || policyStatus.state === "bootstrapping") policyNote = "policy plane connecting...";
    else if (policyStatus.state === "no_environment") policyNote = "policy plane not configured";
    else if (policyStatus.state === "unreachable") policyNote = `policy plane unreachable: ${policyStatus.reason}`;
    else if (policiesError) policyNote = describePolicyError(policiesError);
    else policyNote = "policy plane unavailable";
  }

  const rows =
    identityReady && identities !== null && alerts !== null
      ? buildAccessRows(identities, alerts, policiesAvailable ? policies : null)
      : null;

  return (
    <Section
      title="Access matrix"
      right={
        <div className="flex items-center gap-2">
          <ExportButtons
            name="genaryx-access-matrix"
            columns={ACCESS_COLUMNS}
            rows={rows === null ? null : accessExportRows(rows)}
            meta={() =>
              accessExportMeta({
                environment: idryxEnvironment(identityStatus, consoleHost()),
                takenAt: new Date().toISOString(),
                ...when,
                policyNote,
                rows: rows ?? [],
              })
            }
          />
          {policyNote && (
            <span className="chip" style={cssVar("dot", "var(--faint)")} title={policyNote}>
              <span className="dot" aria-hidden="true" />
              {policyNote}
            </span>
          )}
          <FreshBadge
            variant="snapshot"
            detail={hhmm}
            title="Built from the same idryx identities/alerts snapshot as the sections above, plus a one-shot Wardryx policies read"
          />
        </div>
      }
    >
      {!identityReady ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          {!identityStatus || identityStatus.state === "bootstrapping"
            ? "connecting to the identity plane"
            : identityStatus.state === "no_environment"
              ? "no identity plane found"
              : "could not reach idryx"}{" "}
          - the access matrix needs idryx's identities/alerts snapshot to build any row.
        </div>
      ) : rows === null ? (
        <Loading />
      ) : (
        <AccessMatrixTable rows={rows} onOpenAgent={onOpenAgent} />
      )}
    </Section>
  );
}

/**
 * `GET /api/remediations`, all four fields of it.
 *
 * `code` and `created_at` were on the wire and rendered nowhere. `created_at`
 * is the one an operator acts on: a right-size suggestion computed months ago
 * describes an identity as it was then, and a list with no dates on it reads
 * as if every row were current.
 *
 * `code` is deliberately NOT labelled. Nothing in this console, the connector
 * or the wire types says what idryx puts there, so the row shows it verbatim
 * behind a plain "code" toggle and the legend says the console does not
 * interpret it. Calling it a fix, a command or a patch would be this console
 * asserting something it has never read.
 */
function RemediationsTable({ remediations }: { remediations: IdryxRecommendation[] }) {
  const [openIdx, setOpenIdx] = useState<number | null>(null);
  const columns = "1fr 110px 170px 1fr 70px";

  return (
    <div className="flex flex-col gap-2">
      <span className="px-5 text-[11px]" style={{ color: "var(--faint)" }}>
        created is the date idryx attached to the suggestion, empty where it sent none. code is whatever idryx
        returns beside the explanation, shown exactly as sent: this console does not interpret it and makes no claim
        about what it is for.
      </span>
      <div style={{ overflowX: "auto" }}>
        <div
          className="grid gap-3 px-5 py-2"
          style={{ gridTemplateColumns: columns, borderBottom: "1px solid var(--line)" }}
        >
          {["identity", "kind", "created", "explanation", "code"].map((label) => (
            <span
              key={label}
              className="mono"
              style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
            >
              {label}
            </span>
          ))}
        </div>
        {remediations.map((r, idx) => (
          <div key={`${r.identity}-${r.kind}-${idx}`}>
            <div className="grid items-center gap-3 px-5 py-2.5 bus-row" style={{ gridTemplateColumns: columns }}>
              <span className="mono truncate text-[11.5px]" title={r.identity} style={{ color: "var(--fg)" }}>
                {r.identity}
              </span>
              <span
                className="badge"
                style={cssVar("tone", r.kind === "rotation" ? "var(--sev-medium)" : "var(--sev-info)")}
              >
                {r.kind}
              </span>
              <span className="mono truncate text-[11px]" title={r.created_at || undefined} style={{ color: "var(--dim)" }}>
                {r.created_at !== "" ? r.created_at : <span style={{ color: "var(--faint)" }}>-</span>}
              </span>
              <span className="truncate text-[11.5px]" title={r.explanation} style={{ color: "var(--dim)" }}>
                {r.explanation}
              </span>
              {r.code !== "" ? (
                <button
                  type="button"
                  className="mono text-[11px] px-2 py-0.5 rounded"
                  aria-expanded={openIdx === idx}
                  style={{
                    background: "var(--panel-2)",
                    color: "var(--dim)",
                    border: "1px solid var(--line)",
                    cursor: "pointer",
                  }}
                  onClick={() => setOpenIdx(openIdx === idx ? null : idx)}
                >
                  {openIdx === idx ? "hide" : "show"}
                </button>
              ) : (
                <span style={{ color: "var(--faint)" }}>-</span>
              )}
            </div>
            {openIdx === idx && r.code !== "" && (
              <pre
                className="mono text-[11px] mx-5 mb-2 px-3 py-2"
                style={{
                  color: "var(--fg)",
                  background: "var(--panel-2)",
                  border: "1px solid var(--line)",
                  borderRadius: 8,
                  whiteSpace: "pre-wrap",
                  wordBreak: "break-word",
                }}
              >
                {r.code}
              </pre>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

/** 30s poll cadence for the Credentials card (I15) - the gateway's key
 * report changes as calls come in, unlike idryx's load-once snapshot above,
 * so this one genuinely benefits from a periodic re-fetch. */
const CREDENTIALS_REFRESH_MS = 30_000;

/**
 * The Identity panel (docs/PHASE3.md W2): an Identities list (type
 * filters), an Alerts stream (severity + detector filters, plus Rescan),
 * a Remediations list, and a Credentials card (I15 "key lifecycle health"),
 * over a read-only Idryx connection plus an entirely independent read-only
 * gateway connection. Mirrors `PolicyView.tsx`'s overall shape (status hook,
 * empty state, section layout) but deliberately does NOT mirror its 20s
 * periodic auto-refresh FOR THE IDRYX-BACKED SECTIONS: `idryx serve` is a
 * load-once snapshot (docs/PHASE3.md - "Polling /api/* returns
 * byte-identical data for the process lifetime"), so a timer here would
 * either be a no-op or, worse, silently overwrite a fresher Rescan result
 * with the older REST snapshot. Data only ever changes via the explicit
 * Refresh button (re-reads the REST snapshot, e.g. after idryx was
 * restarted externally) or Rescan (recomputes and replaces the alerts
 * list only - idryx's CLI batch output is alerts-only, it can never refresh
 * the identities/remediations lists).
 *
 * The Credentials card is the one exception, and deliberately so: it reads
 * the gateway's `GET /v1/keys`, which is live (call counts and mismatches
 * change constantly), so it polls every `CREDENTIALS_REFRESH_MS` on its own,
 * exactly like `MoneyView.tsx`'s `REFRESH_INTERVAL_MS` convention. It is
 * ALSO deliberately NOT gated behind this component's own `ready` (idryx)
 * check: the gateway is a separate descriptor service
 * (`services.gateway`, not `services.idryx`) that resolves independently
 * (see `useCredentialsStatus`'s doc comment) - the common case of an
 * environment brought up without `--with idryx` must still show a working
 * Credentials card, mirroring `PostureView.tsx`'s own "never gated behind a
 * single-plane ready check" rule for exactly this class of problem.
 */
export function IdentityView({ onOpenAgent }: { onOpenAgent: (agentId: string) => void }) {
  const status = useIdentityStatus();
  const ready = status?.state === "ready";

  const [identities, setIdentities] = useState<IdryxIdentity[] | null>(null);
  const [alerts, setAlerts] = useState<IdryxAlert[] | null>(null);
  const [remediations, setRemediations] = useState<IdryxRecommendation[] | null>(null);
  const [error, setError] = useState<IdentityError | null>(null);
  // Two timestamps, not one, and they answer different questions. `asOfMs` is
  // when the REST snapshot was last read; `rescanAtMs` is when a `detect` pass
  // last REPLACED the alert list and is null until one has. A saved file has
  // to be able to say that its alerts and its identities came from different
  // reads, and a single "as of" cannot say it. Refresh clears the rescan,
  // because it puts the snapshot's own alerts back.
  const [asOfMs, setAsOfMs] = useState<number | null>(null);
  const [rescanAtMs, setRescanAtMs] = useState<number | null>(null);
  const [rescanning, setRescanning] = useState(false);

  const load = useCallback(async () => {
    if (!ready) return;
    try {
      const [i, a, r] = await Promise.all([fetchIdentities(), fetchAlerts(), fetchRemediations()]);
      setIdentities(i);
      setAlerts(a);
      setRemediations(r);
      setAsOfMs(Date.now());
      setRescanAtMs(null);
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
      setRescanAtMs(Date.now());
      setError(null);
    } catch (err) {
      setError(err as IdentityError);
    } finally {
      setRescanning(false);
    }
  }, []);

  // Credentials (I15): an entirely independent plane/status hook from idryx
  // above - see this component's doc comment for why it is never gated
  // behind `ready`.
  const credentialsStatus = useCredentialsStatus();
  const credentialsReady = credentialsStatus?.state === "ready";
  const [keysReport, setKeysReport] = useState<GatewayKeysReport | null>(null);
  const [credentialsError, setCredentialsError] = useState<CredentialsError | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());

  const loadCredentials = useCallback(async () => {
    if (!credentialsReady) return;
    try {
      const report = await fetchCredentialsKeys();
      setKeysReport(report);
      setCredentialsError(null);
    } catch (err) {
      setCredentialsError(err as CredentialsError);
    }
  }, [credentialsReady]);

  useEffect(() => {
    void loadCredentials();
    const id = window.setInterval(() => void loadCredentials(), CREDENTIALS_REFRESH_MS);
    return () => window.clearInterval(id);
  }, [loadCredentials]);

  // Drives every age-based label in the Credentials card (key-status
  // derivation, "last seen" cells) without needing a fresh fetch just to
  // re-evaluate staleness - same rationale as `lib/usePostureData.ts`'s
  // `NOW_TICK_MS`.
  useEffect(() => {
    const id = window.setInterval(() => setNowMs(Date.now()), 5_000);
    return () => window.clearInterval(id);
  }, []);

  const keyIssuesCount = keysReport
    ? keysReport.keys.filter((k) => isKeyIssue(deriveKeyStatus(k, keysReport, nowMs))).length
    : 0;

  // Access matrix (I5): an entirely independent plane/status hook from idryx
  // above, same pattern as Credentials - see `AccessMatrixSection`'s doc
  // comment for why it is never gated behind `ready`. One-shot fetch, no
  // interval: wardryx policies are not a live-changing feed the way the
  // gateway's key report is, and this view's own "no new polling loops"
  // rule (I5 spec) applies here just as it does to the idryx-backed
  // sections above.
  const policyStatus = usePolicyStatus();
  const policyReady = policyStatus?.state === "ready";
  const [policies, setPolicies] = useState<PolicyRecord[] | null>(null);
  const [policiesError, setPoliciesError] = useState<PolicyError | null>(null);

  useEffect(() => {
    if (!policyReady) return;
    let cancelled = false;
    setPolicies(null);
    setPoliciesError(null);
    void fetchPolicies()
      .then((p) => {
        if (!cancelled) setPolicies(p);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setPolicies(null);
        setPoliciesError(err as PolicyError);
      });
    return () => {
      cancelled = true;
    };
  }, [policyReady]);

  // The badge shows the later of the two reads, which is what "as of" means on
  // screen. The exports get both separately.
  const latestMs =
    asOfMs === null ? rescanAtMs : rescanAtMs === null ? asOfMs : Math.max(asOfMs, rescanAtMs);
  const hhmm = latestMs !== null ? formatHm(latestMs) : undefined;
  const when = { snapshotAt: isoOrNull(asOfMs), rescanAt: isoOrNull(rescanAtMs) };
  const snapshotMeta = (): SnapshotMetaInput => ({
    environment: idryxEnvironment(status, consoleHost()),
    takenAt: new Date().toISOString(),
    ...when,
  });

  const credentialsSection = (
    <CredentialsSection
      status={credentialsStatus}
      report={keysReport}
      error={credentialsError}
      nowMs={nowMs}
    />
  );

  const accessMatrixSection = (
    <AccessMatrixSection
      identityReady={ready}
      identityStatus={status}
      identities={identities}
      alerts={alerts}
      policyStatus={policyStatus}
      policiesError={policiesError}
      policies={policies}
      hhmm={hhmm}
      when={when}
      onOpenAgent={onOpenAgent}
    />
  );

  if (!ready) {
    return (
      <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-4">
        <IdentityEmptyState status={status} />
        {accessMatrixSection}
        {credentialsSection}
      </div>
    );
  }

  const attestationAlerts = (alerts ?? []).filter((a) => ATTESTATION_DETECTORS.has(a.detector));
  const privilegedCount = (identities ?? []).filter((i) => i.privileged).length;

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
              <KpiTile
                label="Key issues"
                value={keyIssuesCount.toLocaleString("en-US")}
                tone={keyIssuesCount > 0 ? "var(--sev-high)" : "var(--mint)"}
                sub="removed · dangling · unbound · mismatching"
              />
            </>
          }
        />
      )}

      <Section
        title="Identities"
        right={
          <div className="flex items-center gap-2">
            <ExportButtons
              name="genaryx-identities"
              columns={IDENTITY_COLUMNS}
              rows={identities === null ? null : identityExportRows(identities)}
              meta={() => identityExportMeta({ ...snapshotMeta(), identities: identities ?? [] })}
            />
            <FreshBadge variant="snapshot" detail={hhmm} />
          </div>
        }
      >
        {identities === null ? <Loading /> : <IdentityList identities={identities} onOpenAgent={onOpenAgent} />}
      </Section>

      <Section
        title="Alerts"
        right={
          <div className="flex items-center gap-2">
            <ExportButtons
              name="genaryx-identity-alerts"
              columns={ALERT_COLUMNS}
              rows={alerts === null ? null : alertExportRows(alerts)}
              meta={() => alertExportMeta(snapshotMeta())}
            />
            <FreshBadge variant="snapshot" detail={hhmm} />
          </div>
        }
      >
        {alerts === null ? <Loading /> : <IdentityAlerts alerts={alerts} onOpenAgent={onOpenAgent} />}
      </Section>

      <Section
        title="Remediations"
        right={
          <div className="flex items-center gap-2">
            <ExportButtons
              name="genaryx-identity-remediations"
              columns={REMEDIATION_COLUMNS}
              rows={remediations === null ? null : remediationExportRows(remediations)}
              meta={() => remediationExportMeta(snapshotMeta())}
            />
            <FreshBadge variant="snapshot" detail={hhmm} />
          </div>
        }
      >
        {remediations === null ? (
          <Loading />
        ) : remediations.length === 0 ? (
          <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
            no remediations in this snapshot.
          </div>
        ) : (
          <RemediationsTable remediations={remediations} />
        )}
      </Section>

      {accessMatrixSection}
      {credentialsSection}
    </div>
  );
}
