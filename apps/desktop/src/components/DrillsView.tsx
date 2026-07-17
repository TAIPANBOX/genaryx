import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { describeDrillsError, runDrills } from "../lib/drills";
import { useDrillsStatus } from "../lib/useDrillsStatus";
import type { DrillsError, DrillsStatus, MockryxReport } from "../drillsTypes";
import { DrillsResults } from "./DrillsResults";

const FIELD_STYLE = {
  background: "var(--panel)",
  border: "1px solid var(--line-2)",
  borderRadius: 8,
  padding: "6px 10px",
  fontSize: 12,
  color: "var(--fg)",
} as const;

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

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-6">
      <div className="flex flex-wrap items-center gap-2">
        <span className="chip" style={cssVar("dot", "var(--src-mockryx)")}>
          <span className="dot" aria-hidden="true" />
          {status.mockryx_bin}
        </span>
        <span className="chip" style={cssVar("dot", "var(--faint)")}>
          <span className="dot" aria-hidden="true" />
          {status.gateway_url}
          {status.has_api_key ? " · bearer resolved" : " · no bearer resolved"}
        </span>
        <span className="chip" style={cssVar("dot", "var(--faint)")}>
          <span className="dot" aria-hidden="true" />
          {ranAtMs !== null ? `as of last run · ${new Date(ranAtMs).toLocaleTimeString()}` : "no run yet"}
        </span>
      </div>

      <div className="panel px-4 py-3 flex flex-col gap-2.5" style={{ background: "var(--panel-2)" }}>
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
          <span className="text-[11px]" style={{ color: "var(--faint)" }}>
            makes real calls against the gateway above and burns real budget - never runs on its own.
          </span>
        </div>
      </div>

      {error && (
        <div className="panel px-3 py-2 mono text-[11.5px]" style={{ background: "var(--panel-2)", color: "var(--sev-high)" }}>
          {describeDrillsError(error)}
        </div>
      )}

      {report === null ? (
        <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
          run drills to see results.
        </div>
      ) : (
        <DrillsResults report={report} />
      )}
    </div>
  );
}
