import { cssVar } from "../lib/cssVars";
import { formatTimestamp } from "../lib/format";
import type { EvidenceBuildResult } from "../evidenceTypes";
import { EvidenceArtifactsTable } from "./EvidenceArtifactsTable";

function SectionHeader({ title }: { title: string }) {
  return (
    <span className="mono" style={{ fontSize: 11, letterSpacing: "0.1em", textTransform: "uppercase", color: "var(--faint)" }}>
      {title}
    </span>
  );
}

/**
 * The built pack's contents view (docs/PHASE4.md W3): the header
 * (pack_version, generated_at, operator, org, a SIGNED/UNSIGNED badge), the
 * artifact table, and a clearly-separate "Not included" list built from
 * `manifest.missing` - so the pack's honesty is visible at a glance, never
 * just "here is a zip, trust it". `signed` is read verbatim off the result
 * (never inferred/assumed): an unsigned pack is always labeled UNSIGNED, in
 * the same tone as a missing source, not a quiet gray.
 */
export function EvidenceManifestView({ result }: { result: EvidenceBuildResult }) {
  const { manifest, signed, cloud_included, journaled, journal_error } = result;

  return (
    <div className="flex flex-col gap-4">
      <div className="panel px-4 py-3 flex flex-col gap-2.5" style={{ background: "var(--panel-2)" }}>
        <div className="flex items-center gap-2 flex-wrap">
          <span className="badge" style={cssVar("tone", signed ? "var(--sev-low)" : "var(--sev-medium)")}>
            {signed ? "SIGNED" : "UNSIGNED"}
          </span>
          <span className="mono text-[11px]" style={{ color: "var(--faint)" }}>
            {manifest.pack_version}
          </span>
          <span className="mono text-[11px]" style={{ color: "var(--faint)" }}>
            as of {formatTimestamp(manifest.generated_at)}
          </span>
        </div>
        <div className="flex items-center gap-4 flex-wrap text-[11.5px]" style={{ color: "var(--dim)" }}>
          <span>
            operator <span className="mono" style={{ color: "var(--fg)" }}>{manifest.operator}</span>
          </span>
          <span>
            org <span className="mono" style={{ color: "var(--fg)" }}>{manifest.org}</span>
          </span>
        </div>
        <span className="text-[11px]" style={{ color: "var(--faint)" }}>
          {journaled ? "journaled as console_evidence_built" : `not journaled${journal_error ? ` (${journal_error})` : ""}`}
          {!cloud_included && " · Cloud sources not included in this pack"}
        </span>
      </div>

      <section className="flex flex-col gap-2">
        <SectionHeader title={`Artifacts (${manifest.artifacts.length})`} />
        <EvidenceArtifactsTable artifacts={manifest.artifacts} />
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title={`Not included (${manifest.missing.length})`} />
        {manifest.missing.length === 0 ? (
          <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
            every requested source was included.
          </div>
        ) : (
          <div className="flex flex-col gap-1.5">
            {manifest.missing.map((m) => (
              <div
                key={m.name}
                className="panel px-3 py-2 flex items-baseline gap-2 flex-wrap"
                style={{ background: "var(--panel-2)", borderLeft: "3px solid var(--sev-medium)" }}
              >
                <span className="text-[11.5px]" style={{ color: "var(--fg)" }}>
                  {m.name}
                </span>
                <span className="mono text-[11px]" style={{ color: "var(--dim)" }}>
                  {m.reason}
                </span>
              </div>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
