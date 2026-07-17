import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { describeQualityError, fetchBaselines, fetchRunScores, fetchRunSummaries } from "../lib/quality";
import { useQualityStatus } from "../lib/useQualityStatus";
import type { QualityError, QualityStatus, VerdryxBaseline, VerdryxRunSummary, VerdryxScore } from "../qualityTypes";
import { QualityBaselines } from "./QualityBaselines";
import { QualityDriftStream } from "./QualityDriftStream";
import { QualityRunDetail } from "./QualityRunDetail";
import { QualityRunsList } from "./QualityRunsList";

function SectionHeader({ title }: { title: string }) {
  return (
    <span className="mono" style={{ fontSize: 11, letterSpacing: "0.1em", textTransform: "uppercase", color: "var(--faint)" }}>
      {title}
    </span>
  );
}

function Loading() {
  return (
    <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
      loading...
    </div>
  );
}

/**
 * Shared "not ready yet" rendering for the Quality view - mirrors
 * `IdentityView.tsx`'s local `IdentityEmptyState`'s honest, distinct states
 * (never a generic spinner-forever or error toast), Verdryx-flavored: still
 * connecting, no quality plane configured, or a resolved `verdryx.db` path
 * that failed to open.
 */
function QualityEmptyState({ status }: { status: QualityStatus | null }) {
  if (!status || status.state === "bootstrapping") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center">
        <div className="mono text-[12px]" style={{ color: "var(--faint)" }}>
          connecting to a Verdryx quality plane...
        </div>
      </div>
    );
  }

  if (status.state === "no_environment") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center px-6">
        <div className="panel px-5 py-4 flex flex-col gap-2" style={{ background: "var(--panel-2)", maxWidth: 520 }}>
          <span style={{ fontSize: 13, color: "var(--fg)" }}>No quality plane found</span>
          <span className="mono text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.7 }}>
            Verdryx has no server to connect to - it writes <span style={{ color: "var(--fg)" }}>verdryx.db</span> when
            you run <span style={{ color: "var(--fg)" }}>verdryx eval</span>. Park (or symlink) that file at{" "}
            <span style={{ color: "var(--fg)" }}>~/.taipan/verdryx.db</span> for the console to auto-discover it.
          </span>
        </div>
      </div>
    );
  }

  if (status.state === "unreachable") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center px-6">
        <div className="panel px-5 py-4 flex flex-col gap-2" style={{ background: "var(--panel-2)", maxWidth: 520 }}>
          <span style={{ fontSize: 13, color: "var(--sev-high)" }}>Could not open verdryx.db</span>
          <span className="mono text-[11.5px]" style={{ color: "var(--dim)" }}>
            {status.db_path || "(no path resolved)"}
          </span>
          <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
            {status.reason}
          </span>
        </div>
      </div>
    );
  }

  // `status.state === "ready"`: callers only render this component when NOT
  // ready, so this branch is unreachable in practice.
  return null;
}

/**
 * The Quality panel (docs/PHASE4.md W1): eval-runs history + run detail,
 * saved baselines, and live drift alerts, over a read-only Verdryx
 * connection. Mirrors `IdentityView.tsx`'s overall shape (status hook, empty
 * state, section layout) and its "no periodic auto-refresh" discipline: like
 * `idryx serve`, `verdryx.db` only changes when an operator runs `verdryx
 * eval`/`baseline` externally, so a timer here would just be a no-op most of
 * the time - the explicit Refresh button re-reads the current file instead.
 */
export function QualityView({ onOpenAgent }: { onOpenAgent: (agentId: string) => void }) {
  const status = useQualityStatus();
  const ready = status?.state === "ready";

  const [runs, setRuns] = useState<VerdryxRunSummary[] | null>(null);
  const [baselines, setBaselines] = useState<VerdryxBaseline[] | null>(null);
  const [error, setError] = useState<QualityError | null>(null);
  const [asOfMs, setAsOfMs] = useState<number | null>(null);

  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [scores, setScores] = useState<VerdryxScore[] | null>(null);
  const [scoresError, setScoresError] = useState<QualityError | null>(null);

  const load = useCallback(async () => {
    if (!ready) return;
    try {
      const [r, b] = await Promise.all([fetchRunSummaries(), fetchBaselines()]);
      setRuns(r);
      setBaselines(b);
      setAsOfMs(Date.now());
      setError(null);
      setSelectedRunId((prev) => (prev && r.some((x) => x.run.id === prev) ? prev : (r[0]?.run.id ?? null)));
    } catch (err) {
      setError(err as QualityError);
    }
  }, [ready]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!selectedRunId) {
      setScores(null);
      setScoresError(null);
      return;
    }
    let cancelled = false;
    setScores(null);
    setScoresError(null);
    fetchRunScores(selectedRunId)
      .then((s) => {
        if (!cancelled) setScores(s);
      })
      .catch((err: unknown) => {
        if (!cancelled) setScoresError(err as QualityError);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedRunId]);

  if (!ready) {
    return <QualityEmptyState status={status} />;
  }

  const selectedSummary = runs?.find((r) => r.run.id === selectedRunId) ?? null;

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-6">
      <div className="flex flex-wrap items-center gap-2">
        <span className="chip" style={cssVar("dot", "var(--src-verdryx)")}>
          <span className="dot" aria-hidden="true" />
          {status.source.source === "taipan" ? `taipan up · ${status.source.name}` : "well-known ~/.taipan/verdryx.db"}
          &nbsp;&middot;&nbsp;{status.db_path}
        </span>
        <span className="chip" style={cssVar("dot", "var(--faint)")}>
          <span className="dot" aria-hidden="true" />
          as of load{asOfMs !== null ? ` · fetched ${new Date(asOfMs).toLocaleTimeString()}` : ""}
        </span>
        <div className="flex-1" />
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 10px", fontSize: 11 }}
          onClick={() => void load()}
        >
          Refresh
        </button>
      </div>

      {error && (
        <div className="panel px-3 py-2 mono text-[11.5px]" style={{ background: "var(--panel-2)", color: "var(--sev-high)" }}>
          {describeQualityError(error)}
        </div>
      )}

      <section className="flex flex-col gap-2">
        <SectionHeader title="Eval Runs" />
        {runs === null ? <Loading /> : <QualityRunsList runs={runs} selectedRunId={selectedRunId} onSelect={setSelectedRunId} />}
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Run Detail" />
        <QualityRunDetail summary={selectedSummary} scores={scores} error={scoresError} />
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Baselines" />
        {baselines === null ? <Loading /> : <QualityBaselines baselines={baselines} runs={runs} />}
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Drift Alerts" />
        <QualityDriftStream onOpenAgent={onOpenAgent} />
      </section>
    </div>
  );
}
