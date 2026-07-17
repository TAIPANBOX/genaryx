import type { IdentityStatus, IdryxAlert, IdryxIdentity } from "../identityTypes";
import { ATTESTATION_DETECTORS } from "../identityTypes";
import type { MoneyStatus } from "../moneyTypes";
import type { PolicyRecord, PolicyStatus } from "../policyTypes";
import type { Severity } from "../types";

/**
 * Posture-lite (docs/PHASE2.md Wave 3) + Posture full's identity-plane
 * zonds (docs/PHASE3.md W4, position 6): the pure signals-in/findings-out
 * core of the Posture view, kept free of React/Tauri so it is easy to
 * reason about (and, if ever needed, test) in isolation -
 * `PostureView.tsx` owns all the data-fetching (money/policy/identity
 * status, policies, identities, alerts, bus signals) and just calls
 * [`computeStackPostureFindings`]/[`computeIdentityPostureFindings`] with
 * what it currently has.
 *
 * Posture-lite's 4 v0 zonds (identical set the SwiftUI track builds),
 * computed purely from signals every panel in this app already fetches - no
 * new Tauri command or connector change:
 *
 * 1. devkey in use - `money_status`/`policy_status`'s own `org_domain`.
 * 2. Governance fail-open - `policy_list_policies()` returned zero policies.
 * 3. Schema mix v0.1 + v0.2 - both envelope versions observed on the bus.
 * 4. Bus stale - no bus event observed recently, or the feed is empty.
 *
 * Posture full's 5 identity-plane zonds (PHASE3.md position 6), same shape,
 * computed purely from the SAME reads the Identity panel/Agent 360 already
 * make (`identity_status`, `identity_list_identities`,
 * `identity_list_alerts`) plus `policy_status` - again no new Tauri command:
 *
 * 5. idryx exposed - the discovered idryx URL is not loopback (idryx has no
 *    auth on any route, per docs/PHASE3.md's grounded contract).
 * 6. Attestation coverage - of the privileged identities, how many carry an
 *    `attestation_missing`/`bom_incomplete` alert.
 * 7. Identity snapshot age - idryx `serve` is load-once; how long since this
 *    console last confirmed the data (never idryx's own uptime, which the
 *    console cannot observe - see [`identitySnapshotAgeFinding`]).
 * 8. Detector-feed freshness - the newest alert's `time` vs now.
 * 9. Wardryx admin key: hand-set fallback - Wardryx resolved via
 *    `WARDRYX_ADMIN_KEY` (`EnvSource::EnvFallback`) rather than a
 *    `taipan up`-minted key - a real gap the existing devkey zond does NOT
 *    catch (see [`wardryxKeylessAdminFinding`]'s doc comment).
 *
 * Every finding also carries a `state` distinguishing "checked and clean"
 * (`ok`) from "checked and it fired" (`triggered`) from "cannot tell yet"
 * (`unknown` - e.g. the relevant backend has not finished bootstrapping, or
 * this console has not read the identity plane yet). PHASE2.md's own
 * wording ("a read-only list of stack-sanity findings") reads as a punch
 * list, but always rendering the full checklist with an honest
 * `unknown`/`ok`/`triggered` state per row is both what the Wave-3 parity
 * checklist asks for ("Posture panel shows the 4 v0 zonds") and more useful
 * in practice - the whole point of a posture board is seeing what currently
 * CANNOT be verified, not just what has already gone wrong.
 */

export type FindingState = "ok" | "triggered" | "unknown";

export interface PostureFinding {
  id:
    | "devkey"
    | "governance_fail_open"
    | "schema_mix"
    | "bus_stale"
    | "idryx_exposed"
    | "attestation_coverage"
    | "identity_snapshot_age"
    | "detector_feed_freshness"
    | "wardryx_keyless_admin";
  title: string;
  /** The severity IF `state === "triggered"` - fixed per zond, straight
   * from PHASE2.md/PHASE3.md (devkey/governance/idryx_exposed/
   * wardryx_keyless_admin high, schema-mix/identity_snapshot_age info,
   * bus-stale/attestation_coverage/detector_feed_freshness medium); a
   * non-triggered finding renders its own calm `ok`/`unknown` badge instead
   * (see `PostureView.tsx`), never this severity. */
  severity: Severity;
  state: FindingState;
  whyItMatters: string;
  /** A concrete command / env var, worded to match PHASE2.md/PHASE3.md's own
   * zond lists as closely as possible rather than paraphrased. */
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

  // ---- Phase-3 W4 additions: the identity-plane zonds' own inputs
  // (docs/PHASE3.md position 6). Same status hook (`useIdentityStatus`) and
  // fetchers (`fetchIdentities`/`fetchAlerts`) the Identity panel/Agent 360
  // already use - `PostureView.tsx` makes its OWN independent fetch rather
  // than reaching into another view's state, the same "each view owns its
  // own reads" convention `Agent360.tsx` already follows for this exact
  // data. ----

  /** Whole-panel identity connection state. */
  identityStatus: IdentityStatus | null;
  /** `null` until PostureView's own `identity_list_identities` read has
   * resolved at least once (only ever attempted once
   * `identityStatus.state === "ready"`). */
  identities: IdryxIdentity[] | null;
  /** `null` until PostureView's own `identity_list_alerts` read has resolved
   * at least once, same gating as `identities`. */
  identityAlerts: IdryxAlert[] | null;
  /** Epoch-ms of the moment PostureView's own identities+alerts fetch last
   * resolved - NOT idryx's actual process-start time, which the console has
   * no way to observe (see [`identitySnapshotAgeFinding`]'s doc comment).
   * `null` until that fetch has resolved at least once. */
  identitySnapshotAsOfMs: number | null;
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

/** "Flag the identity snapshot as aging" threshold for the
 * `identity_snapshot_age` zond - idryx `serve` never refreshes itself
 * (docs/PHASE3.md's grounded contract: "load-once... Polling /api/* returns
 * byte-identical data for the process lifetime"), so past this many ms
 * since this console's own last read, it is worth reminding the operator
 * they may be looking at a while-old picture. Purely informational
 * (severity `info`), not a defect - unlike `STALE_THRESHOLD_MS` this is not
 * "something is wrong", just "here is how old this is". */
const IDENTITY_SNAPSHOT_STALE_MS = 5 * 60_000;

/** "No fresh detector output" threshold for `detector_feed_freshness` -
 * deliberately much longer than `STALE_THRESHOLD_MS`: bus events are
 * continuous, but detector alerts are only ever produced at idryx startup
 * or an explicit Rescan, so a bus-like 60s bar would false-positive on
 * every idle demo box with nothing new to flag. */
const DETECTOR_FEED_STALE_MS = 15 * 60_000;

/** Loopback hostnames for the `idryx_exposed` zond. `new URL(...).hostname`
 * already lowercases and, for a bracketed IPv6 literal, keeps the brackets
 * (`"[::1]"`), so both bracketed and bare forms are listed defensively.
 * `0.0.0.0` (idryx's real documented default bind address, per the grounded
 * contract) is deliberately NOT here: as a connect target it means "every
 * interface", the opposite of loopback-only. */
const LOOPBACK_HOSTNAMES: ReadonlySet<string> = new Set(["127.0.0.1", "localhost", "::1", "[::1]"]);

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

/** Parse a URL's hostname, lowercased, or `null` if `url` does not parse -
 * used only by [`idryxExposedFinding`], which treats a parse failure as
 * "cannot honestly assert either way" (`unknown`), never a guessed `ok`. */
function safeHostname(url: string): string | null {
  try {
    return new URL(url).hostname.toLowerCase();
  } catch {
    return null;
  }
}

/** idryx has no authentication on ANY route at all (docs/PHASE3.md's
 * grounded contract: "every serve route... is unauthenticated... the
 * connector sends no bearer, no signer") - so whether it is reachable
 * off-box is a real exposure question, not a hardening nice-to-have. If the
 * discovered `idryx_url`'s host is not loopback, anyone who can reach that
 * address can read every identity, alert, and remediation with zero
 * credentials. An unparseable URL stays `unknown` rather than guessed `ok`. */
function idryxExposedFinding(input: PostureInput): PostureFinding {
  let state: FindingState = "unknown";
  if (input.identityStatus?.state === "ready") {
    const host = safeHostname(input.identityStatus.idryx_url);
    if (host !== null) {
      state = LOOPBACK_HOSTNAMES.has(host) ? "ok" : "triggered";
    }
  }
  return {
    id: "idryx_exposed",
    title: "idryx exposed off-box",
    severity: "high",
    state,
    whyItMatters:
      "idryx has no authentication on any route - if the discovered idryx URL is not loopback, anyone who can reach it can read every identity, alert, and remediation with zero credentials.",
    howToFix:
      "Bind idryx to loopback (127.0.0.1) or reach it only through an authenticated tunnel - taipan up already remaps it to 127.0.0.1:8081 for you.",
  };
}

/** Of the privileged identities in the current snapshot, how many carry an
 * `attestation_missing`/`bom_incomplete` alert - the same two detectors
 * `ATTESTATION_DETECTORS` names for the Identity panel/Agent 360, joined
 * here on `identity` the same way those views join `identityAlerts` to
 * `identities`. `0` privileged identities is a vacuous pass (`ok`), not
 * `unknown` - there is genuinely nothing to attest. */
function attestationCoverageFinding(input: PostureInput): PostureFinding {
  let state: FindingState = "unknown";
  let detail = "privileged identities and their attestation status are not loaded yet.";
  if (input.identityStatus?.state === "ready" && input.identities !== null && input.identityAlerts !== null) {
    const privileged = input.identities.filter((i) => i.privileged);
    const flaggedIds = new Set(
      input.identityAlerts.filter((a) => ATTESTATION_DETECTORS.has(a.detector)).map((a) => a.identity),
    );
    const flagged = privileged.filter((i) => flaggedIds.has(i.id));
    state = flagged.length > 0 ? "triggered" : "ok";
    detail =
      privileged.length === 0
        ? "no privileged identities in the current idryx snapshot."
        : `${flagged.length} of ${privileged.length} privileged identities carry an attestation_missing/bom_incomplete alert.`;
  }
  return {
    id: "attestation_coverage",
    title: "Attestation coverage: privileged identities",
    severity: "medium",
    state,
    whyItMatters: `Attestation is not a clean field on an identity (idryx has none) - it surfaces only via attestation_missing/bom_incomplete alerts. Currently: ${detail}`,
    howToFix: "Attest those agents (OIDC / SPIFFE-SVID / enclave-key / mTLS), then Rescan to confirm the alert clears.",
  };
}

/** How old is the identity data this console is showing. Deliberately NOT
 * "how long has idryx been running": `serve` is load-once (docs/PHASE3.md's
 * grounded contract), but the console has no field that reports idryx's own
 * process-start time, only when IT last successfully read the snapshot
 * (`identitySnapshotAsOfMs`, stamped by `PostureView.tsx` the moment its own
 * fetch resolves - see that component). That is still an honest, useful
 * signal ("you are looking at data confirmed this many minutes ago"), just
 * a different claim than "idryx's snapshot is N minutes old" would be. */
function identitySnapshotAgeFinding(input: PostureInput): PostureFinding {
  let state: FindingState = "unknown";
  let ageLabel = "not read yet.";
  if (input.identitySnapshotAsOfMs !== null) {
    const ageMs = Math.max(0, input.nowMs - input.identitySnapshotAsOfMs);
    state = ageMs > IDENTITY_SNAPSHOT_STALE_MS ? "triggered" : "ok";
    ageLabel = `last confirmed ${formatAgeShort(ageMs)} ago.`;
  }
  return {
    id: "identity_snapshot_age",
    title: "Identity snapshot age",
    severity: "info",
    state,
    whyItMatters: `idryx serve loads once and never reloads on its own, so this console's identity/alert data is "as of load", not live - ${ageLabel}`,
    howToFix: "Rescan (recomputes alerts only) or restart idryx (reloads identities too) for a fresher picture.",
  };
}

/** How long since idryx's 21 detectors last flagged anything at all, vs
 * now - a large gap can mean either "nothing anomalous has happened" (fine)
 * or "idryx has not been asked to look at recent bus activity" (worth a
 * Rescan), which is exactly why this is `info`/`medium` rather than a hard
 * failure: it is a prompt to check, not proof of a problem. Zero alerts
 * ever recorded is treated as `triggered` too (an empty feed cannot itself
 * be called "fresh"), not silently folded into `unknown`. */
function detectorFeedFreshnessFinding(input: PostureInput): PostureFinding {
  let state: FindingState = "unknown";
  let detail = "no alerts loaded yet.";
  if (input.identityStatus?.state === "ready" && input.identityAlerts !== null) {
    if (input.identityAlerts.length === 0) {
      state = "triggered";
      detail = "no detector alerts recorded at all yet.";
    } else {
      const times = input.identityAlerts.map((a) => Date.parse(a.time)).filter((ms) => Number.isFinite(ms));
      if (times.length === 0) {
        detail = "every recorded alert has an unparseable timestamp.";
      } else {
        const ageMs = Math.max(0, input.nowMs - Math.max(...times));
        state = ageMs > DETECTOR_FEED_STALE_MS ? "triggered" : "ok";
        detail = `newest alert ${formatAgeShort(ageMs)} ago.`;
      }
    }
  }
  return {
    id: "detector_feed_freshness",
    title: "Detector-feed freshness",
    severity: "medium",
    state,
    whyItMatters: `How long since idryx's 21 detectors last flagged anything, vs now - ${detail}`,
    howToFix: "Run traffic through the stack, or Rescan, so the detectors have something current to analyze.",
  };
}

/** Wardryx's admin API is bearer-only with no devkey concept of its own
 * (unlike TokenFuse Cloud's `ALLOW_DEVKEY`) - so the existing `devkeyFinding`
 * above, which only checks `org_domain === "default"`, structurally CANNOT
 * catch a hand-configured Wardryx: `EnvSource::EnvFallback` resolves
 * Wardryx's `org_domain` to the fixed `"wardryx.local"`
 * (`policy/state.rs::org_domain_for`), which never equals `"default"`. The
 * real, already-observable signal for "this admin key did not come through
 * the trusted taipan-mint pipeline" is `PolicyStatus.source.source ===
 * "env_fallback"` itself - a genuinely different, previously-uncaught gap,
 * not a duplicate of the devkey zond. */
function wardryxKeylessAdminFinding(input: PostureInput): PostureFinding {
  let state: FindingState = "unknown";
  if (input.policyStatus?.state === "ready") {
    state = input.policyStatus.source.source === "env_fallback" ? "triggered" : "ok";
  }
  return {
    id: "wardryx_keyless_admin",
    title: "Wardryx admin key: hand-set fallback",
    severity: "high",
    state,
    whyItMatters:
      'Wardryx was reached via a hand-set WARDRYX_ADMIN_KEY (env fallback), not a taipan up-minted, per-environment key from the keyfile - this is true even when org_domain never equals "default" for Wardryx, so it is not the same signal the devkey zond above already checks.',
    howToFix:
      "Bring wardryx up via taipan up --with wardryx (mints and journals a real per-environment admin key), or replace WARDRYX_ADMIN_KEY with a dedicated, non-shared secret.",
  };
}

/** Compact relative-age label ("42s"/"7m"/"3h") for the two age-based
 * zonds above - deliberately coarse (no exact seconds past the first
 * rung), since the point is "roughly how stale", not a precise clock. */
function formatAgeShort(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.round(m / 60);
  return `${h}h`;
}

/** Posture-lite's 4 v0 zonds, in PHASE2.md's own order. Pure and total:
 * never throws, always returns exactly 4 findings regardless of how
 * incomplete `input` currently is (an unresolved backend just yields more
 * `unknown` states, never a missing row or a crash). */
export function computeStackPostureFindings(input: PostureInput): PostureFinding[] {
  return [devkeyFinding(input), governanceFinding(input), schemaMixFinding(input), busStaleFinding(input)];
}

/** Posture full's 5 identity-plane zonds (docs/PHASE3.md W4, position 6),
 * same purity/totality guarantee as [`computeStackPostureFindings`]. Kept as
 * its own function (rather than folded into one big list) so
 * `PostureView.tsx` can render the two groups under separate section
 * headers without slicing a combined array by a fragile fixed index. */
export function computeIdentityPostureFindings(input: PostureInput): PostureFinding[] {
  return [
    idryxExposedFinding(input),
    attestationCoverageFinding(input),
    identitySnapshotAgeFinding(input),
    detectorFeedFreshnessFinding(input),
    wardryxKeylessAdminFinding(input),
  ];
}

/** All 9 zonds together, Posture-lite's 4 then the identity-plane 5, in one
 * fixed order - for any caller that just wants "the whole board" as a
 * single list. `PostureView.tsx` itself calls the two halves above
 * separately (see that component). */
export function computePostureFindings(input: PostureInput): PostureFinding[] {
  return [...computeStackPostureFindings(input), ...computeIdentityPostureFindings(input)];
}
