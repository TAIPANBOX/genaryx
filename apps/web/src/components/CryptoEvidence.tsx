import { useCallback, useState } from "react";
import { describeCryptoError, scanEvidence, verifyEvidence } from "../lib/crypto";
import {
  EVIDENCE_ASSET_EXPORT_COLUMNS,
  evidenceAssetExportMeta,
  evidenceAssetExportRows,
  evidenceAssets,
  evidenceAssetsNote,
  evidenceProvenance,
  evidenceSeverityNote,
  evidenceSeverityRows,
  evidenceUnaccounted,
} from "../lib/cryptoExport";
import { ExportBar } from "../lib/cryptoExportBar";
import { downloadCsv, downloadJson } from "../lib/download";
import type { CryptoError, EvidenceReport, VerifyOutcome } from "../cryptoTypes";
import { SeverityBadge } from "./SeverityBadge";
import { StatTile } from "./StatTile";

const ASSET_COLUMNS = "150px 110px 120px 90px 70px 1fr";

/** A one-line note the operator should read rather than skim past: a missing
 * list, a count that does not reconcile, a breakdown the report did not
 * carry. Toned as a caution because every one of them means the panel is
 * showing less than the question asked for. */
function HonestNote({ children }: { children: React.ReactNode }) {
  return (
    <span className="text-[11.5px]" style={{ color: "var(--sev-medium)", lineHeight: 1.6, maxWidth: 760 }}>
      {children}
    </span>
  );
}

/** Where the bundle came from: which build of qryx made it, what it graded
 * against, when, and over which root. An attestation that does not say who
 * signed off and when is a screenshot. */
function EvidenceProvenance({ report }: { report: EvidenceReport }) {
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1">
      {evidenceProvenance(report).map((l) => (
        <span key={l.label} className="mono text-[11px] min-w-0" style={{ color: "var(--faint)" }}>
          {l.label}{" "}
          <span className="truncate" style={{ color: l.missing ? "var(--sev-medium)" : "var(--dim)" }} title={l.value}>
            {l.value}
          </span>
        </span>
      ))}
    </div>
  );
}

/** `summary.bySeverity`, which reached no component. It is the triage order
 * for the non-compliant assets: without it, "29 non-compliant" says nothing
 * about whether tomorrow is soon enough. */
function SeverityBreakdown({ report }: { report: EvidenceReport }) {
  const rows = evidenceSeverityRows(report);
  const note = evidenceSeverityNote(report);
  if (rows.length === 0) {
    return note !== null ? <HonestNote>{note}</HonestNote> : null;
  }
  return (
    <div className="flex flex-wrap items-center gap-2">
      <span className="text-[11px]" style={{ color: "var(--faint)" }}>
        by severity
      </span>
      {rows.map((r) => (
        <span key={r.severity} className="inline-flex items-center gap-1.5">
          <SeverityBadge severity={r.severity} />
          <span className="mono tabular text-[12px]" style={{ color: "var(--fg)" }}>
            {r.count.toLocaleString("en-US")}
          </span>
        </span>
      ))}
    </div>
  );
}

/**
 * The per-asset CNSA rows. The summary says how many assets are
 * non-compliant; these say which ones, by when qryx wants them migrated, and
 * what it says to do about each. They cross the backend as raw JSON, so every
 * cell is read tolerantly and an absent one is left blank rather than filled
 * with a dash that reads like a value.
 */
function EvidenceAssetsTable({ report }: { report: EvidenceReport }) {
  const assets = evidenceAssets(report);
  if (assets.length === 0) {
    const note = evidenceAssetsNote(report);
    return note !== null ? <HonestNote>{note}</HonestNote> : null;
  }
  return (
    <div style={{ overflowX: "auto" }}>
      <div
        className="grid gap-3 py-2"
        style={{ gridTemplateColumns: ASSET_COLUMNS, borderBottom: "1px solid var(--line)" }}
      >
        {["algorithm", "type", "cnsa status", "deadline", "count", "action"].map((label) => (
          <span
            key={label}
            className="mono"
            style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
          >
            {label}
          </span>
        ))}
      </div>
      {assets.map((a, idx) => (
        <div
          key={`${a.algorithm ?? "asset"}-${idx}`}
          className="grid items-center gap-3 py-2 bus-row"
          style={{ gridTemplateColumns: ASSET_COLUMNS }}
        >
          <span className="mono truncate text-[12px]" style={{ color: "var(--fg)" }} title={a.algorithm}>
            {a.algorithm ?? ""}
          </span>
          <span className="mono truncate text-[11.5px]" style={{ color: "var(--dim)" }}>
            {a.type ?? ""}
          </span>
          <span className="mono truncate text-[11.5px]" style={{ color: "var(--dim)" }}>
            {a.status ?? ""}
          </span>
          <span className="mono truncate text-[11.5px]" style={{ color: "var(--dim)" }}>
            {a.deadline ?? ""}
          </span>
          <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }}>
            {typeof a.occurrences === "number" ? a.occurrences : ""}
          </span>
          <span
            className="truncate text-[11.5px]"
            style={{ color: "var(--faint)" }}
            title={[a.action, (a.locations ?? []).join(", ")].filter((x) => x).join(" | ")}
          >
            {a.action ?? ""}
          </span>
        </div>
      ))}
    </div>
  );
}

/** Genaryx v2 design spec section 7 parity fix #5: the Evidence
 * section has a `repository` / `agent stack` scope toggle (`CryptoModel.swift`'s
 * `EvidenceScope`, backed by two distinct `CryptoHandle` methods -
 * `scan_evidence` vs `agents_evidence`). The `crypto_scan_evidence`
 * command (`crates/api/src/crypto/commands.rs`) takes only `{ path, sign_key }`
 * - no scope argument, and there is no second `crypto_agents_evidence`
 * command either - so this toggle is UI-only for now: visible for parity,
 * disabled with an honest tooltip, wired to nothing. Flip `disabled` off
 * once the backend grows the argument (or the second command) and thread the
 * chosen scope into `scanEvidence`. */
const SCOPE_DISABLED_TITLE = "needs crypto_scan_evidence scope arg (backend)";

function EvidenceScopeToggle() {
  return (
    <div className="flex items-center gap-2">
      <span className="text-[11.5px] shrink-0" style={{ color: "var(--dim)" }}>
        scope
      </span>
      <div className="flex items-center gap-1.5" role="group" aria-label="Evidence scope (not yet wired to the backend)">
        <button type="button" className="chip" disabled title={SCOPE_DISABLED_TITLE} style={{ opacity: 1 }} aria-pressed="true">
          repository
        </button>
        <button type="button" className="chip" disabled title={SCOPE_DISABLED_TITLE} style={{ opacity: 0.45 }} aria-pressed="false">
          agent stack
        </button>
      </div>
    </div>
  );
}

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
export function CryptoEvidence({ defaultPath, environment }: { defaultPath: string; environment: string }) {
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

  const assetRows = report !== null ? evidenceAssetExportRows(report) : [];
  const unaccounted = report !== null ? evidenceUnaccounted(report) : null;

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
      <div className="d-card px-4 py-3 flex flex-col gap-2.5">
        <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
          Build an unsigned CNSA evidence bundle for a target path.
        </span>
        <EvidenceScopeToggle />
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
          <div className="flex flex-col gap-3">
            <span className="text-[11px]" style={{ color: "var(--faint)" }}>
              as of build{builtAtMs !== null ? ` · ${new Date(builtAtMs).toLocaleTimeString()}` : ""}
            </span>
            <EvidenceProvenance report={report} />
            <div className="grid gap-3" style={{ gridTemplateColumns: "repeat(4, minmax(0,1fr))" }}>
              {/* `total` is the score's denominator and it never reached the
                  screen: 77% of 127 assets and 77% of 12 are the same number
                  about very different estates. */}
              <StatTile
                label="Score"
                value={`${report.summary.scorePct}%`}
                sub={`of ${report.summary.total.toLocaleString("en-US")} graded`}
              />
              <StatTile label="Compliant" value={report.summary.compliant.toLocaleString("en-US")} />
              <StatTile label="Non-compliant" value={report.summary.nonCompliant.toLocaleString("en-US")} />
              <StatTile label="Issues" value={report.summary.issues.toLocaleString("en-US")} />
            </div>
            {unaccounted !== null && <HonestNote>{unaccounted}</HonestNote>}
            <SeverityBreakdown report={report} />

            <div className="flex items-center justify-between gap-2 pt-1" style={{ borderTop: "1px solid var(--line)" }}>
              <span className="mono pt-2" style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}>
                graded assets
              </span>
              <span className="pt-2">
                <ExportBar
                  label="the graded assets in this bundle"
                  disabledHint="this bundle carried no per-asset rows"
                  disabled={assetRows.length === 0}
                  onCsv={() =>
                    downloadCsv(
                      "genaryx-evidence-assets.csv",
                      EVIDENCE_ASSET_EXPORT_COLUMNS,
                      assetRows,
                      evidenceAssetExportMeta(report, new Date().toISOString(), environment),
                    )
                  }
                  onJson={() =>
                    downloadJson(
                      "genaryx-evidence-assets.json",
                      assetRows,
                      evidenceAssetExportMeta(report, new Date().toISOString(), environment),
                    )
                  }
                />
              </span>
            </div>
            <EvidenceAssetsTable report={report} />

            <span className="mono text-[11px] truncate" style={{ color: "var(--dim)" }} title={report.digest}>
              digest {report.digest}
            </span>
            {report.signature ? (
              // The alg alone does not answer the question a signature is
              // asked. "Signed with ml-dsa-65" is not actionable; "signed
              // with THIS key" is, because the operator can tell whether it
              // is one they recognise. The signature `value` itself stays
              // out: nobody checks base64 by eye, and the check it exists
              // for is the Verify form below.
              <span className="mono text-[11px] truncate" style={{ color: "var(--dim)" }} title={report.signature.publicKey}>
                signature {report.signature.alg} · public key {report.signature.publicKey}
              </span>
            ) : (
              <span className="mono text-[11px]" style={{ color: "var(--dim)" }}>
                signature none - this console asks qryx for an unsigned bundle, so this is its own request rather than
                something qryx could not do
              </span>
            )}
          </div>
        )}
      </div>

      <div className="d-card px-4 py-3 flex flex-col gap-2.5">
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
