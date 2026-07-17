import { formatBytes } from "../lib/format";
import type { ManifestArtifact } from "../evidenceTypes";

const COLUMNS = "1fr 1.4fr 130px 90px 1.3fr";

/** `"sha256:<64 hex chars>"` -> a short, still-copyable-by-eye prefix/suffix
 * pair for the table cell; the full hash is always still available via the
 * cell's `title` tooltip - never lossy, just compact. */
function shortSha256(sha256: string): string {
  const hex = sha256.startsWith("sha256:") ? sha256.slice("sha256:".length) : sha256;
  if (hex.length <= 16) return hex;
  return `${hex.slice(0, 8)}…${hex.slice(-6)}`;
}

/** A verify-status string's tone, keyed on qryx's/the audit chain's own
 * wording (`"VERIFIED"`/`"self-verifying"` vs `"BROKEN"`/`"NOT"`) - free text
 * from the tool itself, not a closed enum, so this is a best-effort read,
 * never a claim the console independently re-verified anything. */
function verifyTone(status: string): string {
  const lower = status.toLowerCase();
  if (lower.includes("broken") || lower.includes("not verified")) return "var(--sev-high)";
  if (lower.includes("verified") || lower.includes("self-verifying")) return "var(--sev-low)";
  return "var(--faint)";
}

/**
 * The evidence pack's artifact table (docs/PHASE4.md W3): name, source,
 * short sha256, size, and the artifact's OWN verify status where present
 * (Qryx's self-verifying digest/signature, the Cloud audit-chain verdict) -
 * `null` renders as a plain dash, never a fabricated "unverified" claim (an
 * artifact with no self-verification story, like the FOCUS CSV or the
 * Agent-BOM, simply has none).
 */
export function EvidenceArtifactsTable({ artifacts }: { artifacts: ManifestArtifact[] }) {
  if (artifacts.length === 0) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        no artifacts in this pack.
      </div>
    );
  }

  return (
    <div className="panel" style={{ background: "var(--panel)", overflow: "hidden" }}>
      <div
        className="grid gap-3 px-4 py-2"
        style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line-2)", background: "var(--panel-2)" }}
      >
        {["name", "source", "sha256", "size", "verify status"].map((label) => (
          <span
            key={label}
            className="mono"
            style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
          >
            {label}
          </span>
        ))}
      </div>
      {artifacts.map((a) => (
        <div key={a.filename} className="grid items-center gap-3 px-4 py-2.5 bus-row" style={{ gridTemplateColumns: COLUMNS }}>
          <div className="flex flex-col gap-0.5 min-w-0">
            <span className="truncate text-[12px]" title={a.name} style={{ color: "var(--fg)" }}>
              {a.name}
            </span>
            <span className="mono truncate text-[10.5px]" style={{ color: "var(--faint)" }} title={a.filename}>
              {a.filename} &middot; {a.content_type}
            </span>
          </div>
          <span className="mono truncate text-[11px]" style={{ color: "var(--dim)" }} title={a.source}>
            {a.source}
          </span>
          <span className="mono tabular text-[11px]" style={{ color: "var(--dim)" }} title={a.sha256}>
            {shortSha256(a.sha256)}
          </span>
          <span className="mono tabular text-[11.5px]" style={{ color: "var(--fg)" }}>
            {formatBytes(a.size_bytes)}
          </span>
          {a.verify_status ? (
            <span className="text-[11px] truncate" style={{ color: verifyTone(a.verify_status) }} title={a.verify_status}>
              {a.verify_status}
            </span>
          ) : (
            <span className="text-[11px]" style={{ color: "var(--faint)" }}>
              -
            </span>
          )}
        </div>
      ))}
    </div>
  );
}
