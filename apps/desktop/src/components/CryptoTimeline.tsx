import { cssVar } from "../lib/cssVars";
import type { CryptoError, NcscReport } from "../cryptoTypes";
import { describeCryptoError } from "../lib/crypto";

const VERDICT_TONE: Record<string, string> = {
  "on-track": "var(--sev-low)",
  "at-risk": "var(--sev-medium)",
  "not-started": "var(--sev-high)",
};

function verdictTone(verdict: string): string {
  return VERDICT_TONE[verdict] ?? "var(--faint)";
}

function MilestoneCard({
  title,
  verdict,
  stats,
  note,
}: {
  title: string;
  verdict: string;
  stats: { label: string; value: string }[];
  note?: string;
}) {
  return (
    <div className="d-card px-4 py-3.5 flex flex-col gap-2.5">
      <div className="flex items-center justify-between gap-2">
        <span className="mono" style={{ fontSize: 11.5, color: "var(--fg)", fontWeight: 650 }}>
          {title}
        </span>
        <span className="badge" style={cssVar("tone", verdictTone(verdict))}>
          {verdict}
        </span>
      </div>
      <div className="flex flex-col gap-1">
        {stats.map((s) => (
          <div key={s.label} className="flex items-center justify-between gap-2">
            <span className="text-[11px]" style={{ color: "var(--faint)" }}>
              {s.label}
            </span>
            <span className="mono tabular text-[12px]" style={{ color: "var(--fg)" }}>
              {s.value}
            </span>
          </div>
        ))}
      </div>
      {note && (
        <span className="text-[11px]" style={{ color: "var(--dim)", lineHeight: 1.5 }}>
          {note}
        </span>
      )}
    </div>
  );
}

/**
 * The PQC readiness timeline (docs/PHASE4.md W1 position 1, the Crypto
 * panel's hero): the three NCSC milestones, each with its verdict
 * (on-track/at-risk/not-started, color-coded) and counts. The 2031 card's
 * "migrated" stat is always `0` in qryx's own report - labeled "not tracked
 * by qryx" rather than shown as real remediation progress (see
 * `cryptoTypes.ts`'s doc comment on `NcscPriority.migratedCount`).
 */
export function CryptoTimeline({
  report,
  loading,
  error,
}: {
  report: NcscReport | null;
  loading: boolean;
  error: CryptoError | null;
}) {
  if (loading) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        scanning...
      </div>
    );
  }
  if (error) {
    return (
      <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
        {describeCryptoError(error)}
      </div>
    );
  }
  if (!report) {
    return (
      <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
        set a target path and scan to see the PQC readiness timeline.
      </div>
    );
  }

  const { discovery2028: d, highestPriority2031: p, fullMigration2035: f } = report;

  return (
    <div className="grid gap-3" style={{ gridTemplateColumns: "repeat(3, minmax(0, 1fr))" }}>
      <MilestoneCard
        title="2028 · complete discovery"
        verdict={d.verdict}
        stats={[
          { label: "inventoried", value: String(d.totalInventoried) },
          { label: "quantum-vulnerable", value: String(d.quantumVulnerableCount) },
          { label: "migration plan", value: d.migrationPlanExists ? "exists" : "none" },
        ]}
        note={d.migrationPlanNote || undefined}
      />
      <MilestoneCard
        title="2031 · highest-priority systems"
        verdict={p.verdict}
        stats={[
          { label: "in scope", value: String(p.count) },
          { label: "migrated", value: `${p.migratedCount} (not tracked by qryx)` },
          { label: "remaining", value: String(p.remainingCount) },
        ]}
        note={p.criteria || undefined}
      />
      <MilestoneCard
        title="2035 · full migration"
        verdict={f.verdict}
        stats={[{ label: "in scope", value: String(f.count) }]}
      />
    </div>
  );
}
