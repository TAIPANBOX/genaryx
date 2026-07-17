import type { ReactNode } from "react";
import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { buildEvidence, describeEvidenceError, downloadEvidencePack } from "../lib/evidence";
import { fetchMoneyStatus } from "../lib/money";
import { useEvidenceStatus } from "../lib/useEvidenceStatus";
import type { EvidenceBuildResult, EvidenceError } from "../evidenceTypes";
import type { MoneyStatus } from "../moneyTypes";
import { EvidenceManifestView } from "./EvidenceManifestView";

const FIELD_STYLE = {
  background: "var(--panel)",
  border: "1px solid var(--line-2)",
  borderRadius: 8,
  padding: "6px 10px",
  fontSize: 12,
  color: "var(--fg)",
} as const;

/** `money_status`'s current state, polled independently of the Money view
 * itself (Evidence needs to KNOW whether Cloud is available, not manage its
 * connection - see `evidence::commands`'s module doc for why this panel
 * never re-derives Money's own state on the Rust side). Reuses
 * `fetchMoneyStatus` directly rather than `useMoneyStatus`'s polling hook: a
 * single fetch on mount is enough here (the Cloud checkbox just needs an
 * honest snapshot, not a live-updating connection indicator the way the
 * Money view itself is). */
function useCloudAvailability(): { status: MoneyStatus | null; available: boolean; hint: string } {
  const [status, setStatus] = useState<MoneyStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    void fetchMoneyStatus().then((s) => {
      if (!cancelled) setStatus(s);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const available = status?.state === "ready";
  let hint = "";
  if (!status || status.state === "bootstrapping") hint = "still connecting to the Cloud (see Money)";
  else if (status.state === "no_environment") hint = "no TokenFuse Cloud environment found (see Money)";
  else if (status.state === "pairing_failed") hint = `pairing failed: ${status.reason}`;

  return { status, available, hint };
}

function SourceRow({
  label,
  detail,
  checked,
  available,
  hint,
  onChange,
  children,
}: {
  label: string;
  detail: string;
  checked: boolean;
  available: boolean;
  hint: string;
  onChange: (next: boolean) => void;
  children?: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5 py-2" style={{ borderBottom: "1px solid var(--line-2)" }}>
      <label className="flex items-start gap-2.5" style={{ cursor: available ? "pointer" : "not-allowed" }}>
        <input
          type="checkbox"
          checked={checked && available}
          disabled={!available}
          onChange={(e) => onChange(e.target.checked)}
          style={{ marginTop: 2 }}
        />
        <div className="flex flex-col gap-0.5 min-w-0">
          <span className="text-[12.5px]" style={{ color: available ? "var(--fg)" : "var(--faint)" }}>
            {label}
          </span>
          <span className="text-[11px]" style={{ color: "var(--dim)" }}>
            {detail}
          </span>
          {!available && (
            <span className="mono text-[10.5px]" style={{ color: "var(--faint)" }}>
              unavailable - {hint}
            </span>
          )}
        </div>
      </label>
      {available && checked && children && <div className="pl-6">{children}</div>}
    </div>
  );
}

/**
 * Honest empty state when NOTHING is resolvable - no Cloud pairing AND none
 * of qryx/idryx/tokenfuse resolved. A normal, renderable state (a fresh box
 * with no `taipan up` environment and no local tools built yet), never an
 * error.
 */
function EvidenceEmptyState({ resolving }: { resolving: boolean }) {
  if (resolving) {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center">
        <div className="mono text-[12px]" style={{ color: "var(--faint)" }}>
          resolving evidence sources...
        </div>
      </div>
    );
  }
  return (
    <div className="flex-1 min-h-0 flex items-center justify-center px-6">
      <div className="panel px-5 py-4 flex flex-col gap-2" style={{ background: "var(--panel-2)", maxWidth: 560 }}>
        <span style={{ fontSize: 13, color: "var(--fg)" }}>No evidence sources available</span>
        <span className="mono text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.7 }}>
          Evidence needs at least one of: a paired TokenFuse Cloud (see the Money view), the{" "}
          <span style={{ color: "var(--fg)" }}>qryx</span> binary, the{" "}
          <span style={{ color: "var(--fg)" }}>idryx</span> binary, or the{" "}
          <span style={{ color: "var(--fg)" }}>tokenfuse-gateway</span> binary at{" "}
          <span style={{ color: "var(--fg)" }}>~/.taipan/bin</span>. Bring one up (or build/install a tool there)
          for the console to auto-discover it.
        </span>
      </div>
    </div>
  );
}

/**
 * The Evidence Center panel (docs/PHASE4.md W3): choose sources, build (and,
 * when Cloud is paired, sign) one pack, download it, and inspect its
 * manifest. Genuinely on-demand, like Crypto/Drills: nothing here ever
 * auto-builds - only an explicit "Build evidence pack" click does.
 */
export function EvidenceView() {
  const evidenceStatus = useEvidenceStatus();
  const { available: cloudAvailable, hint: cloudHint } = useCloudAvailability();

  const evidenceReady = evidenceStatus?.state === "ready";
  const qryxAvailable = evidenceReady && evidenceStatus.qryx_available;
  const idryxAvailable = evidenceReady && evidenceStatus.idryx_available;
  const tokenfuseAvailable = evidenceReady && evidenceStatus.tokenfuse_available;

  const [includeCloud, setIncludeCloud] = useState(false);
  const [includeQryx, setIncludeQryx] = useState(false);
  const [qryxTarget, setQryxTarget] = useState("");
  const [includeIdryx, setIncludeIdryx] = useState(false);
  const [includeTokenfuse, setIncludeTokenfuse] = useState(false);
  const [tokenfuseTracesDir, setTokenfuseTracesDir] = useState("");

  useEffect(() => {
    if (evidenceReady && qryxTarget.length === 0 && evidenceStatus.qryx_default_target) {
      setQryxTarget(evidenceStatus.qryx_default_target);
    }
    if (evidenceReady && tokenfuseTracesDir.length === 0 && evidenceStatus.tokenfuse_default_traces_dir) {
      setTokenfuseTracesDir(evidenceStatus.tokenfuse_default_traces_dir);
    }
    // Prefill once, on the first `ready` status - never overwrite an
    // operator's own edit on a later re-poll.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [evidenceReady]);

  const [building, setBuilding] = useState(false);
  const [result, setResult] = useState<EvidenceBuildResult | null>(null);
  const [error, setError] = useState<EvidenceError | null>(null);
  const [builtAtMs, setBuiltAtMs] = useState<number | null>(null);

  const canBuild =
    (includeCloud && cloudAvailable) ||
    (includeQryx && qryxAvailable) ||
    (includeIdryx && idryxAvailable) ||
    (includeTokenfuse && tokenfuseAvailable);

  const onBuild = useCallback(async () => {
    if (!canBuild || building) return;
    setBuilding(true);
    setError(null);
    try {
      const r = await buildEvidence({
        include_cloud: includeCloud && cloudAvailable,
        include_qryx: includeQryx && qryxAvailable,
        qryx_target: qryxTarget.trim().length > 0 ? qryxTarget : null,
        include_idryx: includeIdryx && idryxAvailable,
        include_tokenfuse: includeTokenfuse && tokenfuseAvailable,
        tokenfuse_traces_dir: tokenfuseTracesDir.trim().length > 0 ? tokenfuseTracesDir : null,
      });
      setResult(r);
      setBuiltAtMs(Date.now());
      downloadEvidencePack(r);
    } catch (err) {
      setError(err as EvidenceError);
    } finally {
      setBuilding(false);
    }
  }, [
    canBuild,
    building,
    includeCloud,
    cloudAvailable,
    includeQryx,
    qryxAvailable,
    qryxTarget,
    includeIdryx,
    idryxAvailable,
    includeTokenfuse,
    tokenfuseAvailable,
    tokenfuseTracesDir,
  ]);

  const resolving = !evidenceReady;
  const nothingResolvable = evidenceReady && !cloudAvailable && !qryxAvailable && !idryxAvailable && !tokenfuseAvailable;

  if (resolving || nothingResolvable) {
    return <EvidenceEmptyState resolving={resolving} />;
  }

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-6">
      <div className="flex flex-wrap items-center gap-2">
        <span className="chip" style={cssVar("dot", "var(--faint)")}>
          <span className="dot" aria-hidden="true" />
          {builtAtMs !== null ? `as of last build · ${new Date(builtAtMs).toLocaleTimeString()}` : "no pack built yet"}
        </span>
      </div>

      <div className="panel px-4 py-3 flex flex-col gap-1" style={{ background: "var(--panel-2)" }}>
        <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
          Choose sources, then build one signed evidence pack. Signing needs a paired TokenFuse Cloud device (see
          Money) - without one, the pack is honestly built UNSIGNED.
        </span>

        <SourceRow
          label="Cloud"
          detail="EU AI Act / SR 11-7 / SOC 2 compliance evidence + the audit-chain verdict"
          checked={includeCloud}
          available={cloudAvailable}
          hint={cloudHint}
          onChange={setIncludeCloud}
        />

        <SourceRow
          label="Qryx"
          detail="CNSA crypto evidence (self-verifying digest) + the CBOM inventory"
          checked={includeQryx}
          available={qryxAvailable}
          hint={evidenceReady ? `qryx binary not found (looked at ${evidenceStatus.qryx_bin ?? "~/.taipan/bin/qryx"})` : ""}
          onChange={setIncludeQryx}
        >
          <input
            className="mono flex-1 min-w-0"
            style={{ ...FIELD_STYLE, width: "100%" }}
            value={qryxTarget}
            onChange={(e) => setQryxTarget(e.target.value)}
            placeholder="/path/to/scan"
            spellCheck={false}
          />
        </SourceRow>

        <SourceRow
          label="Agent-BOM"
          detail="idryx CycloneDX Agent-BOM, built from the stack-bus event loads it can see"
          checked={includeIdryx}
          available={idryxAvailable}
          hint={evidenceReady ? `idryx binary not found (looked at ${evidenceStatus.idryx_bin ?? "~/.taipan/bin/idryx"})` : ""}
          onChange={setIncludeIdryx}
        >
          {evidenceReady && evidenceStatus.idryx_load_sources.length > 0 ? (
            <span className="mono text-[10.5px]" style={{ color: "var(--faint)" }}>
              loads: {evidenceStatus.idryx_load_sources.join(", ")}
            </span>
          ) : (
            <span className="mono text-[10.5px]" style={{ color: "var(--faint)" }}>
              no stack-bus event files found to load from - the Agent-BOM will be honestly smaller
            </span>
          )}
        </SourceRow>

        <SourceRow
          label="FOCUS CSV"
          detail="TokenFuse FinOps cost export (FOCUS 1.2), one row per LLM call"
          checked={includeTokenfuse}
          available={tokenfuseAvailable}
          hint={
            evidenceReady
              ? `tokenfuse-gateway binary not found (looked at ${evidenceStatus.tokenfuse_bin ?? "~/.taipan/bin/tokenfuse-gateway"})`
              : ""
          }
          onChange={setIncludeTokenfuse}
        >
          <input
            className="mono flex-1 min-w-0"
            style={{ ...FIELD_STYLE, width: "100%" }}
            value={tokenfuseTracesDir}
            onChange={(e) => setTokenfuseTracesDir(e.target.value)}
            placeholder="/path/to/traces"
            spellCheck={false}
          />
        </SourceRow>

        <div className="flex items-center gap-3 flex-wrap pt-2">
          <button
            type="button"
            className="icon-btn"
            style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
            onClick={() => void onBuild()}
            disabled={building || !canBuild}
          >
            {building ? "Building..." : "Build evidence pack"}
          </button>
          <span className="text-[11px]" style={{ color: "var(--faint)" }}>
            downloads the zip immediately and shows its manifest below - never runs on its own.
          </span>
        </div>
      </div>

      {error && (
        <div className="panel px-3 py-2 mono text-[11.5px]" style={{ background: "var(--panel-2)", color: "var(--sev-high)" }}>
          {describeEvidenceError(error)}
        </div>
      )}

      {result === null ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          build a pack to see its manifest.
        </div>
      ) : (
        <EvidenceManifestView result={result} />
      )}
    </div>
  );
}
