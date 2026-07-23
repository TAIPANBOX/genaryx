import { useEffect, useState } from "react";
import type { CopilotStatus } from "../copilotTypes";
import type { IdentityStatus, IdryxAlert, IdryxIdentity } from "../identityTypes";
import type { MoneyStatus, Run } from "../moneyTypes";
import type { Approval, PolicyError, PolicyRecord, PolicyStatus } from "../policyTypes";
import type { UiEvent } from "../types";
import { fetchCopilotStatus } from "./copilot";
import { fetchAlerts, fetchIdentities } from "./identity";
import { fetchRuns } from "./money";
import { fetchApprovals, fetchPolicies } from "./policy";
import {
  computeConnectionHealthFindings,
  computeIdentityPostureFindings,
  computeStackPostureFindings,
  type PostureFinding,
} from "./posture";
import { fetchRecentEvents } from "./recentEvents";
import { hasBackend, subscribeBackend } from "./transport";
import { useCryptoStatus } from "./useCryptoStatus";
import { useDrillsStatus } from "./useDrillsStatus";
import { useIdentityStatus } from "./useIdentityStatus";
import { useMemoryStatus } from "./useMemoryStatus";
import { useMoneyStatus } from "./useMoneyStatus";
import { usePolicyStatus } from "./usePolicyStatus";
import { useQualityStatus } from "./useQualityStatus";
import { useRemoteStatus } from "./useRemoteStatus";

/** Same cap DecisionStream/BusExplorer/PostureView's own prior bus read
 * applied - enough of a recent window to tell whether both schema versions
 * are present and when the newest event landed, not the whole history. */
const FETCH_LIMIT = 500;

/** Tauri event name the Rust live feeder (`src-tauri/src/live.rs`) emits on -
 * the SAME event every other bus-consuming view listens for. */
const LIVE_EVENT = "bus:event";

/** Matches `PolicyView.tsx`'s own periodic re-fetch cadence for the policy
 * list (unchanged from PostureView.tsx's prior inline effect) - approvals
 * ride the SAME cadence since both are Wardryx/policy-plane reads, but as a
 * fully independent fetch (see the approvals effect below) so an approvals
 * failure can never suppress a policies update or vice versa. */
const POLICIES_REFRESH_MS = 20_000;

/** Money runs, for [`cloudIngestFreshnessFinding`] - matches
 * `OverviewView.tsx`/`MoneyView.tsx`'s own `REFRESH_INTERVAL_MS` so "how
 * fresh is the newest run" is judged against data no staler than the Money
 * panel's own. */
const RUNS_REFRESH_MS = 20_000;

/** How often `nowMs` re-ticks so every age-based zond keeps re-evaluating
 * even when no new event/fetch arrives to otherwise trigger a render - well
 * under any of this file's staleness thresholds so a badge flips promptly. */
const NOW_TICK_MS = 5_000;

function parseTsMs(ts: string): number | null {
  const ms = new Date(ts).getTime();
  return Number.isNaN(ms) ? null : ms;
}

/**
 * Everything the Posture board computes, plus the raw plane statuses
 * `PostureView.tsx` itself renders as connectivity chips. Split out of
 * `PostureView.tsx` (which owned all of this inline before I3) so
 * `OverviewView.tsx`'s Incident Center (I2, `lib/incidents.ts`) can consume
 * the SAME live findings rather than re-deriving its own copy of nine
 * `*_status` reads plus policies/identities/alerts/bus signals/runs/
 * approvals - both views mount this hook independently (never
 * simultaneously; `AppShell.tsx` renders exactly one view at a time), each
 * getting its own fresh reads, matching this codebase's existing "each view
 * owns its own reads" convention (`Agent360.tsx`'s identity fetch, this
 * file's own identities/alerts effect below) rather than one view reaching
 * into another's state.
 */
export interface PostureData {
  moneyStatus: MoneyStatus | null;
  policyStatus: PolicyStatus | null;
  identityStatus: IdentityStatus | null;
  /** Surfaces a `policy_list_policies` failure the same way
   * `PostureView.tsx` always has - approvals failures are deliberately NOT
   * surfaced here (see the approvals effect below), matching the "empty
   * list on failure" fallback this file already uses for identities/alerts. */
  policiesError: PolicyError | null;
  stackFindings: PostureFinding[];
  identityFindings: PostureFinding[];
  connectionFindings: PostureFinding[];
}

export function usePostureData(): PostureData {
  const moneyStatus = useMoneyStatus();
  const policyStatus = usePolicyStatus();
  const identityStatus = useIdentityStatus();
  const qualityStatus = useQualityStatus();
  const cryptoStatus = useCryptoStatus();
  const memoryStatus = useMemoryStatus();
  const drillsStatus = useDrillsStatus();
  const remoteStatus = useRemoteStatus();

  // Copilot has no bootstrapping/polling shape at all (a flat DTO, see
  // `copilotTypes.ts`), so there is no dedicated `use*Status` hook for it -
  // this one-shot fetch mirrors `CopilotView.tsx`'s own identical inline
  // pattern rather than inventing a new hook for a status that never
  // re-resolves on its own.
  const [copilotStatus, setCopilotStatus] = useState<CopilotStatus | null>(null);
  useEffect(() => {
    void fetchCopilotStatus().then(setCopilotStatus);
  }, []);

  const [policies, setPolicies] = useState<PolicyRecord[] | null>(null);
  const [policiesError, setPoliciesError] = useState<PolicyError | null>(null);
  const [approvals, setApprovals] = useState<Approval[] | null>(null);

  // Identity: fetched ONCE when the identity plane becomes ready (never on a
  // periodic timer, unlike `policies`/`approvals` below) - mirrors
  // `IdentityView.tsx`'s own no-auto-refresh rationale, doubly so here: idryx
  // `serve` never changes on its own, so a periodic re-fetch would keep
  // resetting `identitySnapshotAsOfMs` back to "just now", hiding exactly the
  // aging the `identity_snapshot_age` zond exists to surface. A failed fetch
  // deliberately leaves `identitySnapshotAsOfMs` at `null` rather than
  // stamping a "successful" read that never happened.
  const [identities, setIdentities] = useState<IdryxIdentity[] | null>(null);
  const [identityAlerts, setIdentityAlerts] = useState<IdryxAlert[] | null>(null);
  const [identitySnapshotAsOfMs, setIdentitySnapshotAsOfMs] = useState<number | null>(null);

  const [moneyRuns, setMoneyRuns] = useState<Run[] | null>(null);

  const [busLoaded, setBusLoaded] = useState(false);
  const [busEventCount, setBusEventCount] = useState(0);
  const [lastEventAtMs, setLastEventAtMs] = useState<number | null>(null);
  const [schemasSeen, setSchemasSeen] = useState<ReadonlySet<string>>(new Set());

  const [nowMs, setNowMs] = useState(() => Date.now());

  useEffect(() => {
    if (identityStatus?.state !== "ready") return;
    let cancelled = false;
    void Promise.all([fetchIdentities(), fetchAlerts()])
      .then(([ids, alerts]) => {
        if (cancelled) return;
        setIdentities(ids);
        setIdentityAlerts(alerts);
        setIdentitySnapshotAsOfMs(Date.now());
      })
      .catch(() => {
        if (cancelled) return;
        setIdentities([]);
        setIdentityAlerts([]);
      });
    return () => {
      cancelled = true;
    };
  }, [identityStatus?.state]);

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

  // Approvals (I3): same plane/cadence as policies above, but a fully
  // separate fetch/state/failure path rather than a `Promise.all` alongside
  // it - so an approvals-only hiccup can never suppress the policies update
  // `governanceFinding` depends on, or vice versa. Failure falls back to an
  // empty list (mirrors identities/alerts above), which
  // `approvalsWaitingFinding` reads honestly as "nothing pending" only if the
  // fetch actually succeeded with an empty array - a failed fetch instead
  // leaves `approvals` at whatever it last was, or `null` on the very first
  // attempt, so the zond keeps reporting `unknown` rather than a fabricated
  // `ok`.
  useEffect(() => {
    if (policyStatus?.state !== "ready") return;
    let cancelled = false;
    const load = () => {
      fetchApprovals()
        .then((a) => {
          if (!cancelled) setApprovals(a);
        })
        .catch(() => {
          // leave `approvals` as-is (or `null`) - see doc comment above.
        });
    };
    load();
    const id = window.setInterval(load, POLICIES_REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [policyStatus?.state]);

  // Money runs (I3): same cadence Money/Overview already poll runs at.
  useEffect(() => {
    if (moneyStatus?.state !== "ready") return;
    let cancelled = false;
    const load = () => {
      fetchRuns()
        .then((r) => {
          if (!cancelled) setMoneyRuns(r);
        })
        .catch(() => {
          // leave `moneyRuns` as-is (or `null`) - same "never fabricate a
          // successful read" rationale as approvals above.
        });
    };
    load();
    const id = window.setInterval(load, RUNS_REFRESH_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [moneyStatus?.state]);

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
    if (!hasBackend()) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    subscribeBackend<UiEvent>(LIVE_EVENT, (payload) => {
      const e = payload;
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
        console.error(`subscribe(${LIVE_EVENT}) failed (posture):`, err);
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

  const postureInput = {
    moneyStatus,
    policyStatus,
    policies,
    busLoaded,
    busEventCount,
    lastEventAtMs,
    schemasSeen,
    nowMs,
    identityStatus,
    identities,
    identityAlerts,
    identitySnapshotAsOfMs,
    qualityStatus,
    cryptoStatus,
    memoryStatus,
    drillsStatus,
    copilotStatus,
    remoteStatus,
    moneyRuns,
    approvals,
  };

  return {
    moneyStatus,
    policyStatus,
    identityStatus,
    policiesError,
    stackFindings: computeStackPostureFindings(postureInput),
    identityFindings: computeIdentityPostureFindings(postureInput),
    connectionFindings: computeConnectionHealthFindings(postureInput),
  };
}
