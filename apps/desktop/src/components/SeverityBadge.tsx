import { cssVar } from "../lib/cssVars";
import { SEVERITIES, type Severity } from "../types";

const LABEL: Record<Severity, string> = {
  info: "Info",
  low: "Low",
  medium: "Medium",
  high: "High",
  critical: "Critical",
};

const TONE_VAR: Record<Severity, string> = {
  info: "var(--sev-info)",
  low: "var(--sev-low)",
  medium: "var(--sev-medium)",
  high: "var(--sev-high)",
  critical: "var(--sev-critical)",
};

function isKnownSeverity(value: string): value is Severity {
  return (SEVERITIES as readonly string[]).includes(value);
}

/**
 * Severity pill, one color per rung of the ladder. `severity` is a raw
 * string end to end (the core never closes this enum, see `types.ts`), so
 * an unrecognized value still renders, just in a neutral tone with its own
 * text rather than falling over.
 */
export function SeverityBadge({ severity }: { severity: string | null }) {
  if (!severity) {
    return (
      <span className="badge" style={cssVar("tone", "var(--faint)")}>
        n/a
      </span>
    );
  }
  if (isKnownSeverity(severity)) {
    return (
      <span className="badge" style={cssVar("tone", TONE_VAR[severity])}>
        {LABEL[severity]}
      </span>
    );
  }
  return (
    <span className="badge" style={cssVar("tone", "var(--faint)")}>
      {severity}
    </span>
  );
}
