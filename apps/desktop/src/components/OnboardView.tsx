import type { ReactNode } from "react";
import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { formatHm } from "../lib/format";
import {
  describeOnboardError,
  fetchOnboardStatus,
  generateOnboardBundle,
  isExistingFileError,
  writeOnboardPassport,
} from "../lib/onboard";
import { ATTESTATION_METHODS } from "../onboardTypes";
import type {
  OnboardBundle,
  OnboardError,
  OnboardGenerateRequest,
  OnboardStatus,
  OnboardWriteResult,
} from "../onboardTypes";
import { ConfirmButton } from "./ConfirmButton";
import { FreshBadge } from "./FreshBadge";
import { JsonPreview } from "./JsonPreview";
import { Hero, HeroBand, KpiTile, Section } from "./dash";

const FIELD_STYLE = {
  background: "var(--panel)",
  border: "1px solid var(--line-2)",
  borderRadius: 8,
  padding: "6px 10px",
  fontSize: 12,
  color: "var(--fg)",
  width: "100%",
} as const;

const PASSPORT_COLUMNS = "1fr 1fr 1fr 110px";

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

/** A blank string, or one that does not parse to a finite number, both mean
 * "not set" - an honest `null` beats silently sending a typo through as a
 * number. */
function parseOptionalUsd(s: string): number | null {
  const t = s.trim();
  if (t.length === 0) return null;
  const n = Number(t);
  return Number.isFinite(n) ? n : null;
}

/**
 * One labelled, copy-to-clipboard text block - the shape every piece of the
 * generated bundle renders in (mirrors `RemoteCloudInventory.tsx`'s
 * `CliInventory`'s identical copy-button idiom). `json` renders through
 * `JsonPreview` for the syntax-colored view when the text parses; otherwise
 * (or when it does not parse) it falls back to a plain preformatted block -
 * every field here is already a pretty-printed string from the backend, this
 * never re-serializes anything of its own.
 */
function CopyBlock({
  label,
  text,
  json,
  note,
}: {
  label: string;
  text: string;
  json?: boolean;
  note?: ReactNode;
}) {
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

  let parsed: unknown = undefined;
  if (json) {
    try {
      parsed = JSON.parse(text);
    } catch {
      parsed = undefined;
    }
  }

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center justify-between gap-2">
        <span
          className="mono text-[10px] uppercase tracking-wider"
          style={{ color: "var(--faint)" }}
        >
          {label}
        </span>
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
      {note}
      {json && parsed !== undefined ? (
        <JsonPreview value={parsed} />
      ) : (
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
      )}
    </div>
  );
}

/** No backend at all (a plain `vite build`/browser preview) - the standard
 * no-backend guard every plane shows, Onboard-flavored: this wizard reads
 * the OPERATOR's own local filesystem, so there is no honest mock to fall
 * back to. */
function OnboardEmptyState() {
  return (
    <div className="flex-1 min-h-0 flex items-center justify-center px-6">
      <div
        className="panel px-5 py-4 flex flex-col gap-2"
        style={{ background: "var(--panel-2)", maxWidth: 480 }}
      >
        <span style={{ fontSize: 13, color: "var(--fg)" }}>No backend available</span>
        <span className="mono text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.7 }}>
          The Onboard wizard reads your local identity map and passports directory directly - open
          this console from the desktop app, or from a live web session, to use it.
        </span>
      </div>
    </div>
  );
}

/**
 * The Onboard panel (docs/ONBOARD.md, D15/B2): the "new agent" wizard. Reads
 * the loaded identity map and the passports staging dir on mount, proposes a
 * full registration bundle (passport, client key, identity-map fragment,
 * Wardryx policy stub, Terraform alternative) from a form, and can write
 * ONLY the passport file - everything else is copy-paste, committed by the
 * operator into their own git. Genuinely on-demand like Drills/Evidence:
 * nothing here ever auto-runs except the initial status read.
 */
export function OnboardView() {
  const [status, setStatus] = useState<OnboardStatus | null>(null);
  const [statusError, setStatusError] = useState<OnboardError | null>(null);
  const [statusLoading, setStatusLoading] = useState(true);
  const [statusAtMs, setStatusAtMs] = useState<number | null>(null);

  const loadStatus = useCallback(async () => {
    setStatusLoading(true);
    try {
      const s = await fetchOnboardStatus();
      setStatus(s);
      setStatusError(null);
      setStatusAtMs(Date.now());
    } catch (err) {
      setStatusError(err as OnboardError);
    } finally {
      setStatusLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadStatus();
  }, [loadStatus]);

  // ---- form state ----
  const [trustDomain, setTrustDomain] = useState("");
  const [path, setPath] = useState("");
  const [unitMode, setUnitMode] = useState<"existing" | "custom">("existing");
  const [selectedUnit, setSelectedUnit] = useState("");
  const [customUnit, setCustomUnit] = useState("");
  const [owner, setOwner] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [runtime, setRuntime] = useState("");
  const [attestationMethod, setAttestationMethod] = useState<string>(ATTESTATION_METHODS[0]);
  const [unitBudget, setUnitBudget] = useState("");
  const [keyIdOverride, setKeyIdOverride] = useState("");
  const [keyIdTouched, setKeyIdTouched] = useState(false);
  const [bindPattern, setBindPattern] = useState("");
  const [bindPatternTouched, setBindPatternTouched] = useState(false);
  const [requireHumanAboveUsd, setRequireHumanAboveUsd] = useState("");

  const units = status?.units ?? [];
  const hasUnitChoices = units.length > 0;
  const effectiveUnit = (hasUnitChoices && unitMode !== "custom" ? selectedUnit : customUnit).trim();
  const unitIsNew = effectiveUnit.length > 0 && !units.some((u) => u.id === effectiveUnit);

  // Prefill the unit picker once, on the first status load - never overwrite
  // an operator's own pick on a later re-poll (mirrors EvidenceView.tsx's
  // identical "prefill once" discipline).
  useEffect(() => {
    if (units.length > 0 && unitMode !== "custom" && selectedUnit === "") {
      setSelectedUnit(units[0].id);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [status]);

  const computedAgentId =
    trustDomain.trim().length > 0 && path.trim().length > 0
      ? `agent://${trustDomain.trim()}/${path.trim()}`
      : "";
  const computedKeyId = path.trim().length > 0 ? path.trim().replace(/\//g, "-") : "";

  // Live-prefilled (not once): the bind pattern / key id previews track the
  // trust domain + path as the operator types, exactly like the backend's
  // own defaults would compute them, until the operator edits the field
  // directly - at which point their own value wins and stops following.
  useEffect(() => {
    if (!bindPatternTouched) setBindPattern(computedAgentId);
  }, [computedAgentId, bindPatternTouched]);
  useEffect(() => {
    if (!keyIdTouched) setKeyIdOverride(computedKeyId);
  }, [computedKeyId, keyIdTouched]);

  const [generating, setGenerating] = useState(false);
  const [bundle, setBundle] = useState<OnboardBundle | null>(null);
  const [generateError, setGenerateError] = useState<OnboardError | null>(null);
  const [generatedAtMs, setGeneratedAtMs] = useState<number | null>(null);

  const [writing, setWriting] = useState(false);
  const [writeResult, setWriteResult] = useState<OnboardWriteResult | null>(null);
  const [writeError, setWriteError] = useState<OnboardError | null>(null);
  const [needsOverwriteConfirm, setNeedsOverwriteConfirm] = useState(false);

  const canGenerate =
    trustDomain.trim().length > 0 &&
    path.trim().length > 0 &&
    effectiveUnit.length > 0 &&
    owner.trim().length > 0 &&
    !generating;

  const onGenerate = useCallback(async () => {
    if (!canGenerate) return;
    setGenerating(true);
    setGenerateError(null);
    try {
      const request: OnboardGenerateRequest = {
        trust_domain: trustDomain.trim(),
        path: path.trim(),
        unit: effectiveUnit,
        owner: owner.trim(),
        display_name: displayName.trim().length > 0 ? displayName.trim() : null,
        runtime: runtime.trim().length > 0 ? runtime.trim() : null,
        attestation_method: attestationMethod,
        key_id: keyIdTouched && keyIdOverride.trim().length > 0 ? keyIdOverride.trim() : null,
        bind_pattern:
          bindPatternTouched && bindPattern.trim().length > 0 ? bindPattern.trim() : null,
        require_human_above_usd: parseOptionalUsd(requireHumanAboveUsd),
        unit_budget_usd_month: unitIsNew ? parseOptionalUsd(unitBudget) : null,
        map_path: status?.map_path ?? null,
        passports_dir: status?.passports_dir ?? null,
      };
      const b = await generateOnboardBundle(request);
      setBundle(b);
      setGeneratedAtMs(Date.now());
      setWriteResult(null);
      setWriteError(null);
      setNeedsOverwriteConfirm(false);
    } catch (err) {
      setGenerateError(err as OnboardError);
    } finally {
      setGenerating(false);
    }
  }, [
    canGenerate,
    trustDomain,
    path,
    effectiveUnit,
    owner,
    displayName,
    runtime,
    attestationMethod,
    keyIdTouched,
    keyIdOverride,
    bindPatternTouched,
    bindPattern,
    requireHumanAboveUsd,
    unitIsNew,
    unitBudget,
    status,
  ]);

  const doWrite = useCallback(
    async (overwrite: boolean) => {
      if (!bundle) return;
      setWriting(true);
      setWriteError(null);
      setNeedsOverwriteConfirm(false);
      try {
        const res = await writeOnboardPassport({
          passport_json: bundle.passport_json,
          passport_path: bundle.passport_path,
          passports_dir: status?.passports_dir ?? null,
          overwrite,
        });
        setWriteResult(res);
        void loadStatus();
      } catch (err) {
        const oe = err as OnboardError;
        setWriteError(oe);
        if (isExistingFileError(oe)) setNeedsOverwriteConfirm(true);
      } finally {
        setWriting(false);
      }
    },
    [bundle, status, loadStatus],
  );

  if (statusError?.kind === "no_environment") {
    return <OnboardEmptyState />;
  }

  if (statusLoading && !status) {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center">
        <div className="mono text-[12px]" style={{ color: "var(--faint)" }}>
          reading the local identity map and passports dir...
        </div>
      </div>
    );
  }

  const statusHhmm = statusAtMs !== null ? formatHm(statusAtMs) : undefined;
  const generatedHhmm = generatedAtMs !== null ? formatHm(generatedAtMs) : undefined;
  const inMapCount = status?.passports.filter((p) => p.in_map).length ?? 0;
  const notInMapCount = (status?.passports.length ?? 0) - inMapCount;
  const skippedCount = status?.skipped.length ?? 0;

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <FreshBadge
          variant="onDemand"
          detail={statusHhmm}
          title="onboard never runs on its own - status, generate, and write only happen on an explicit click"
        />
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 10px", fontSize: 11 }}
          onClick={() => void loadStatus()}
        >
          Refresh
        </button>
        <div className="flex-1" />
      </div>

      {statusError && (
        <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
          {describeOnboardError(statusError)}
        </div>
      )}

      {status && (
        <HeroBand
          hero={
            <Hero
              cap="Onboard · agent passports"
              value={status.passports.length.toLocaleString("en-US")}
              sub={
                <>
                  {inMapCount} in map · {notInMapCount} not in map
                </>
              }
            />
          }
          tiles={
            <>
              <KpiTile
                label="In map"
                value={inMapCount.toLocaleString("en-US")}
                tone={inMapCount > 0 ? "var(--mint)" : undefined}
                sub={`of ${status.passports.length.toLocaleString("en-US")} provisioned`}
              />
              <KpiTile
                label="Not in map"
                value={notInMapCount.toLocaleString("en-US")}
                tone={notInMapCount > 0 ? "var(--sev-medium)" : "var(--mint)"}
                sub={notInMapCount > 0 ? "provisioned, no map entry yet" : "every passport is mapped"}
              />
              <KpiTile
                label="Skipped"
                value={skippedCount.toLocaleString("en-US")}
                tone={skippedCount > 0 ? "var(--sev-medium)" : "var(--mint)"}
                sub={skippedCount > 0 ? "could not be parsed" : "no unparseable files"}
              />
            </>
          }
        />
      )}

      {status && (
        <Section title="Identity map" right={<FreshBadge variant="onDemand" detail={statusHhmm} />}>
          <div className="flex flex-col gap-2 px-4 py-3">
            <div className="flex flex-wrap items-center gap-2">
              <span
                className="chip"
                style={cssVar(
                  "dot",
                  status.map_path
                    ? status.map_loaded
                      ? "var(--mint)"
                      : "var(--sev-medium)"
                    : "var(--faint)",
                )}
              >
                <span className="dot" aria-hidden="true" />
                {status.map_path
                  ? `${status.map_loaded ? "map loaded" : "map error"} · ${status.map_path}`
                  : "no identity map configured"}
              </span>
              <span className="chip" style={cssVar("dot", "var(--faint)")}>
                <span className="dot" aria-hidden="true" />
                passports dir · {status.passports_dir}
              </span>
            </div>
            {status.map_error && (
              <span className="mono text-[11px]" style={{ color: "var(--sev-medium)" }}>
                {status.map_error}
              </span>
            )}
            {!status.map_path && (
              <span className="text-[11px]" style={{ color: "var(--faint)" }}>
                No identity map found - the unit field below falls back to free text; the wizard still
                proposes a full bundle.
              </span>
            )}
          </div>
        </Section>
      )}

      {status && (
        <Section title="Provisioned passports">
          {status.passports.length === 0 ? (
            <div className="px-4 py-6 mono" style={{ color: "var(--faint)", fontSize: 12 }}>
              no provisioned passports found in {status.passports_dir} yet.
            </div>
          ) : (
            <div style={{ overflowX: "auto" }}>
              <div
                className="grid gap-3 px-4 py-2"
                style={{
                  gridTemplateColumns: PASSPORT_COLUMNS,
                  borderBottom: "1px solid var(--line-2)",
                  background: "var(--panel-3)",
                }}
              >
                {["agent id", "owner", "file", ""].map((label) => (
                  <span
                    key={label}
                    className="mono"
                    style={{
                      fontSize: 10,
                      letterSpacing: "0.08em",
                      textTransform: "uppercase",
                      color: "var(--faint)",
                    }}
                  >
                    {label}
                  </span>
                ))}
              </div>
              {status.passports.map((p) => (
                <div
                  key={p.file}
                  className="grid items-center gap-3 px-4 py-2 bus-row"
                  style={{ gridTemplateColumns: PASSPORT_COLUMNS }}
                >
                  <span
                    className="mono truncate text-[11.5px]"
                    title={p.agent_id}
                    style={{ color: "var(--fg)" }}
                  >
                    {p.agent_id}
                  </span>
                  <span
                    className="mono truncate text-[11.5px]"
                    title={p.owner}
                    style={{ color: "var(--dim)" }}
                  >
                    {p.owner}
                  </span>
                  <span
                    className="mono truncate text-[11px]"
                    title={p.file}
                    style={{ color: "var(--faint)" }}
                  >
                    {p.file}
                  </span>
                  <span
                    className="badge"
                    style={cssVar("tone", p.in_map ? "var(--mint)" : "var(--sev-medium)")}
                  >
                    {p.in_map ? "in map" : "not in map"}
                  </span>
                </div>
              ))}
            </div>
          )}
          {status.skipped.length > 0 && (
            <div className="px-4 py-2.5" style={{ borderTop: "1px solid var(--line-2)" }}>
              <details>
                <summary className="text-[11px]" style={{ color: "var(--faint)", cursor: "pointer" }}>
                  {status.skipped.length} file{status.skipped.length === 1 ? "" : "s"} skipped (could
                  not be parsed)
                </summary>
                <div className="flex flex-col gap-1 mt-1.5">
                  {status.skipped.map((s) => (
                    <div key={s.file} className="mono text-[11px]" style={{ color: "var(--dim)" }}>
                      {s.file} - {s.reason}
                    </div>
                  ))}
                </div>
              </details>
            </div>
          )}
        </Section>
      )}

      <div className="d-card px-4 py-3 flex flex-col gap-2.5">
        <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
          The wizard proposes; you commit. Only the passport file is written, into the local staging
          dir.
        </span>

        <div className="grid gap-2.5" style={{ gridTemplateColumns: "1fr 1fr" }}>
          <Field label="trust domain">
            <input
              className="mono"
              style={FIELD_STYLE}
              value={trustDomain}
              onChange={(e) => setTrustDomain(e.target.value)}
              placeholder="bank.example"
              spellCheck={false}
            />
          </Field>
          <Field label="path">
            <input
              className="mono"
              style={FIELD_STYLE}
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder="treasury/recon-batch"
              spellCheck={false}
            />
          </Field>
        </div>

        <Field label="unit">
          {hasUnitChoices ? (
            <div className="flex items-center gap-2 flex-wrap">
              <select
                className="mono"
                style={{ ...FIELD_STYLE, width: "auto", minWidth: 220 }}
                value={unitMode === "custom" ? "__custom__" : selectedUnit}
                onChange={(e) => {
                  if (e.target.value === "__custom__") {
                    setUnitMode("custom");
                  } else {
                    setUnitMode("existing");
                    setSelectedUnit(e.target.value);
                  }
                }}
              >
                {units.map((u) => (
                  <option key={u.id} value={u.id}>
                    {u.name ? `${u.name} (${u.id})` : u.id}
                    {u.budget_usd_month != null ? ` · $${u.budget_usd_month}/mo` : ""}
                  </option>
                ))}
                <option value="__custom__">Other (create new unit)</option>
              </select>
              {unitMode === "custom" && (
                <input
                  className="mono flex-1 min-w-0"
                  style={{ ...FIELD_STYLE, width: "auto" }}
                  value={customUnit}
                  onChange={(e) => setCustomUnit(e.target.value)}
                  placeholder="new-unit-id"
                  spellCheck={false}
                />
              )}
            </div>
          ) : (
            <input
              className="mono"
              style={FIELD_STYLE}
              value={customUnit}
              onChange={(e) => setCustomUnit(e.target.value)}
              placeholder="unit id (no identity map loaded - free text)"
              spellCheck={false}
            />
          )}
        </Field>

        {unitIsNew && (
          <Field label="monthly budget for the new unit, USD (optional)">
            <input
              className="mono"
              style={{ ...FIELD_STYLE, width: 160 }}
              type="number"
              min={0}
              step="0.01"
              value={unitBudget}
              onChange={(e) => setUnitBudget(e.target.value)}
              placeholder="blank = no budget set"
            />
          </Field>
        )}

        <div className="grid gap-2.5" style={{ gridTemplateColumns: "1fr 1fr" }}>
          <Field label="owner">
            <input
              className="mono"
              style={FIELD_STYLE}
              value={owner}
              onChange={(e) => setOwner(e.target.value)}
              placeholder="user://bank.example/olena, or free text"
              spellCheck={false}
            />
          </Field>
          <Field label="display name (optional)">
            <input
              className="mono"
              style={FIELD_STYLE}
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              placeholder="Recon batch agent"
              spellCheck={false}
            />
          </Field>
        </div>

        <div className="grid gap-2.5" style={{ gridTemplateColumns: "1fr 1fr" }}>
          <Field label="runtime (optional)">
            <input
              className="mono"
              style={FIELD_STYLE}
              value={runtime}
              onChange={(e) => setRuntime(e.target.value)}
              placeholder="claude-sonnet-4-5"
              spellCheck={false}
            />
          </Field>
          <Field label="attestation method">
            <select
              className="mono"
              style={FIELD_STYLE}
              value={attestationMethod}
              onChange={(e) => setAttestationMethod(e.target.value)}
            >
              {ATTESTATION_METHODS.map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
          </Field>
        </div>

        <details>
          <summary className="text-[11px]" style={{ color: "var(--faint)", cursor: "pointer" }}>
            advanced overrides (key id, bind pattern, approval threshold)
          </summary>
          <div className="flex flex-col gap-2.5 mt-2">
            <Field label="key id override (default: path with / -> -)">
              <input
                className="mono"
                style={FIELD_STYLE}
                value={keyIdOverride}
                onChange={(e) => {
                  setKeyIdOverride(e.target.value);
                  setKeyIdTouched(true);
                }}
                placeholder={computedKeyId || "auto from path"}
                spellCheck={false}
              />
            </Field>
            <Field label="bind pattern (default: the exact agent id; may end with one trailing *)">
              <input
                className="mono"
                style={FIELD_STYLE}
                value={bindPattern}
                onChange={(e) => {
                  setBindPattern(e.target.value);
                  setBindPatternTouched(true);
                }}
                placeholder={computedAgentId || "agent://<domain>/<path>"}
                spellCheck={false}
              />
            </Field>
            <Field label="require human approval above, USD (optional)">
              <input
                className="mono"
                style={{ ...FIELD_STYLE, width: 160 }}
                type="number"
                min={0}
                step="0.01"
                value={requireHumanAboveUsd}
                onChange={(e) => setRequireHumanAboveUsd(e.target.value)}
                placeholder="blank = no threshold"
              />
            </Field>
          </div>
        </details>

        <div className="flex items-center gap-3 flex-wrap pt-1">
          <button
            type="button"
            className="icon-btn"
            style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
            onClick={() => void onGenerate()}
            disabled={!canGenerate}
          >
            {generating ? "Generating..." : "Generate"}
          </button>
          <span className="text-[11px]" style={{ color: "var(--faint)" }}>
            proposes a passport, a client key, an identity-map fragment, a Wardryx policy stub, and a
            Terraform alternative - nothing is written yet.
          </span>
        </div>
      </div>

      {generateError && (
        <div className="d-card px-3 py-2 mono" style={{ fontSize: 11.5, color: "var(--sev-high)" }}>
          {describeOnboardError(generateError)}
        </div>
      )}

      {bundle && (
        <Section
          title="Generated bundle"
          right={<FreshBadge variant="onDemand" detail={generatedHhmm} />}
        >
          <div className="flex flex-col gap-4 px-4 py-3">
            <div className="flex items-center gap-2 flex-wrap">
              <span className="chip" style={cssVar("dot", "var(--mint)")}>
                <span className="dot" aria-hidden="true" />
                {bundle.agent_id}
              </span>
              {bundle.unit_is_new && (
                <span className="badge" style={cssVar("tone", "var(--sev-medium)")}>
                  new unit
                </span>
              )}
            </div>

            <CopyBlock label="Passport JSON" text={bundle.passport_json} json />

            <div className="flex items-center gap-3 flex-wrap">
              <button
                type="button"
                className="icon-btn"
                style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
                onClick={() => void doWrite(false)}
                disabled={writing}
              >
                {writing ? "Writing..." : `Write to ${bundle.passport_path}`}
              </button>
              {writeResult && (
                <span className="text-[11px]" style={{ color: "var(--mint)" }}>
                  written · {writeResult.written_path}
                  {writeResult.created_dir ? " (created the passports dir)" : ""}
                </span>
              )}
            </div>

            {writeError && !needsOverwriteConfirm && (
              <div
                className="panel px-3 py-2 mono text-[11.5px]"
                style={{ background: "var(--panel)", color: "var(--sev-high)" }}
              >
                {describeOnboardError(writeError)}
              </div>
            )}

            {needsOverwriteConfirm && writeError && (
              <div
                className="panel px-3 py-2.5 flex items-center gap-3 flex-wrap"
                style={{
                  background: "var(--panel)",
                  borderColor: "color-mix(in srgb, var(--sev-medium) 45%, var(--line-2))",
                }}
              >
                <span className="text-[11.5px]" style={{ color: "var(--sev-medium)" }}>
                  {bundle.passport_path} already exists.
                </span>
                <ConfirmButton
                  label="Overwrite"
                  confirmLabel="Overwrite"
                  tone="var(--sev-medium)"
                  onConfirm={() => doWrite(true)}
                />
              </div>
            )}

            <div
              className="panel px-3 py-2.5 flex flex-col gap-1"
              style={{
                background: "color-mix(in srgb, var(--sev-high) 8%, var(--panel))",
                borderColor: "color-mix(in srgb, var(--sev-high) 40%, var(--line-2))",
              }}
            >
              <span className="text-[11.5px]" style={{ color: "var(--sev-high)" }}>
                Shown ONCE. This console never stores this secret - copy it now, before leaving this
                view.
              </span>
            </div>
            <CopyBlock label="Client key -> TOKENFUSE_CLIENT_KEYS" text={bundle.client_keys_line} />

            <CopyBlock
              label="Identity map fragment"
              text={bundle.identity_map_fragment}
              json
              note={
                bundle.unit_is_new ? (
                  <span className="text-[11px]" style={{ color: "var(--faint)" }}>
                    includes a new unit entry - this unit is not yet in the map.
                  </span>
                ) : undefined
              }
            />

            <CopyBlock label="Wardryx policy stub" text={bundle.wardryx_policy_stub} />

            <details>
              <summary className="text-[11px]" style={{ color: "var(--faint)", cursor: "pointer" }}>
                Terraform alternative
              </summary>
              <div className="mt-2">
                <CopyBlock label="Terraform" text={bundle.terraform_snippet} />
              </div>
            </details>
          </div>
        </Section>
      )}
    </div>
  );
}
