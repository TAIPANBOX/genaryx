import { useEffect, useState } from "react";
import { fetchRecord, verifyRecord, type RecordRead } from "../lib/trailRecord";
import { useConsoleStateVersion } from "../lib/consoleState";
import { formatTimestamp } from "../lib/format";
import { visibleText } from "../lib/policyExport";
import type { SealState, Signature, VerifyState } from "../recordTypes";

/**
 * Record: what Trailryx sealed, and whether the current root carries a Keyryx
 * signature (CLAUDE.md invariant 54).
 *
 * Read from the node's own files and decoder on this box. The one control is
 * Verify, which packs the record and runs trailryx-verify on it; nothing here
 * writes to the record. Every string from the box is the node's, so each goes
 * through `visibleText` and renders as a text node.
 */
export function RecordView() {
  const [read, setRead] = useState<RecordRead | null | undefined>(undefined);
  const [verifying, setVerifying] = useState(false);
  const [verifyError, setVerifyError] = useState<string | null>(null);
  const version = useConsoleStateVersion();

  useEffect(() => {
    let alive = true;
    const load = async () => {
      const r = await fetchRecord();
      if (alive) setRead(r);
    };
    void load();
    const t = setInterval(() => void load(), 30_000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [version]);

  const onVerify = async () => {
    if (verifying) return;
    setVerifying(true);
    try {
      const r = await verifyRecord();
      if (r === null) return;
      if (r.ok) {
        setRead(r);
        setVerifyError(null);
      } else {
        setVerifyError(r.reason);
      }
    } finally {
      setVerifying(false);
    }
  };

  return (
    <div className="flex flex-col min-h-0 h-full overflow-auto" style={{ padding: "18px 22px", gap: 14 }}>
      <div>
        <div className="text-[11px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
          Assure
        </div>
        <h1 className="text-[20px]" style={{ color: "var(--fg)", fontWeight: 600, margin: "2px 0 4px" }}>
          Record
        </h1>
        <p className="text-[12px]" style={{ color: "var(--dim)", margin: 0, maxWidth: 760 }}>
          What Trailryx has sealed, read from the node's own files with its own decoder, and whether the
          current root carries a Keyryx signature. Verify packs the record and runs trailryx-verify on it;
          nothing here writes to the record.
        </p>
      </div>
      <Body read={read} verifying={verifying} verifyError={verifyError} onVerify={onVerify} />
    </div>
  );
}

function Note({ tone, children, testId }: { tone: "warn" | "calm" | "bad"; children: React.ReactNode; testId?: string }) {
  const edge = tone === "bad" ? "var(--sev-high)" : tone === "warn" ? "var(--amber)" : "var(--line-2)";
  return (
    <div
      data-testid={testId}
      className="text-[12px]"
      style={{
        padding: "8px 12px",
        borderLeft: `2px solid ${edge}`,
        background: tone === "calm" ? "transparent" : "var(--panel-2)",
        color: tone === "calm" ? "var(--dim)" : "var(--fg)",
        lineHeight: 1.6,
      }}
    >
      {children}
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="flex flex-col" style={{ gap: 8 }}>
      <h2 className="text-[13px]" style={{ color: "var(--fg)", fontWeight: 600, margin: 0 }}>
        {title}
      </h2>
      {children}
    </section>
  );
}

/** A unix-seconds time, written the way the rest of the console writes one. */
function when(seconds: number): string {
  return formatTimestamp(new Date(seconds * 1000).toISOString());
}

function Body({
  read,
  verifying,
  verifyError,
  onVerify,
}: {
  read: RecordRead | null | undefined;
  verifying: boolean;
  verifyError: string | null;
  onVerify: () => void;
}) {
  if (read === undefined) return <Note tone="calm">reading the record...</Note>;
  if (read === null) return <Note tone="calm">No box to ask: this build has no backend.</Note>;
  if (!read.ok) {
    return (
      <Note tone="warn" testId="record-error">
        The box did not answer for the record: {visibleText(read.reason)}
      </Note>
    );
  }
  const { seal, verify, verify_stale } = read.panel;
  return (
    <>
      <Section title="Last seal">
        <SealSection seal={seal} />
      </Section>
      <Section title="Verify">
        {seal.state === "sealed" && (
          <div>
            <button
              type="button"
              data-testid="record-verify-button"
              disabled={verifying}
              onClick={onVerify}
              className="text-[12px]"
              style={{
                padding: "6px 14px",
                border: "1px solid var(--line-2)",
                background: "var(--panel-2)",
                color: "var(--fg)",
                cursor: verifying ? "default" : "pointer",
                opacity: verifying ? 0.6 : 1,
              }}
            >
              {verifying ? "Verifying..." : "Verify now"}
            </button>
          </div>
        )}
        {verifyError !== null && (
          <Note tone="warn" testId="record-verify-error">
            The verification could not be started: {visibleText(verifyError)}
          </Note>
        )}
        <VerifyLine verify={verify} />
        {verify_stale && (verify.state === "verified" || verify.state === "not_verified") && (
          <Note tone="calm" testId="record-verify-stale">
            That result was for an earlier root; the record has been sealed further since.
          </Note>
        )}
      </Section>
    </>
  );
}

function SealSection({ seal }: { seal: SealState }) {
  switch (seal.state) {
    case "not_configured":
      return (
        <Note tone="warn" testId="record-not-configured">
          No Trailryx data directory is named on this box, so the console cannot say what is sealed. Mount
          the node's data directory read-only and set <span className="mono">{visibleText(seal.variable)}</span>{" "}
          to it.
        </Note>
      );
    case "refused":
      return (
        <Note tone="warn" testId="record-refused">
          <span className="mono">{visibleText(seal.variable)}</span> is set but refused: {visibleText(seal.reason)}
        </Note>
      );
    case "unreadable":
      return (
        <Note tone="warn" testId="record-unreadable">
          The store at <span className="mono">{visibleText(seal.path)}</span> could not be read:{" "}
          {visibleText(seal.reason)}
        </Note>
      );
    case "nothing_sealed":
      return (
        <Note tone="calm" testId="record-nothing-sealed">
          Nothing is sealed yet. <span className="mono">{visibleText(seal.path)}</span> holds no manifest.
        </Note>
      );
    case "sealed": {
      const last = seal.last;
      return (
        <div className="flex flex-col" style={{ gap: 6 }}>
          <div className="mono text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.8 }} data-testid="record-sealed">
            {seal.segments} segment(s), {seal.records} record(s); last segment {last.segment}, {last.records}{" "}
            record(s), {when(last.first_recorded_at)}..{when(last.last_recorded_at)}
          </div>
          <div className="mono text-[11.5px]" style={{ color: "var(--fg)", overflowWrap: "anywhere" }} data-testid="record-root">
            {visibleText(seal.root)}
          </div>
          <SignatureLine signature={seal.signature} />
          {seal.other_shards > 0 && (
            <Note tone="calm" testId="record-other-shards">
              {seal.other_shards} other shard manifest(s) seen and not read.
            </Note>
          )}
        </div>
      );
    }
  }
}

function SignatureLine({ signature }: { signature: Signature }) {
  switch (signature.state) {
    case "signed":
      return (
        <Note tone="calm" testId="record-signature">
          Keyryx signature on file: {visibleText(signature.algorithm)}, key{" "}
          <span className="mono">{visibleText(signature.key)}</span>, at {when(signature.at)}. The node checked it
          before storing it; Verify proves it.
        </Note>
      );
    case "queued":
      return (
        <Note tone="calm" testId="record-signature">
          Waiting for a Keyryx signature since {when(signature.since)}.
        </Note>
      );
    case "none":
      return (
        <Note tone="calm" testId="record-signature">
          No signature on file for this root.
        </Note>
      );
    case "unreadable":
      return (
        <Note tone="warn" testId="record-signature">
          The signing file could not be read: {visibleText(signature.reason)}
        </Note>
      );
  }
}

function VerifyLine({ verify }: { verify: VerifyState }) {
  switch (verify.state) {
    case "not_run":
      return (
        <Note tone="calm" testId="record-verify-state">
          Not verified since this console started.
        </Note>
      );
    case "verified":
      return (
        <div className="flex flex-col" style={{ gap: 6 }}>
          <Note tone="calm" testId="record-verify-state">
            <span style={{ color: "var(--mint)", fontWeight: 600 }}>Verified</span> at {when(verify.at)} in{" "}
            {verify.millis} ms.
          </Note>
          <Findings findings={verify.findings} />
        </div>
      );
    case "not_verified":
      return (
        <div className="flex flex-col" style={{ gap: 6 }}>
          <Note tone="bad" testId="record-verify-state">
            NOT verified at {when(verify.at)}.
          </Note>
          <Findings findings={verify.findings} />
        </div>
      );
    case "not_checked":
      return (
        <Note tone="warn" testId="record-verify-state">
          Could not be checked at {when(verify.at)}: {visibleText(verify.reason)}. This is not a verdict.
        </Note>
      );
  }
}

function Findings({ findings }: { findings: string[] }) {
  if (findings.length === 0) return null;
  return (
    <ul data-testid="record-findings" className="mono text-[11.5px]" style={{ color: "var(--dim)", margin: 0, paddingLeft: 18, lineHeight: 1.7 }}>
      {findings.map((f, i) => (
        <li key={i} style={{ overflowWrap: "anywhere" }}>
          {visibleText(f)}
        </li>
      ))}
    </ul>
  );
}
