import { cssVar } from "../lib/cssVars";
import type { NcscFinding } from "../cryptoTypes";
import { SeverityBadge } from "./SeverityBadge";

const COLUMNS = "140px 120px 90px 70px 1fr 190px";

/**
 * Quantum-vulnerable findings (docs/PHASE4.md W1 position 2): the 2028
 * discovery milestone's finding list - algorithm, asset type, severity,
 * occurrence count, locations, and the externally-facing/long-lived/planned
 * flags. `findings === null` means "no scan has run yet" (distinct from an
 * empty array, which means "scanned, found nothing").
 */
export function CryptoFindingsTable({ findings }: { findings: NcscFinding[] | null }) {
  if (findings === null) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        scan a target to see quantum-vulnerable findings.
      </div>
    );
  }
  if (findings.length === 0) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        no quantum-vulnerable findings in the last scan.
      </div>
    );
  }

  return (
    <div style={{ overflowX: "auto" }}>
      <div
        className="grid gap-3 px-5 py-2"
        style={{ gridTemplateColumns: COLUMNS, borderBottom: "1px solid var(--line)" }}
      >
        {["algorithm", "type", "severity", "count", "locations", "flags"].map((label) => (
          <span
            key={label}
            className="mono"
            style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
          >
            {label}
          </span>
        ))}
      </div>
      {findings.map((f, idx) => (
        <div key={`${f.algorithm}-${idx}`} className="grid items-center gap-3 px-5 py-2 bus-row" style={{ gridTemplateColumns: COLUMNS }}>
          <span className="mono truncate text-[12px]" style={{ color: "var(--fg)" }} title={f.algorithm}>
            {f.algorithm}
          </span>
          <span className="mono truncate text-[11.5px]" style={{ color: "var(--dim)" }}>
            {f.type}
          </span>
          <SeverityBadge severity={f.severity} />
          <span className="mono tabular text-[12px]" style={{ color: "var(--dim)" }}>
            {f.occurrences}
          </span>
          <span
            className="mono truncate text-[11px]"
            style={{ color: "var(--faint)" }}
            title={f.locations.length > 0 ? f.locations.join(", ") : undefined}
          >
            {f.locations.length > 0 ? f.locations.join(", ") : "-"}
          </span>
          <span className="flex items-center gap-1.5 flex-wrap">
            {f.externallyFacing && (
              <span className="badge" style={cssVar("tone", "var(--sev-high)")}>
                external
              </span>
            )}
            {f.longLivedData && (
              <span className="badge" style={cssVar("tone", "var(--sev-medium)")}>
                long-lived
              </span>
            )}
            {f.planned && (
              <span className="badge" style={cssVar("tone", "var(--sev-low)")}>
                planned
              </span>
            )}
            {!f.externallyFacing && !f.longLivedData && !f.planned && (
              <span className="text-[11px]" style={{ color: "var(--faint)" }}>
                -
              </span>
            )}
          </span>
        </div>
      ))}
    </div>
  );
}
