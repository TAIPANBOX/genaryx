import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { describeDrillsError, runDrills } from "../lib/drills";
import { useDrillsStatus } from "../lib/useDrillsStatus";
import { formatHm, formatUsd } from "../lib/format";
import { hasGaps } from "../drillsTypes";
import type { DrillsError, DrillsStatus, MockryxReport } from "../drillsTypes";
import { DrillsResults } from "./DrillsResults";
import { FreshBadge } from "./FreshBadge";
import { Hero, HeroBand, KpiTile, Section } from "./dash";

const FIELD_STYLE = {
  background: "var(--panel)",
  border: "1px solid var(--line-2)",
  borderRadius: 8,
  padding: "6px 10px",
  fontSize: 12,
  color: "var(--fg)",
} as const;

/**
 * Which `taipan up` environment the gateway was resolved from.
 *
 * "Runs real gateway calls and burns real budget" is written under the button.
 * WHICH environment those calls land in came down the wire the whole time, in
 * `status.source` (`drills::env::EnvSource`, the descriptor's own name), and
 * nothing showed it: the only thing on screen to tell two environments apart
 * was a loopback URL that is identical in both.
 *
 * A status with no source says so rather than falling back to a plausible
 * name. There is no default environment to name.
 */
export function environmentLabel(status: Extract<DrillsStatus, { state: "ready" }>): string {
  const name = status.source?.name;
  return name ? `env ${name}` : "env not recorded";
}

/**
 * Shared "not ready yet" rendering for the Drills view - mirrors
 * `CryptoView.tsx`'s local `CryptoEmptyState`: still resolving, or no drills
 * plane (no `mockryx` binary and/or no `taipan up` gateway - see
 * `drills::env`'s doc comment for why those are not distinguished here). No
 * "unreachable" state, same reason Crypto has none: mockryx has no serve
 * process to confirm reachable at bootstrap.
 */
function DrillsEmptyState({ status }: { status: DrillsStatus | null }) {
  if (!status || status.state === "bootstrapping") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center">
        <div className="mono text-[12px]" style={{ color: "var(--faint)" }}>
          resolving the mockryx binary and gateway...
        </div>
      </div>
    );
  }

  // `status.state === "ready"`: callers only render this component when NOT
  // ready, so only `no_environment` reaches here in practice.
  return (
    <div className="flex-1 min-h-0 flex items-center justify-center px-6">
      <div className="panel px-5 py-4 flex flex-col gap-2" style={{ background: "var(--panel-2)", maxWidth: 560 }}>
        <span style={{ fontSize: 13, color: "var(--fg)" }}>No drills plane found</span>
        <span className="mono text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.7 }}>
          Drills needs BOTH the <span style={{ color: "var(--fg)" }}>mockryx</span> binary (at{" "}
          <span style={{ color: "var(--fg)" }}>~/.taipan/bin/mockryx</span>, or a{" "}
          <span style={{ color: "var(--fg)" }}>~/Development/mockryx/bin/mockryx</span> checkout build) AND a{" "}
          <span style={{ color: "var(--fg)" }}>taipan up</span> environment for its gateway URL. Bring one up and
          build mockryx for the console to auto-discover both.
        </span>
      </div>
    </div>
  );
}

/**
 * The Drills panel (docs/PHASE4.md W2): an on-demand `mockryx run` rehearsal
 * against the resolved TokenFuse gateway. Genuinely on-demand, like Crypto's
 * qryx: mockryx has no live feed at all, so nothing here ever auto-runs -
 * only the explicit "Run drills" click does (spec: "never auto-run"), and
 * the result is labeled "as of last run".
 */
export function DrillsView() {
  const status = useDrillsStatus();
  const ready = status?.state === "ready";

  const [scenarioDir, setScenarioDir] = useState("");
  useEffect(() => {
    if (status?.state !== "ready" || scenarioDir.length > 0) return;
    const suggested = status.scenario_dir;
    if (suggested) setScenarioDir(suggested);
  }, [status, scenarioDir]);

  const [apiKeyOverride, setApiKeyOverride] = useState("");
  const [failOnSkip, setFailOnSkip] = useState(false);
  const [savePath, setSavePath] = useState("");

  const [report, setReport] = useState<MockryxReport | null>(null);
  const [error, setError] = useState<DrillsError | null>(null);
  const [running, setRunning] = useState(false);
  const [ranAtMs, setRanAtMs] = useState<number | null>(null);

  const onRun = useCallback(async () => {
    if (!ready || scenarioDir.trim().length === 0) return;
    setRunning(true);
    setError(null);
    try {
      const r = await runDrills(scenarioDir, apiKeyOverride, failOnSkip, savePath);
      setReport(r);
      setRanAtMs(Date.now());
    } catch (err) {
      setError(err as DrillsError);
    } finally {
      setRunning(false);
    }
  }, [ready, scenarioDir, apiKeyOverride, failOnSkip, savePath]);

  if (!ready) {
    return <DrillsEmptyState status={status} />;
  }

  const hhmm = ranAtMs !== null ? formatHm(ranAtMs) : undefined;
  const heldCount = report ? report.results.filter((r) => r.status !== "failed" && r.findings.length === 0).length : 0;
  const gapCount = report ? report.results.length - heldCount : 0;
  const totalCalls = report ? report.results.reduce((s, r) => s + r.metrics.calls, 0) : 0;
  const totalBudget = report ? report.results.reduce((s, r) => s + r.metrics.budget_burned_usd, 0) : 0;

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <span className="chip" style={cssVar("dot", "var(--src-mockryx)")}>
          <span className="dot" aria-hidden="true" />
          {status.mockryx_bin}
        </span>
        <span className="chip" style={cssVar("dot", "var(--faint)")}>
          <span className="dot" aria-hidden="true" />
          {environmentLabel(status)}
          {" · "}
          {status.gateway_url}
          {status.has_api_key ? " · bearer resolved" : " · no bearer resolved"}
        </span>
        <FreshBadge variant="onDemand" detail={hhmm} title="mockryx has no live feed - nothing here auto-runs" />
      </div>

      <div className="d-card px-4 py-3 flex flex-col gap-2.5">
        <div className="flex items-center gap-2">
          <span className="text-[11.5px] shrink-0" style={{ color: "var(--dim)" }}>
            scenario dir
          </span>
          <input
            className="mono flex-1 min-w-0"
            style={FIELD_STYLE}
            value={scenarioDir}
            onChange={(e) => setScenarioDir(e.target.value)}
            placeholder="/path/to/mockryx/scenarios"
            spellCheck={false}
          />
        </div>
        <div className="flex items-center gap-2 flex-wrap">
          <span className="text-[11.5px] shrink-0" style={{ color: "var(--dim)" }}>
            api key override
          </span>
          <input
            className="mono flex-1"
            style={{ ...FIELD_STYLE, minWidth: 140 }}
            value={apiKeyOverride}
            onChange={(e) => setApiKeyOverride(e.target.value)}
            placeholder={status.has_api_key ? "leave blank to use the resolved bearer" : "leave blank for none"}
            spellCheck={false}
          />
          <span className="text-[11.5px] shrink-0" style={{ color: "var(--dim)" }}>
            save report to
          </span>
          <input
            className="mono flex-1"
            style={{ ...FIELD_STYLE, minWidth: 140 }}
            value={savePath}
            onChange={(e) => setSavePath(e.target.value)}
            placeholder="/path/to/report.json (optional)"
            spellCheck={false}
          />
        </div>
        <label className="flex items-center gap-2 text-[11.5px]" style={{ color: "var(--dim)" }}>
          <input type="checkbox" checked={failOnSkip} onChange={(e) => setFailOnSkip(e.target.checked)} />
          fail on skip (promotes unconfigured-guardrail skips into gaps)
        </label>
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center gap-3 flex-wrap">
            <button
              type="button"
              className="icon-btn"
              style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
              onClick={() => void onRun()}
              disabled={running || scenarioDir.trim().length === 0}
            >
              {running ? "Running..." : "Run drills"}
            </button>
          </div>
          <span className="text-[11px]" style={{ color: "var(--faint)" }}>
            Runs real gateway calls and burns real budget.
          </span>
        </div>
      </div>

      {error && (
        <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
          {describeDrillsError(error)}
        </div>
      )}

      {report === null ? (
        <div className="mono" style={{ fontSize: 12, color: "var(--faint)" }}>
          run drills to see the verdict.
        </div>
      ) : (
        <HeroBand
          hero={
            <Hero
              cap="Drills · guardrail verdict"
              value={`${heldCount}/${report.results.length}`}
              sub={<>{hasGaps(report) ? "GAPS FOUND" : "all held"}</>}
              fuseFraction={report.results.length > 0 ? heldCount / report.results.length : 0}
              fuseTone={gapCount === 0 ? "mint" : "ember"}
              noteLeft={<>held <b>{heldCount}</b></>}
              noteRight={<>gaps <b>{gapCount}</b></>}
            />
          }
          tiles={
            <>
              <KpiTile label="Scenarios" value={report.results.length.toLocaleString("en-US")} sub={`${gapCount} with gaps`} />
              <KpiTile label="Budget burned" value={formatUsd(totalBudget)} sub={`${totalCalls.toLocaleString("en-US")} calls`} />
            </>
          }
        />
      )}

      <Section title="Results" right={<FreshBadge variant="onDemand" detail={hhmm} />}>
        {report === null ? (
          <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
            run drills to see results.
          </div>
        ) : (
          <DrillsResults report={report} />
        )}
      </Section>
    </div>
  );
}
