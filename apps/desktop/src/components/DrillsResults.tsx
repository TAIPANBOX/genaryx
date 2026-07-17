import { cssVar } from "../lib/cssVars";
import { formatUsd } from "../lib/format";
import { hasGaps } from "../drillsTypes";
import type { MockryxFinding, MockryxReport, MockryxResult } from "../drillsTypes";

const STATUS_TONE: Record<string, string> = {
  passed: "var(--sev-low)",
  failed: "var(--sev-high)",
  skipped_not_configured: "var(--sev-medium)",
};

function statusLabel(status: string): string {
  switch (status) {
    case "passed":
      return "held";
    case "failed":
      return "GAP";
    case "skipped_not_configured":
      return "skipped";
    default:
      return status;
  }
}

function FindingRow({ finding, gap }: { finding: MockryxFinding; gap: boolean }) {
  return (
    <div
      className="px-3 py-2.5 flex flex-col gap-1.5"
      style={{
        background: "var(--panel)",
        borderRadius: 8,
        borderLeft: `3px solid ${gap ? "var(--sev-high)" : "var(--sev-medium)"}`,
      }}
    >
      <div className="flex items-center gap-2 flex-wrap">
        <span className="mono text-[11.5px]" style={{ color: "var(--fg)" }}>
          {finding.step}
        </span>
        <span className="text-[10px]" style={{ color: "var(--faint)" }}>
          attempt {finding.attempt}
        </span>
        <div className="flex-1" />
        <span className="mono tabular text-[11px]" style={{ color: "var(--dim)" }}>
          expected {finding.expect_status} &middot; got {finding.got_status}
        </span>
      </div>
      {(finding.expect_header ?? finding.got_headers) && (
        <div className="mono text-[10.5px]" style={{ color: "var(--faint)" }}>
          {finding.expect_header && <span>expect headers {JSON.stringify(finding.expect_header)}</span>}
          {finding.expect_header && finding.got_headers && " · "}
          {finding.got_headers && <span>got headers {JSON.stringify(finding.got_headers)}</span>}
        </div>
      )}
      <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
        {finding.detail}
      </span>
      {(finding.expect_event_source ?? finding.expect_event_type) && (
        <span className="mono text-[10.5px]" style={{ color: "var(--faint)" }}>
          expected event {finding.expect_event_source ?? "?"}.{finding.expect_event_type ?? "?"}
        </span>
      )}
    </div>
  );
}

function ScenarioCard({ result }: { result: MockryxResult }) {
  return (
    <div className="panel px-4 py-3 flex flex-col gap-2.5" style={{ background: "var(--panel-2)" }}>
      <div className="flex items-center justify-between gap-2">
        <span className="mono text-[12px]" style={{ color: "var(--fg)", fontWeight: 650 }}>
          {result.scenario}
        </span>
        <span className="badge" style={cssVar("tone", STATUS_TONE[result.status] ?? "var(--faint)")}>
          {statusLabel(result.status)}
        </span>
      </div>
      <div className="flex items-center gap-4">
        <span className="text-[11px]" style={{ color: "var(--faint)" }}>
          calls{" "}
          <span className="mono tabular" style={{ color: "var(--fg)" }}>
            {result.metrics.calls}
          </span>
        </span>
        <span className="text-[11px]" style={{ color: "var(--faint)" }}>
          budget burned{" "}
          <span className="mono tabular" style={{ color: "var(--fg)" }}>
            {formatUsd(result.metrics.budget_burned_usd)}
          </span>
        </span>
      </div>

      {result.findings.length > 0 && (
        <div className="flex flex-col gap-1.5">
          <span
            className="mono"
            style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--sev-high)" }}
          >
            action items ({result.findings.length})
          </span>
          {result.findings.map((f, idx) => (
            <FindingRow key={`${f.step}-${idx}`} finding={f} gap />
          ))}
        </div>
      )}

      {result.skipped_findings.length > 0 && (
        <div className="flex flex-col gap-1.5">
          <span
            className="mono"
            style={{ fontSize: 10, letterSpacing: "0.08em", textTransform: "uppercase", color: "var(--faint)" }}
          >
            guardrail not observed active ({result.skipped_findings.length})
          </span>
          {result.skipped_findings.map((f, idx) => (
            <FindingRow key={`${f.step}-${idx}`} finding={f} gap={false} />
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * Drill results (docs/PHASE4.md W2 Drills positions 2-3): an overall
 * verdict banner from [`hasGaps`] (guardrails held vs gaps found), then
 * per-scenario cards - status (`passed` = held / `failed` = GAP /
 * `skipped_not_configured` = skip), metrics, and findings surfaced as clear
 * action items (expected vs got status/headers + detail, the exact gap the
 * operator must fix). `skipped_findings` render separately and
 * informationally ("guardrail not observed active"), never counted toward
 * the gap tally on their own - only `findings` (which, after
 * `--fail-on-skip`, can include promoted skips) does.
 */
export function DrillsResults({ report }: { report: MockryxReport }) {
  const gaps = hasGaps(report);
  return (
    <div className="flex flex-col gap-3">
      <div className="panel px-4 py-3 flex items-center gap-3" style={{ background: "var(--panel-2)" }}>
        <span className="badge" style={cssVar("tone", gaps ? "var(--sev-high)" : "var(--sev-low)")}>
          {gaps ? "GAPS FOUND" : "guardrails held"}
        </span>
        <span className="mono text-[11px]" style={{ color: "var(--faint)" }}>
          run {report.run_id} &middot; {report.gateway} &middot; {report.results.length} scenarios
        </span>
      </div>
      {report.results.length === 0 ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          no scenarios in this report.
        </div>
      ) : (
        <div className="grid gap-3" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(340px, 1fr))" }}>
          {report.results.map((r) => (
            <ScenarioCard key={r.scenario} result={r} />
          ))}
        </div>
      )}
    </div>
  );
}
