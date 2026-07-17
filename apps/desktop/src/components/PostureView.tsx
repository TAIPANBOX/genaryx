import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { describePolicyError, fetchPolicies } from "../lib/policy";
import { computePostureFindings, type FindingState, type PostureFinding } from "../lib/posture";
import { fetchRecentEvents } from "../lib/recentEvents";
import { useMoneyStatus } from "../lib/useMoneyStatus";
import { usePolicyStatus } from "../lib/usePolicyStatus";
import type { MoneyStatus } from "../moneyTypes";
import type { PolicyError, PolicyRecord, PolicyStatus } from "../policyTypes";
import type { UiEvent } from "../types";
import { SeverityBadge } from "./SeverityBadge";

/** Same cap DecisionStream/BusExplorer apply to the bus read - Posture only
 * needs enough of a recent window to tell whether both schema versions are
 * present and when the newest event landed, not the whole history. */
const FETCH_LIMIT = 500;

/** Tauri event name the Rust live feeder (`src-tauri/src/live.rs`) emits on -
 * the SAME event every other bus-consuming view listens for. */
const LIVE_EVENT = "bus:event";

/** Matches `PolicyView.tsx`'s own periodic re-fetch cadence for the policy
 * list, since the "governance fail-open" zond reads the exact same
 * `policy_list_policies` data that panel's Policies section shows. */
const POLICIES_REFRESH_MS = 20_000;

/** How often `nowMs` re-ticks so the "bus stale" zond keeps re-evaluating
 * even when no new event arrives to otherwise trigger a render - well under
 * `posture.ts`'s 60s staleness threshold so the badge flips promptly. */
const NOW_TICK_MS = 5_000;

function parseTsMs(ts: string): number | null {
  const ms = new Date(ts).getTime();
  return Number.isNaN(ms) ? null : ms;
}

function SectionHeader({ title }: { title: string }) {
  return (
    <span className="mono" style={{ fontSize: 11, letterSpacing: "0.1em", textTransform: "uppercase", color: "var(--faint)" }}>
      {title}
    </span>
  );
}

function moneyStatusLabel(status: MoneyStatus | null): string {
  if (!status) return "connecting...";
  switch (status.state) {
    case "bootstrapping":
      return "connecting...";
    case "no_environment":
      return "no environment";
    case "pairing_failed":
      return "pairing failed";
    case "ready":
      return status.source.source === "taipan" ? `taipan up . ${status.source.name}` : "env fallback";
  }
}

function policyStatusLabel(status: PolicyStatus | null): string {
  if (!status) return "connecting...";
  switch (status.state) {
    case "bootstrapping":
      return "connecting...";
    case "no_environment":
      return "no policy plane";
    case "unreachable":
      return "unreachable";
    case "ready":
      return status.source.source === "taipan" ? `taipan up . ${status.source.name}` : "env fallback";
  }
}

/** Small connectivity chip, one per plane Posture reads from - context for
 * WHY a given zond below might read "n/a" (that plane not ready yet)
 * without duplicating either panel's own full empty-state treatment
 * (`MoneyEmptyState`/`PolicyEmptyState` already own that). */
function PlaneChip({ label, ready, text }: { label: string; ready: boolean; text: string }) {
  return (
    <span className="chip" style={cssVar("dot", ready ? "var(--sev-low)" : "var(--faint)")}>
      <span className="dot" aria-hidden="true" />
      {label}: {text}
    </span>
  );
}

/** The finding's status badge: its real (PHASE2.md-assigned) severity when
 * it has actually fired, a calm "OK" when the check passed, or a neutral
 * "n/a" while the signal it needs is not available yet - never a fake
 * severity for the latter two, matching `SeverityBadge`'s own "never look
 * more assured than the data actually is" spirit. */
function FindingStatusBadge({ state, severity }: { state: FindingState; severity: PostureFinding["severity"] }) {
  if (state === "triggered") return <SeverityBadge severity={severity} />;
  if (state === "unknown") {
    return (
      <span className="badge" style={cssVar("tone", "var(--faint)")}>
        n/a
      </span>
    );
  }
  return (
    <span className="badge" style={cssVar("tone", "var(--sev-low)")}>
      OK
    </span>
  );
}

function FindingRow({ finding }: { finding: PostureFinding }) {
  return (
    <div className="panel px-3 py-2.5 flex flex-col gap-2" style={{ background: "var(--panel-2)" }}>
      <div className="flex items-center gap-3">
        <FindingStatusBadge state={finding.state} severity={finding.severity} />
        <span className="text-[12.5px]" style={{ color: "var(--fg)" }}>
          {finding.title}
        </span>
      </div>
      <span className="text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.6 }}>
        {finding.whyItMatters}
      </span>
      {finding.state === "triggered" && (
        <span className="mono text-[11px]" style={{ color: "var(--faint)" }}>
          fix: <span style={{ color: "var(--fg)" }}>{finding.howToFix}</span>
        </span>
      )}
    </div>
  );
}

/**
 * Posture-lite (docs/PHASE2.md Wave 3): a new read-only sidebar view listing
 * the 4 v0 stack-sanity zonds (`lib/posture.ts`'s `computePostureFindings`),
 * computed entirely from signals this app already fetches elsewhere - the
 * resolved env sources (`usePolicyStatus`/`useMoneyStatus`, same hooks the
 * Policy/Money/Overview views already use), `policy_list_policies()` (same
 * command `PolicyView.tsx`'s own Policies section calls), and the live bus
 * (the same `fetchRecentEvents` + `bus:event` listener pattern
 * `DecisionStream.tsx`/`BusExplorer.tsx` already follow, unfiltered here
 * since "schema mix" and "bus stale" are properties of the WHOLE bus, not
 * just the wardryx slice). No new Tauri command, no new connector call.
 *
 * Deliberately never gated behind a single-plane "ready" check the way
 * `PolicyView`/`OverviewView` are: Posture's whole point is reading across
 * multiple, independently-failing planes at once, so a down Wardryx must
 * never blank the whole panel - each zond just reports its own honest
 * `unknown` state until the signal it specifically needs is available (see
 * `posture.ts`'s doc comment).
 */
export function PostureView() {
  const moneyStatus = useMoneyStatus();
  const policyStatus = usePolicyStatus();

  const [policies, setPolicies] = useState<PolicyRecord[] | null>(null);
  const [policiesError, setPoliciesError] = useState<PolicyError | null>(null);

  const [busLoaded, setBusLoaded] = useState(false);
  const [busEventCount, setBusEventCount] = useState(0);
  const [lastEventAtMs, setLastEventAtMs] = useState<number | null>(null);
  const [schemasSeen, setSchemasSeen] = useState<ReadonlySet<string>>(new Set());

  const [nowMs, setNowMs] = useState(() => Date.now());

  // Policies: only once Wardryx is actually ready (mirrors PolicyView.tsx's
  // own `ready` gate for the exact same read), then on the same 20s cadence.
  useEffect(() => {
    if (policyStatus?.state !== "ready") return;
    let cancelled = false;
    const load = () => {
      fetchPolicies()
        .then((p) => {
          if (cancelled) return;
          setPolicies(p);
          setPoliciesError(null);
        })
        .catch((err: unknown) => {
          if (!cancelled) setPoliciesError(err as PolicyError);
        });
    };
    load();
    const id = window.setInterval(load, POLICIES_REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [policyStatus?.state]);

  // Bus signals, initial batch: same read DecisionStream/BusExplorer make,
  // unfiltered (every source, not just wardryx - both bus-derived zonds
  // concern the whole bus).
  useEffect(() => {
    let cancelled = false;
    void fetchRecentEvents(FETCH_LIMIT).then((res) => {
      if (cancelled) return;
      setBusEventCount(res.events.length);
      setSchemasSeen(new Set(res.events.map((e) => e.schema)));
      // `fetchRecentEvents` is newest-first (mirrors `Store::recent_events`),
      // so the first element is the most recent - see `events.rs`'s doc.
      const newest = res.events[0];
      setLastEventAtMs(newest ? parseTsMs(newest.ts) : null);
      setBusLoaded(true);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // Bus signals, live updates: same listener DecisionStream/BusExplorer
  // subscribe to, folded into the running signals rather than kept as a
  // growing list (Posture never needs to render individual rows).
  useEffect(() => {
    if (!isTauri()) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    listen<UiEvent>(LIVE_EVENT, (event) => {
      const e = event.payload;
      setBusEventCount((n) => n + 1);
      setSchemasSeen((prev) => (prev.has(e.schema) ? prev : new Set(prev).add(e.schema)));
      const ms = parseTsMs(e.ts);
      if (ms !== null) {
        setLastEventAtMs((prev) => (prev === null || ms > prev ? ms : prev));
      }
    })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch((err: unknown) => {
        // eslint-disable-next-line no-console
        console.error(`listen(${LIVE_EVENT}) failed (posture):`, err);
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const id = window.setInterval(() => setNowMs(Date.now()), NOW_TICK_MS);
    return () => window.clearInterval(id);
  }, []);

  const findings = computePostureFindings({
    moneyStatus,
    policyStatus,
    policies,
    busLoaded,
    busEventCount,
    lastEventAtMs,
    schemasSeen,
    nowMs,
  });

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-6">
      <div className="flex flex-wrap items-center gap-2">
        <PlaneChip label="money" ready={moneyStatus?.state === "ready"} text={moneyStatusLabel(moneyStatus)} />
        <PlaneChip label="policy" ready={policyStatus?.state === "ready"} text={policyStatusLabel(policyStatus)} />
      </div>

      {policiesError && (
        <div className="panel px-3 py-2 mono text-[11.5px]" style={{ background: "var(--panel-2)", color: "var(--sev-high)" }}>
          {describePolicyError(policiesError)}
        </div>
      )}

      <section className="flex flex-col gap-2">
        <SectionHeader title="Stack posture" />
        <div className="flex flex-col gap-2">
          {findings.map((f) => (
            <FindingRow key={f.id} finding={f} />
          ))}
        </div>
      </section>
    </div>
  );
}
