import type { ReactNode } from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { formatHm, formatUsd } from "../lib/format";
import {
  describeAdmissionError,
  drillGapNote,
  readyToProposeStrict,
  runAdmissionBaseline,
  runAdmissionCheck,
} from "../lib/admission";
import { useAdmissionStatus } from "../lib/useAdmissionStatus";
import { lastSeenLabel, totalCalls } from "../lib/credentials";
import type { AdmissionBaseline, AdmissionCheck, AdmissionError } from "../admissionTypes";
import { describeDrillsError, runDrills } from "../lib/drills";
import { hasGaps } from "../drillsTypes";
import type { DrillsError, MockryxReport } from "../drillsTypes";
import { ConfirmButton } from "./ConfirmButton";
import { DrillsResults } from "./DrillsResults";
import { FreshBadge } from "./FreshBadge";

const FIELD_STYLE = {
  background: "var(--panel)",
  border: "1px solid var(--line-2)",
  borderRadius: 8,
  padding: "6px 10px",
  fontSize: 12,
  color: "var(--fg)",
  width: "100%",
} as const;

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-[11px]" style={{ color: "var(--dim)" }}>
        {label}
      </span>
      {children}
    </label>
  );
}

/** What the per-row "Verify" action and a fresh "Generate" both push into
 * this section - see this component's own doc comment. `nonce` forces a
 * re-apply of `keyId`/`agentId` even when the operator had since edited the
 * fields for a DIFFERENT newcomer: each is a fresh, deliberate "verify THIS
 * one" intent that should win over an in-flight edit, exactly like
 * `OnboardView.tsx`'s own "prefill live until touched" fields work the other
 * direction (an operator edit wins over a stale computed default). */
export interface AdmissionSeed {
  keyId: string;
  agentId: string;
  nonce: number;
}

/** One scoreboard card - a titled `d-card` with a left tone bar. `tone`
 * `"neutral"` is the honest "not checked / not run yet" state this
 * component's own doc requires for every card, and specifically for the
 * optional baseline leg even after a real attempt failed to run at all. */
function ScoreCard({
  title,
  tone,
  children,
}: {
  title: string;
  tone: "mint" | "amber" | "ember" | "neutral";
  children: ReactNode;
}) {
  const toneVar =
    tone === "mint"
      ? "var(--sev-low)"
      : tone === "amber"
        ? "var(--sev-medium)"
        : tone === "ember"
          ? "var(--sev-high)"
          : "var(--line-2)";
  return (
    <div
      className="d-card px-4 py-3 flex flex-col gap-2"
      style={{ borderLeft: `3px solid ${toneVar}` }}
    >
      <span
        className="mono text-[10px] uppercase tracking-wider"
        style={{ color: "var(--faint)" }}
      >
        {title}
      </span>
      {children}
    </div>
  );
}

function YesNoBadge({ label, yes }: { label: string; yes: boolean }) {
  return (
    <span className="chip" style={cssVar("dot", yes ? "var(--mint)" : "var(--sev-medium)")}>
      <span className="dot" aria-hidden="true" />
      {label}: {yes ? "yes" : "no"}
    </span>
  );
}

/** One labelled, copy-to-clipboard text block for the "enable strict"
 * proposal - a simpler, plain-text sibling of `OnboardView.tsx`'s own
 * `CopyBlock` (no JSON syntax highlighting needed for a shell/env snippet).
 * Deliberately its own small copy, not an import: this codebase's own
 * convention keeps this exact idiom duplicated per view rather than shared
 * (see `OnboardView.tsx`'s `CopyBlock` doc comment, which itself names
 * `RemoteCloudInventory.tsx`'s `CliInventory` as the sibling it mirrors, not
 * imports). */
function CopyText({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  const copy = useCallback(() => {
    void navigator.clipboard?.writeText(text).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      },
      () => setCopied(false),
    );
  }, [text]);
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex justify-end">
        <button
          type="button"
          className="mono"
          style={{
            background: "none",
            border: "none",
            cursor: "pointer",
            fontSize: 10.5,
            color: copied ? "var(--mint)" : "var(--accent)",
          }}
          onClick={copy}
        >
          {copied ? "copied" : "copy"}
        </button>
      </div>
      <pre
        className="mono thin-scroll"
        style={{
          margin: 0,
          background: "var(--panel)",
          border: "1px solid var(--line-2)",
          borderRadius: 8,
          padding: "8px 11px",
          fontSize: 11.5,
          color: "var(--fg)",
          whiteSpace: "pre-wrap",
          wordBreak: "break-word",
          lineHeight: 1.5,
        }}
      >
        {text}
      </pre>
    </div>
  );
}

function strictProposalText(agentId: string): string {
  return [
    "# Admission gate: enable strict identity mode (tokenfuse docs/20)",
    "# The wizard proposes; you edit the gateway's own env and restart it.",
    "",
    "# Step 1 - warn first, observe:",
    "TOKENFUSE_IDENTITY_STRICT=warn",
    "# A mismatched call still proceeds; the response carries",
    "# `x-fuse-identity: would-block=<reason>` and the trace keeps the",
    "# resolved unit, so you can see what WOULD be blocked before anything is.",
    "",
    "# Step 2 - once warn shows no unexpected would-block, enforce:",
    "TOKENFUSE_IDENTITY_STRICT=enforce",
    "# A mismatched call gets 403 with the identity_mismatch error contract;",
    "# the call never reaches the provider.",
    "",
    `# Verified newcomer for this proposal: ${agentId || "(agent id not set)"}`,
    "# Either step requires restarting the gateway process with the new env -",
    "# this console does not restart it or edit any env file for you.",
  ].join("\n");
}

/**
 * The admission-gate Verify section (I6, docs/ADMISSION.md), mounted by
 * `OnboardView.tsx` after the generated-bundle block. Proves a newcomer's
 * key is known and bound, that first traffic flows, rehearses the
 * guardrails with a drill AS the newcomer key (reusing the EXISTING
 * `drills_run` unmodified - no new command for that leg), optionally
 * establishes a Verdryx quality baseline through the gateway, then shows a
 * copy-paste "enable strict" proposal once the whole picture looks ready.
 * Propose, never mutate: nothing here edits env vars or config.
 */
export function AdmissionVerify({ seed }: { seed: AdmissionSeed | null }) {
  const sectionRef = useRef<HTMLDivElement>(null);
  const status = useAdmissionStatus();

  const [keyId, setKeyId] = useState("");
  const [agentId, setAgentId] = useState("");
  // Never persisted anywhere by this console - used only as an argument to
  // `runDrills`/`runAdmissionBaseline` below, cleared on unmount (the effect
  // just below) as a defense-in-depth measure even though the state is
  // already gone the moment this component unmounts.
  const [apiKey, setApiKey] = useState("");
  useEffect(() => {
    return () => setApiKey("");
  }, []);

  // Re-apply the seed whenever a fresh one arrives (a new bundle just
  // generated, or a per-row "Verify" click) - see `AdmissionSeed`'s own doc
  // comment for why `nonce` must win over an in-flight edit here.
  const lastNonce = useRef<number | null>(null);
  useEffect(() => {
    if (seed === null || seed.nonce === lastNonce.current) return;
    lastNonce.current = seed.nonce;
    setKeyId(seed.keyId);
    setAgentId(seed.agentId);
    sectionRef.current?.scrollIntoView({ behavior: "smooth", block: "start" });
  }, [seed]);

  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNowMs(Date.now()), 5_000);
    return () => window.clearInterval(id);
  }, []);

  // ---- admission_check ----
  const [check, setCheck] = useState<AdmissionCheck | null>(null);
  const [checkError, setCheckError] = useState<AdmissionError | null>(null);
  const [checking, setChecking] = useState(false);
  const [checkedAtMs, setCheckedAtMs] = useState<number | null>(null);

  const canCheck = keyId.trim().length > 0 && agentId.trim().length > 0 && !checking;
  const onRunChecks = useCallback(async () => {
    if (!canCheck) return;
    setChecking(true);
    setCheckError(null);
    try {
      const result = await runAdmissionCheck(keyId.trim(), agentId.trim());
      setCheck(result);
      setCheckedAtMs(Date.now());
    } catch (err) {
      setCheckError(err as AdmissionError);
    } finally {
      setChecking(false);
    }
  }, [canCheck, keyId, agentId]);

  // ---- drill (reuses the EXISTING drills_run - no new command) ----
  const [scenarioDir, setScenarioDir] = useState("");
  useEffect(() => {
    if (!status?.drills_scenario_dir || scenarioDir.length > 0) return;
    setScenarioDir(status.drills_scenario_dir);
  }, [status, scenarioDir]);

  const [drillReport, setDrillReport] = useState<MockryxReport | null>(null);
  const [drillError, setDrillError] = useState<DrillsError | null>(null);
  const [drillRunning, setDrillRunning] = useState(false);
  const [drillRanAtMs, setDrillRanAtMs] = useState<number | null>(null);

  const canDrill = scenarioDir.trim().length > 0 && apiKey.trim().length > 0 && !drillRunning;
  const onRunDrill = useCallback(async () => {
    if (!canDrill) return;
    setDrillRunning(true);
    setDrillError(null);
    try {
      const report = await runDrills(scenarioDir.trim(), apiKey, false, "");
      setDrillReport(report);
      setDrillRanAtMs(Date.now());
    } catch (err) {
      setDrillError(err as DrillsError);
    } finally {
      setDrillRunning(false);
    }
  }, [canDrill, scenarioDir, apiKey]);

  // ---- quality baseline ----
  const [evalsetPath, setEvalsetPath] = useState("");
  const [model, setModel] = useState(""); // deliberately empty - no silent default that spends
  const [baseline, setBaseline] = useState<AdmissionBaseline | null>(null);
  const [baselineError, setBaselineError] = useState<AdmissionError | null>(null);
  const [baselineRunning, setBaselineRunning] = useState(false);
  const [baselineRanAtMs, setBaselineRanAtMs] = useState<number | null>(null);

  const verdryxReady = Boolean(status?.verdryx_bin_present && status?.verdryx_db);
  const canBaseline =
    verdryxReady &&
    evalsetPath.trim().length > 0 &&
    model.trim().length > 0 &&
    agentId.trim().length > 0 &&
    apiKey.trim().length > 0 &&
    !baselineRunning;
  const onRunBaseline = useCallback(async () => {
    if (!canBaseline) return;
    setBaselineRunning(true);
    setBaselineError(null);
    try {
      const result = await runAdmissionBaseline(
        evalsetPath.trim(),
        model.trim(),
        agentId.trim(),
        apiKey,
      );
      setBaseline(result);
      setBaselineRanAtMs(Date.now());
    } catch (err) {
      setBaselineError(err as AdmissionError);
    } finally {
      setBaselineRunning(false);
    }
  }, [canBaseline, evalsetPath, model, agentId, apiKey]);

  const checkedHhmm = checkedAtMs !== null ? formatHm(checkedAtMs) : undefined;
  const drillHhmm = drillRanAtMs !== null ? formatHm(drillRanAtMs) : undefined;
  const baselineHhmm = baselineRanAtMs !== null ? formatHm(baselineRanAtMs) : undefined;

  const readyForStrict = readyToProposeStrict(check, drillReport, drillError);
  const gapNote = drillGapNote(drillReport);

  return (
    <div ref={sectionRef} className="flex flex-col gap-4 px-4 py-3">
      <div className="flex items-center gap-2 flex-wrap">
        <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
          Prove the newcomer's key is known and bound, that first traffic flows, then rehearse the
          guardrails and optionally set a quality baseline - all through the live gateway, under
          the newcomer's own key. This never edits env vars or config.
        </span>
      </div>

      <div className="grid gap-2.5" style={{ gridTemplateColumns: "1fr 1fr" }}>
        <Field label="key id">
          <input
            className="mono"
            style={FIELD_STYLE}
            value={keyId}
            onChange={(e) => setKeyId(e.target.value)}
            placeholder="billing-agent"
            spellCheck={false}
          />
        </Field>
        <Field label="agent id">
          <input
            className="mono"
            style={FIELD_STYLE}
            value={agentId}
            onChange={(e) => setAgentId(e.target.value)}
            placeholder="agent://bank.example/treasury/recon-batch"
            spellCheck={false}
          />
        </Field>
      </div>

      <Field label="api key (used only for the drill and baseline runs below - never stored by this console, cleared when you leave this view)">
        <input
          className="mono"
          type="password"
          style={FIELD_STYLE}
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          placeholder="gx_... (the newcomer's own client key secret)"
          spellCheck={false}
          autoComplete="off"
        />
      </Field>

      <div className="flex items-center gap-3 flex-wrap">
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
          onClick={() => void onRunChecks()}
          disabled={!canCheck}
        >
          {checking ? "Checking..." : "Run checks"}
        </button>
        <FreshBadge variant="onDemand" detail={checkedHhmm} />
        {status?.gateway.state !== "ready" && (
          <span className="text-[11px]" style={{ color: "var(--sev-medium)" }}>
            gateway leg: {status?.gateway.state ?? "resolving..."}
          </span>
        )}
      </div>

      {checkError && (
        <div className="mono text-[11.5px]" style={{ color: "var(--sev-high)" }}>
          {describeAdmissionError(checkError)}
        </div>
      )}

      {/* ---- scoreboard ---- */}
      <div className="grid gap-3" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(260px, 1fr))" }}>
        <ScoreCard title="Key known to gateway" tone={check === null ? "neutral" : check.key ? "mint" : "ember"}>
          {check === null ? (
            <span className="text-[11.5px]" style={{ color: "var(--faint)" }}>
              not checked yet
            </span>
          ) : check.key === null ? (
            <span className="text-[11.5px]" style={{ color: "var(--sev-high)" }}>
              key `{check.key_id}` is unknown to the gateway - it has never been configured there.
            </span>
          ) : (
            <div className="flex flex-col gap-1.5">
              <div className="flex flex-wrap gap-1.5">
                <YesNoBadge label="configured" yes={check.key.configured} />
                <YesNoBadge label="bound" yes={check.key.bound} />
              </div>
              <span className="text-[11px]" style={{ color: "var(--dim)" }}>
                unit: {check.key.unit ?? "none"}
              </span>
            </div>
          )}
        </ScoreCard>

        <ScoreCard title="Identity mapped" tone={check === null ? "neutral" : check.in_map ? "mint" : "ember"}>
          {check === null ? (
            <span className="text-[11.5px]" style={{ color: "var(--faint)" }}>
              not checked yet
            </span>
          ) : (
            <div className="flex flex-col gap-1.5">
              <YesNoBadge label="in_map" yes={check.in_map} />
              <span className="text-[11px]" style={{ color: "var(--dim)" }}>
                strict mode: <span className="mono">{check.strict_mode}</span>
              </span>
              {!check.identity_map_configured && (
                <span className="text-[11px]" style={{ color: "var(--faint)" }}>
                  no identity map configured on this gateway - in_map can never be true here.
                </span>
              )}
            </div>
          )}
        </ScoreCard>

        <ScoreCard
          title="First traffic"
          tone={check?.key == null ? "neutral" : totalCalls(check.key) > 0 ? "mint" : "amber"}
        >
          {check?.key == null ? (
            <span className="text-[11.5px]" style={{ color: "var(--faint)" }}>
              no key to check traffic on yet - run checks first.
            </span>
          ) : (
            <div className="flex flex-col gap-1">
              <span className="text-[11px]" style={{ color: "var(--dim)" }}>
                since startup: <span className="mono tabular">{check.key.since_startup.calls}</span> calls
              </span>
              <span className="text-[11px]" style={{ color: "var(--dim)" }}>
                history:{" "}
                <span className="mono tabular">
                  {check.key.history ? check.key.history.calls : "n/a (no history store)"}
                </span>
              </span>
              <span className="text-[11px]" style={{ color: "var(--faint)" }}>
                last seen: {lastSeenLabel(check.key, nowMs)}
              </span>
            </div>
          )}
        </ScoreCard>

        <ScoreCard
          title="Drill"
          tone={
            drillError ? "ember" : drillReport === null ? "neutral" : hasGaps(drillReport) ? "amber" : "mint"
          }
        >
          {drillError ? (
            <span className="text-[11.5px]" style={{ color: "var(--sev-high)" }}>
              {describeDrillsError(drillError)}
            </span>
          ) : drillReport === null ? (
            <span className="text-[11.5px]" style={{ color: "var(--faint)" }}>
              not run yet
            </span>
          ) : (
            <div className="flex flex-col gap-1">
              <span className="text-[11.5px]" style={{ color: hasGaps(drillReport) ? "var(--sev-medium)" : "var(--mint)" }}>
                {hasGaps(drillReport) ? "gaps found" : "guardrails held"}
              </span>
              <span className="text-[11px]" style={{ color: "var(--dim)" }}>
                {drillReport.results.length} scenarios · ran at {drillHhmm}
              </span>
              <span className="text-[10.5px]" style={{ color: "var(--faint)" }}>
                full results below
              </span>
            </div>
          )}
        </ScoreCard>

        <ScoreCard
          title="Quality baseline"
          tone={baselineError ? "ember" : baseline === null ? "neutral" : "mint"}
        >
          {baselineError ? (
            <span className="text-[11.5px]" style={{ color: "var(--sev-high)" }}>
              {describeAdmissionError(baselineError)}
            </span>
          ) : baseline === null ? (
            <span className="text-[11.5px]" style={{ color: "var(--faint)" }}>
              not run
            </span>
          ) : (
            <div className="flex flex-col gap-1">
              <span className="mono text-[11px]" style={{ color: "var(--fg)" }}>
                run {baseline.run_id}
              </span>
              <span className="text-[11px]" style={{ color: "var(--dim)" }}>
                mean score: {baseline.mean_score !== null ? baseline.mean_score.toFixed(3) : "n/a"} ·
                cases: {baseline.case_count}
              </span>
              <span className="text-[11px]" style={{ color: "var(--dim)" }}>
                cost: {formatUsd(baseline.total_cost_usd)}
              </span>
              <span className="mono text-[10.5px]" style={{ color: "var(--faint)" }}>
                baseline: {baseline.baseline_id_or_label}
              </span>
              <span className="text-[10.5px]" style={{ color: "var(--faint)" }}>
                ran at {baselineHhmm}
              </span>
            </div>
          )}
        </ScoreCard>
      </div>

      {/* ---- full drill detail (kept OUT of the narrow scoreboard grid
           above - DrillsResults's own scenario cards want at least 340px
           each and would otherwise overflow a scoreboard column) ---- */}
      {drillReport && (
        <div className="flex flex-col gap-2">
          <span className="text-[11px]" style={{ color: "var(--faint)" }}>
            Drill results
          </span>
          <DrillsResults report={drillReport} />
        </div>
      )}

      {/* ---- drill controls ---- */}
      <div className="d-card px-4 py-3 flex flex-col gap-2.5" style={{ background: "var(--panel-2)" }}>
        <span className="text-[11px]" style={{ color: "var(--dim)" }}>
          Run drill as this key
        </span>
        <div className="flex items-center gap-2 flex-wrap">
          <input
            className="mono flex-1 min-w-0"
            style={FIELD_STYLE}
            value={scenarioDir}
            onChange={(e) => setScenarioDir(e.target.value)}
            placeholder="/path/to/mockryx/scenarios"
            spellCheck={false}
          />
        </div>
        <div className="flex items-center gap-3 flex-wrap">
          <ConfirmButton
            label="Run drill as this key"
            confirmLabel="Confirm - sends real requests"
            tone="var(--sev-medium)"
            disabled={!canDrill}
            onConfirm={() => onRunDrill()}
          />
          <span className="text-[11px]" style={{ color: "var(--faint)" }}>
            Sends real requests through the gateway using the api key above, and will deliberately
            try to trip guardrails - that is the point of a drill.
          </span>
        </div>
        {apiKey.trim().length === 0 && (
          <span className="text-[11px]" style={{ color: "var(--sev-medium)" }}>
            enter the newcomer's api key above to enable this.
          </span>
        )}
      </div>

      {/* ---- baseline controls ---- */}
      <div className="d-card px-4 py-3 flex flex-col gap-2.5" style={{ background: "var(--panel-2)" }}>
        <span className="text-[11px]" style={{ color: "var(--dim)" }}>
          Run baseline eval
        </span>
        <div className="grid gap-2.5" style={{ gridTemplateColumns: "1fr 1fr" }}>
          <Field label="evalset path">
            <input
              className="mono"
              style={FIELD_STYLE}
              value={evalsetPath}
              onChange={(e) => setEvalsetPath(e.target.value)}
              placeholder="/path/to/evalset.json"
              spellCheck={false}
            />
          </Field>
          <Field label="model (no default - type the exact model id)">
            <input
              className="mono"
              style={FIELD_STYLE}
              value={model}
              onChange={(e) => setModel(e.target.value)}
              placeholder="claude-sonnet-5"
              spellCheck={false}
            />
          </Field>
        </div>
        <div className="flex items-center gap-3 flex-wrap">
          <ConfirmButton
            label="Run baseline eval"
            confirmLabel="Confirm - spends real provider money"
            tone="var(--sev-high)"
            disabled={!canBaseline}
            onConfirm={() => onRunBaseline()}
          />
          <span className="text-[11px]" style={{ color: "var(--faint)" }}>
            Calls {model.trim() || "(the model you name above)"} through the gateway under the
            newcomer's own key and spends real provider money.
          </span>
        </div>
        {!verdryxReady && (
          <span className="text-[11px]" style={{ color: "var(--sev-medium)" }}>
            verdryx binary and/or verdryx.db not resolved yet - see the Verify status above the
            wizard.
          </span>
        )}
      </div>

      {/* ---- enable strict proposal ---- */}
      {readyForStrict && (
        <div
          className="panel px-4 py-3 flex flex-col gap-2.5"
          style={{
            background: "color-mix(in srgb, var(--mint) 8%, var(--panel))",
            borderColor: "color-mix(in srgb, var(--mint) 40%, var(--line-2))",
          }}
        >
          <span className="text-[11.5px]" style={{ color: "var(--fg)" }}>
            Key bound, identity mapped, first traffic seen, and the drill ran without an
            infrastructure error - here is a copy-paste proposal to enable strict identity mode.
            This is a proposal: the console changes nothing.
          </span>
          {gapNote && (
            <span className="text-[11px]" style={{ color: "var(--sev-medium)" }}>
              {gapNote}
            </span>
          )}
          <CopyText text={strictProposalText(agentId)} />
        </div>
      )}
    </div>
  );
}
