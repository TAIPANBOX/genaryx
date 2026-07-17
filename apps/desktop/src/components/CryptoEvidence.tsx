import { useCallback, useState } from "react";
import { describeCryptoError, scanEvidence, verifyEvidence } from "../lib/crypto";
import type { CryptoError, EvidenceReport, VerifyOutcome } from "../cryptoTypes";
import { StatTile } from "./StatTile";

function PathInput({
  value,
  onChange,
  placeholder,
}: {
  value: string;
  onChange: (next: string) => void;
  placeholder: string;
}) {
  return (
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
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      spellCheck={false}
    />
  );
}

/**
 * Evidence (docs/PHASE4.md W1 position 4): build an unsigned CNSA evidence
 * bundle for a target path, showing its summary (score %, by-severity,
 * digest, signature alg if present); and, as an independent action, verify a
 * SAVED evidence JSON file's digest+signature. The two are deliberately
 * separate forms rather than one "build then verify" flow - see
 * `crypto::commands`'s module doc for why `crypto_verify_evidence` cannot
 * safely operate on `scan_evidence`'s in-memory result.
 */
export function CryptoEvidence({ defaultPath }: { defaultPath: string }) {
  const [buildPath, setBuildPath] = useState(defaultPath);
  const [report, setReport] = useState<EvidenceReport | null>(null);
  const [buildError, setBuildError] = useState<CryptoError | null>(null);
  const [building, setBuilding] = useState(false);
  const [builtAtMs, setBuiltAtMs] = useState<number | null>(null);

  const [verifyPath, setVerifyPath] = useState("");
  const [outcome, setOutcome] = useState<VerifyOutcome | null>(null);
  const [verifyError, setVerifyError] = useState<CryptoError | null>(null);
  const [verifying, setVerifying] = useState(false);

  const onBuild = useCallback(async () => {
    if (buildPath.trim().length === 0) return;
    setBuilding(true);
    setBuildError(null);
    try {
      const r = await scanEvidence(buildPath);
      setReport(r);
      setBuiltAtMs(Date.now());
    } catch (err) {
      setBuildError(err as CryptoError);
    } finally {
      setBuilding(false);
    }
  }, [buildPath]);

  const onVerify = useCallback(async () => {
    if (verifyPath.trim().length === 0) return;
    setVerifying(true);
    setVerifyError(null);
    setOutcome(null);
    try {
      const o = await verifyEvidence(verifyPath);
      setOutcome(o);
    } catch (err) {
      setVerifyError(err as CryptoError);
    } finally {
      setVerifying(false);
    }
  }, [verifyPath]);

  return (
    <div className="flex flex-col gap-4">
      <div className="panel px-4 py-3 flex flex-col gap-2.5" style={{ background: "var(--panel-2)" }}>
        <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
          Build an unsigned CNSA evidence bundle for a target path.
        </span>
        <div className="flex items-center gap-2">
          <PathInput value={buildPath} onChange={setBuildPath} placeholder="/path/to/scan" />
          <button
            type="button"
            className="icon-btn"
            style={{ width: "auto", padding: "0 12px", fontSize: 11 }}
            onClick={() => void onBuild()}
            disabled={building}
          >
            {building ? "Building..." : "Build evidence"}
          </button>
        </div>

        {buildError && (
          <span className="mono text-[11.5px]" style={{ color: "var(--sev-high)" }}>
            {describeCryptoError(buildError)}
          </span>
        )}

        {report && (
          <div className="flex flex-col gap-2">
            <span className="text-[11px]" style={{ color: "var(--faint)" }}>
              as of build{builtAtMs !== null ? ` · ${new Date(builtAtMs).toLocaleTimeString()}` : ""}
            </span>
            <div className="grid gap-3" style={{ gridTemplateColumns: "repeat(4, minmax(0,1fr))" }}>
              <StatTile label="Score" value={`${report.summary.scorePct}%`} />
              <StatTile label="Compliant" value={String(report.summary.compliant)} />
              <StatTile label="Non-compliant" value={String(report.summary.nonCompliant)} />
              <StatTile label="Issues" value={String(report.summary.issues)} />
            </div>
            <span className="mono text-[11px] truncate" style={{ color: "var(--dim)" }} title={report.digest}>
              digest {report.digest}
            </span>
            <span className="mono text-[11px]" style={{ color: "var(--dim)" }}>
              signature {report.signature ? report.signature.alg : "none (unsigned - W1 always builds unsigned bundles)"}
            </span>
          </div>
        )}
      </div>

      <div className="panel px-4 py-3 flex flex-col gap-2.5" style={{ background: "var(--panel-2)" }}>
        <span className="text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.6 }}>
          Verify a saved evidence JSON file&apos;s digest and signature - a file already on disk (e.g. from a previous
          qryx run), not necessarily the bundle built above.
        </span>
        <div className="flex items-center gap-2">
          <PathInput value={verifyPath} onChange={setVerifyPath} placeholder="/path/to/evidence.json" />
          <button
            type="button"
            className="icon-btn"
            style={{ width: "auto", padding: "0 12px", fontSize: 11 }}
            onClick={() => void onVerify()}
            disabled={verifying}
          >
            {verifying ? "Verifying..." : "Verify"}
          </button>
        </div>
        {verifyError && (
          <span className="mono text-[11.5px]" style={{ color: "var(--sev-high)" }}>
            {describeCryptoError(verifyError)}
          </span>
        )}
        {outcome && (
          <span className="mono text-[11.5px]" style={{ color: outcome.verified ? "var(--sev-low)" : "var(--sev-high)" }}>
            {outcome.verified ? "VERIFIED" : "NOT VERIFIED"} - {outcome.message}
          </span>
        )}
      </div>
    </div>
  );
}
