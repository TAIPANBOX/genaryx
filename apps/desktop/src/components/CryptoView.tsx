import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { scanCbom, scanNcsc } from "../lib/crypto";
import { useCryptoStatus } from "../lib/useCryptoStatus";
import type { CryptoError, CryptoStatus, NcscReport } from "../cryptoTypes";
import { CryptoCbomTable } from "./CryptoCbomTable";
import { CryptoEvidence } from "./CryptoEvidence";
import { CryptoFindingsTable } from "./CryptoFindingsTable";
import { CryptoTimeline } from "./CryptoTimeline";

function SectionHeader({ title }: { title: string }) {
  return (
    <span className="mono" style={{ fontSize: 11, letterSpacing: "0.1em", textTransform: "uppercase", color: "var(--faint)" }}>
      {title}
    </span>
  );
}

/**
 * Shared "not ready yet" rendering for the Crypto view - mirrors
 * `QualityView.tsx`'s local `QualityEmptyState`, Qryx-flavored: still
 * resolving, or no `qryx` binary found. No "unreachable" state (see
 * `crypto::state`'s doc comment for why Crypto has none).
 */
function CryptoEmptyState({ status }: { status: CryptoStatus | null }) {
  if (!status || status.state === "bootstrapping") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center">
        <div className="mono text-[12px]" style={{ color: "var(--faint)" }}>
          resolving the qryx binary...
        </div>
      </div>
    );
  }

  // `status.state === "ready"`: callers only render this component when NOT
  // ready, so only `no_environment` reaches here in practice.
  return (
    <div className="flex-1 min-h-0 flex items-center justify-center px-6">
      <div className="panel px-5 py-4 flex flex-col gap-2" style={{ background: "var(--panel-2)", maxWidth: 520 }}>
        <span style={{ fontSize: 13, color: "var(--fg)" }}>No crypto plane found</span>
        <span className="mono text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.7 }}>
          Qryx has no server to connect to either - it is a pure on-demand CLI. Build/install it at{" "}
          <span style={{ color: "var(--fg)" }}>~/.taipan/bin/qryx</span> for the console to auto-discover it.
        </span>
      </div>
    </div>
  );
}

/**
 * The Crypto panel (docs/PHASE4.md W1): the PQC readiness timeline (hero),
 * quantum-vulnerable findings, CBOM inventory, and evidence build/verify,
 * over an on-demand Qryx CLI. Unlike Quality/Identity, nothing here is a
 * "load once" snapshot at all - qryx has no live feed and no store to poll;
 * every result is genuinely "as of last scan" and only ever changes when the
 * operator clicks Scan (docs/PHASE4.md: "Qryx is on-demand (not a live
 * feed)").
 */
export function CryptoView() {
  const status = useCryptoStatus();
  const ready = status?.state === "ready";

  const [targetPath, setTargetPath] = useState("");
  useEffect(() => {
    if (status?.state === "ready" && targetPath.length === 0) {
      setTargetPath(status.default_target);
    }
  }, [status, targetPath]);

  const [ncsc, setNcsc] = useState<NcscReport | null>(null);
  const [ncscError, setNcscError] = useState<CryptoError | null>(null);
  const [ncscLoading, setNcscLoading] = useState(false);

  const [cbom, setCbom] = useState<unknown>(null);
  const [cbomError, setCbomError] = useState<CryptoError | null>(null);
  const [cbomLoading, setCbomLoading] = useState(false);

  const [scannedAtMs, setScannedAtMs] = useState<number | null>(null);
  const [scanning, setScanning] = useState(false);

  const runScan = useCallback(async () => {
    if (!ready || targetPath.trim().length === 0) return;
    setScanning(true);
    setNcscLoading(true);
    setCbomLoading(true);
    setNcscError(null);
    setCbomError(null);

    const [ncscResult, cbomResult] = await Promise.allSettled([scanNcsc(targetPath), scanCbom(targetPath)]);

    if (ncscResult.status === "fulfilled") {
      setNcsc(ncscResult.value);
    } else {
      setNcscError(ncscResult.reason as CryptoError);
    }
    if (cbomResult.status === "fulfilled") {
      setCbom(cbomResult.value);
    } else {
      setCbomError(cbomResult.reason as CryptoError);
    }

    setNcscLoading(false);
    setCbomLoading(false);
    setScanning(false);
    setScannedAtMs(Date.now());
  }, [ready, targetPath]);

  if (!ready) {
    return <CryptoEmptyState status={status} />;
  }

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-6">
      <div className="flex flex-wrap items-center gap-2">
        <span className="chip" style={cssVar("dot", "var(--src-qryx)")}>
          <span className="dot" aria-hidden="true" />
          {status.qryx_bin}
        </span>
        <span className="chip" style={cssVar("dot", "var(--faint)")}>
          <span className="dot" aria-hidden="true" />
          {scannedAtMs !== null ? `as of last scan · ${new Date(scannedAtMs).toLocaleTimeString()}` : "no scan run yet"}
        </span>
      </div>

      <div className="panel px-4 py-3 flex items-center gap-2" style={{ background: "var(--panel-2)" }}>
        <span className="text-[11.5px] shrink-0" style={{ color: "var(--dim)" }}>
          scan target
        </span>
        <input
          className="mono flex-1 min-w-0"
          style={{
            background: "var(--panel)",
            border: "1px solid var(--line-2)",
            borderRadius: 8,
            padding: "6px 10px",
            fontSize: 12,
            color: "var(--fg)",
          }}
          value={targetPath}
          onChange={(e) => setTargetPath(e.target.value)}
          placeholder="/path/to/scan"
          spellCheck={false}
        />
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 12px", fontSize: 11 }}
          onClick={() => void runScan()}
          disabled={scanning}
        >
          {scanning ? "Scanning..." : "Scan"}
        </button>
      </div>

      <section className="flex flex-col gap-2">
        <SectionHeader title="PQC Readiness Timeline" />
        <CryptoTimeline report={ncsc} loading={ncscLoading} error={ncscError} />
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Quantum-Vulnerable Findings" />
        <CryptoFindingsTable findings={ncsc?.discovery2028.quantumVulnerableFindings ?? null} />
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="CBOM Inventory" />
        <CryptoCbomTable value={cbom} loading={cbomLoading} error={cbomError} />
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Evidence" />
        <CryptoEvidence defaultPath={targetPath} />
      </section>
    </div>
  );
}
