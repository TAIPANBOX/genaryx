import type { MoneyStatus } from "../moneyTypes";
import type { PolicyRecord, PolicyStatus } from "../policyTypes";
import type { Severity } from "../types";

/**
 * Posture-lite (docs/PHASE2.md Wave 3, "Posture-lite"): the pure
 * signals-in/findings-out core of the Posture view, kept free of
 * React/Tauri so it is easy to reason about (and, if ever needed, test) in
 * isolation - `PostureView.tsx` owns all the data-fetching (money status,
 * policy status, policies, bus signals) and just calls
 * [`computePostureFindings`] with what it currently has.
 *
 * All 4 v0 zonds (identical set the SwiftUI track builds), computed purely
 * from signals every panel in this app already fetches - no new Tauri
 * command or connector change:
 *
 * 1. devkey in use - `money_status`/`policy_status`'s own `org_domain`.
 * 2. Governance fail-open - `policy_list_policies()` returned zero policies.
 * 3. Schema mix v0.1 + v0.2 - both envelope versions observed on the bus.
 * 4. Bus stale - no bus event observed recently, or the feed is empty.
 *
 * Every finding also carries a `state` distinguishing "checked and clean"
 * (`ok`) from "checked and it fired" (`triggered`) from "cannot tell yet"
 * (`unknown` - e.g. the relevant backend has not finished bootstrapping).
 * PHASE2.md's own wording ("a read-only list of stack-sanity findings")
 * reads as a punch list, but always rendering the full 4-row checklist with
 * an honest `unknown`/`ok`/`triggered` state per row is both what the Wave-3
 * parity checklist asks for ("Posture panel shows the 4 v0 zonds") and more
 * useful in practice - the whole point of a posture board is seeing what
 * currently CANNOT be verified, not just what has already gone wrong.
 */

export type FindingState = "ok" | "triggered" | "unknown";

export interface PostureFinding {
  id: "devkey" | "governance_fail_open" | "schema_mix" | "bus_stale";
  title: string;
  /** The severity IF `state === "triggered"` - fixed per zond, straight
   * from PHASE2.md (devkey/governance high, schema-mix info, bus-stale
   * medium); a non-triggered finding renders its own calm `ok`/`unknown`
   * badge instead (see `PostureView.tsx`), never this severity. */
  severity: Severity;
  state: FindingState;
  whyItMatters: string;
  /** A concrete command / env var, worded to match PHASE2.md's own Wave-3
   * zond list as closely as possible rather than paraphrased. */
  howToFix: string;
}

/** Everything [`computePostureFindings`] needs, gathered by `PostureView.tsx`
 * from the same commands/hooks every other panel already uses
 * (`usePolicyStatus`/`useMoneyStatus`/`fetchPolicies`/the live bus). */
export interface PostureInput {
  moneyStatus: MoneyStatus | null;
  policyStatus: PolicyStatus | null;
  /** `null` until `policy_list_policies` has resolved at least once (only
   * ever attempted once `policyStatus.state === "ready"`, mirroring
   * `PolicyView.tsx`'s own `ready` gate). */
  policies: PolicyRecord[] | null;
  /** Whether the initial bus read (`fetchRecentEvents`, the same call
   * `DecisionStream.tsx`/`BusExplorer.tsx` make) has resolved - lets the
   * bus-derived zonds report `unknown` while still loading rather than a
   * momentarily-false `ok`/`triggered`. */
  busLoaded: boolean;
  /** Total events observed so far (initial batch + live arrivals) - `0`
   * after a `busLoaded` read is exactly PHASE2.md's "or the feed is empty". */
  busEventCount: number;
  /** Epoch-ms of the most recently observed event's own `ts`, across every
   * source (not just wardryx) - `null` only when `busEventCount === 0`. */
  lastEventAtMs: number | null;
  /** Raw `schema` strings observed across every source so far. */
  schemasSeen: ReadonlySet<string>;
  /** Caller-supplied "now", so this function stays a pure, deterministic
   * computation - `PostureView.tsx` ticks this on an interval so "bus
   * stale" keeps re-evaluating even when no new event ever arrives to
   * otherwise trigger a re-render. */
  nowMs: number;
}

/** Wardryx/TokenFuse Cloud's shared "devkey resolves to this org" constant -
 * grounded by `src-tauri/src/money/state.rs`'s own test
 * (`devkey fallback resolves org=default (unsanitized already-safe)`), the
 * one place in this codebase that actually asserts the devkey -> org
 * mapping. Wardryx's own `org_domain` (`policy/state.rs::org_domain_for`) is
 * derived locally rather than learned from a pairing response and so never
 * literally equals `"default"` today, but the check honors both status
 * sources uniformly - see [`orgDomainIsDefault`]. */
const DEVKEY_ORG_DOMAIN = "default";

/** Schema version literals - `SchemaVersion::SCHEMA_V0_1`/`SCHEMA_V0_2` in
 * `crates/core/src/event.rs`, copied verbatim (this module has no reason to
 * depend on the Rust crate just for two constant strings that never
 * change - see `events.rs`'s / `mockData.ts`'s own identical literals). */
const SCHEMA_V0_1 = "taipanbox.dev/agent-event/v0.1";
const SCHEMA_V0_2 = "taipanbox.dev/agent-event/v0.2";

/** "No bus event observed in the last ~60s" - PHASE2.md's own number for
 * the "Bus stale" zond. */
const STALE_THRESHOLD_MS = 60_000;

function orgDomainIsDefault(status: MoneyStatus | PolicyStatus | null): boolean {
  if (!status || status.state !== "ready") return false;
  return status.org_domain === DEVKEY_ORG_DOMAIN;
}

function devkeyFinding(input: PostureInput): PostureFinding {
  const moneyReady = input.moneyStatus?.state === "ready";
  const policyReady = input.policyStatus?.state === "ready";
  let state: FindingState = "unknown";
  if (moneyReady || policyReady) {
    state = orgDomainIsDefault(input.moneyStatus) || orgDomainIsDefault(input.policyStatus) ? "triggered" : "ok";
  }
  return {
    id: "devkey",
    title: "devkey in use",
    severity: "high",
    state,
    whyItMatters:
      'The environment authenticates via a devkey / ALLOW_DEVKEY fallback (org resolved to "default", or the bearer is literally "devkey").',
    howToFix: "Mint real keys: taipan up mints them automatically, or set real TOKENFUSE_CLOUD_KEYS / WARDRYX_KEYS.",
  };
}

function governanceFinding(input: PostureInput): PostureFinding {
  let state: FindingState = "unknown";
  if (input.policyStatus?.state === "ready" && input.policies !== null) {
    state = input.policies.length === 0 ? "triggered" : "ok";
  }
  return {
    id: "governance_fail_open",
    title: "Governance fail-open: no policies",
    severity: "high",
    state,
    whyItMatters: "Wardryx is reachable but list_policies() is empty, so every agent action is currently allowed.",
    howToFix: "PUT policies onto Wardryx, or bring the stack up with taipan up --with wardryx and a seeded -policy.",
  };
}

function schemaMixFinding(input: PostureInput): PostureFinding {
  let state: FindingState = "unknown";
  if (input.busLoaded) {
    state =
      input.busEventCount > 0 && input.schemasSeen.has(SCHEMA_V0_1) && input.schemasSeen.has(SCHEMA_V0_2)
        ? "triggered"
        : "ok";
  }
  return {
    id: "schema_mix",
    title: "Schema mix v0.1 + v0.2",
    severity: "info",
    state,
    whyItMatters: "The bus carries both envelope versions (tokenfuse/qryx emit v0.1, wardryx/verdryx/mockryx v0.2).",
    howToFix: "Informational, not a defect - resolved by the tokenfuse-core v0.2 migration (workstream C).",
  };
}

function busStaleFinding(input: PostureInput): PostureFinding {
  let state: FindingState = "unknown";
  if (input.busLoaded) {
    if (input.busEventCount === 0 || input.lastEventAtMs === null) {
      state = "triggered"; // "or the feed is empty"
    } else {
      state = input.nowMs - input.lastEventAtMs > STALE_THRESHOLD_MS ? "triggered" : "ok";
    }
  }
  return {
    id: "bus_stale",
    title: "Bus stale",
    severity: "medium",
    state,
    whyItMatters: "No bus event has been observed in the last ~60 seconds, or the events feed is empty.",
    howToFix: "Check the feeder / the descriptor's events paths.",
  };
}

/** The 4 v0 zonds, in PHASE2.md's own order. Pure and total: never throws,
 * always returns exactly 4 findings regardless of how incomplete `input`
 * currently is (an unresolved backend just yields more `unknown` states,
 * never a missing row or a crash). */
export function computePostureFindings(input: PostureInput): PostureFinding[] {
  return [devkeyFinding(input), governanceFinding(input), schemaMixFinding(input), busStaleFinding(input)];
}
