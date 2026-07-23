import type { IdentityStatus, IdryxAlert, IdryxIdentity } from "../identityTypes";
import { ATTESTATION_DETECTORS } from "../identityTypes";
import type { CopilotStatus } from "../copilotTypes";
import type { CredentialsStatus, GatewayKeysReport } from "./credentials";
import type { CryptoStatus } from "../cryptoTypes";
import type { DrillsStatus } from "../drillsTypes";
import type { MemoryStatus } from "../memoryTypes";
import type { MoneyStatus, Run } from "../moneyTypes";
import type { Approval, PolicyRecord, PolicyStatus } from "../policyTypes";
import type { QualityStatus } from "../qualityTypes";
import type { RemoteStatus } from "../remoteTypes";
import type { Severity } from "../types";

/**
 * Posture-lite (docs/PHASE2.md Wave 3) + Posture full's identity-plane
 * zonds (docs/PHASE3.md W4, position 6) + the I3 connection/credential
 * health group below: the pure signals-in/findings-out core of the Posture
 * view, kept free of React/Tauri so it is easy to reason about (and, if
 * ever needed, test) in isolation - `lib/usePostureData.ts` owns all the
 * data-fetching (money/policy/identity/quality/crypto/memory/drills/
 * copilot/remote status, policies, approvals, money runs, identities,
 * alerts, bus signals) and just calls
 * [`computeStackPostureFindings`]/[`computeIdentityPostureFindings`]/
 * [`computeConnectionHealthFindings`] with what it currently has;
 * `PostureView.tsx` and `OverviewView.tsx`'s Incident Center (I2,
 * `lib/incidents.ts`) both consume that one hook's output rather than each
 * re-deriving their own.
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
 * I3's "Connection & credential health" group (itrat-console/15-adjacent,
 * shipped alongside I2's Incident Center) adds 11 more zonds, same shape,
 * computed purely from `*_status` reads this shell already performs for its
 * other panels (money/policy/identity/quality/crypto/memory/drills/copilot/
 * remote - see [`computeConnectionHealthFindings`]) plus two staleness
 * checks over data already read elsewhere (`money_runs`,
 * `policy_list_approvals`). This group is explicitly NOT live probing: it
 * never opens a socket or calls a provider itself, it only reads the
 * console's own already-resolved status objects - see
 * [`computeConnectionHealthFindings`]'s own doc comment for the full list of
 * non-goals (no provider-key validity checks, no MCP secret-handle / JWKS
 * checks).
 *
 * I15 "key lifecycle health" adds 2 more zonds to the SAME
 * [`computeConnectionHealthFindings`] group: `credentials_plane_health`
 * (mirrors the 9 per-plane health rows above exactly, for the Credentials
 * card's own gateway connection) and `key_hygiene` (triggered by the
 * gateway's own key-lifecycle report - dangling/unbound keys, or
 * unauthorized attempts since startup). Both are fed by
 * `lib/usePostureData.ts`'s own `credentials_status`/`credentials_keys`
 * reads, on the same interval pattern the `cloud_ingest_freshness` zond's
 * `money_runs` read already uses.
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
    | "wardryx_keyless_admin"
    // I3 "Connection & credential health" group (see
    // `computeConnectionHealthFindings`): one row per `*_status` plane this
    // shell already calls, plus two staleness checks.
    | "money_plane_health"
    | "policy_plane_health"
    | "identity_plane_health"
    | "quality_plane_health"
    | "crypto_plane_health"
    | "memory_plane_health"
    | "drills_plane_health"
    | "copilot_plane_health"
    | "remote_plane_health"
    | "cloud_ingest_freshness"
    | "approvals_waiting"
    // I15 "key lifecycle health": the Credentials card's own plane-health row
    // plus the gateway key-report zond - see this file's module doc comment.
    | "credentials_plane_health"
    | "key_hygiene";
  title: string;
  /** The severity IF `state === "triggered"` - fixed per zond, straight
   * from PHASE2.md/PHASE3.md (devkey/governance/idryx_exposed/
   * wardryx_keyless_admin high, schema-mix/identity_snapshot_age info,
   * bus-stale/attestation_coverage/detector_feed_freshness medium); a
   * non-triggered finding renders its own calm `ok`/`unknown` badge instead
   * (see `PostureView.tsx`), never this severity. I3's own 11 zonds (and
   * I15's 2 more, `credentials_plane_health`/`key_hygiene`) follow the same
   * fixed-per-zond rule: `policy_plane_health` is high (fail-open class,
   * mirroring `governance_fail_open`/`wardryx_keyless_admin` - see that
   * finding's own doc comment), every other new zond (I15's pair included)
   * is medium - a visibility/operational or hygiene gap, not itself a
   * governance fail-open. */
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

  // ---- I3 additions: the "Connection & credential health" group's own
  // inputs. Six more `*_status` reads this shell already performs for their
  // own panels (`useQualityStatus`/`useCryptoStatus`/`useMemoryStatus`/
  // `useDrillsStatus`/`useRemoteStatus`, plus a one-shot `copilot_status`
  // fetch - mirrors `CopilotView.tsx`'s own inline fetch, there being no
  // dedicated hook for a DTO with no bootstrapping/polling shape to begin
  // with, see `copilotTypes.ts`), plus two more reads other panels already
  // make (`money_runs`, `policy_list_approvals`) for the two staleness
  // checks. All gathered by `lib/usePostureData.ts`. ----

  qualityStatus: QualityStatus | null;
  cryptoStatus: CryptoStatus | null;
  memoryStatus: MemoryStatus | null;
  drillsStatus: DrillsStatus | null;
  /** Flat DTO, not a `state`-tagged union (see `copilotTypes.ts`) - `null`
   * only until the one-shot `copilot_status` fetch first resolves. */
  copilotStatus: CopilotStatus | null;
  remoteStatus: RemoteStatus | null;
  /** `null` until `lib/usePostureData.ts`'s own `money_runs` read has
   * resolved at least once (only ever attempted once
   * `moneyStatus.state === "ready"`) - feeds [`cloudIngestFreshnessFinding`]
   * only; every OTHER money-plane zond in this file keys off `moneyStatus`
   * alone. */
  moneyRuns: Run[] | null;
  /** `null` until `lib/usePostureData.ts`'s own `policy_list_approvals` read
   * has resolved at least once (only ever attempted once
   * `policyStatus.state === "ready"`) - feeds [`approvalsWaitingFinding`]
   * only. */
  approvals: Approval[] | null;

  // ---- I15 "key lifecycle health" additions: the Credentials card's own
  // status hook (`useCredentialsStatus`) plus a `credentials_keys` read on
  // the same interval pattern [`cloudIngestFreshnessFinding`]'s own
  // `moneyRuns` uses above - both gathered by `lib/usePostureData.ts`. ----

  /** Whole-panel Credentials/gateway connection state - feeds
   * [`credentialsPlaneHealthFinding`] only. */
  credentialsStatus: CredentialsStatus | null;
  /** `null` until `lib/usePostureData.ts`'s own `credentials_keys` read has
   * resolved at least once (only ever attempted once
   * `credentialsStatus.state === "ready"`) - feeds [`keyHygieneFinding`]
   * only. */
  keysReport: GatewayKeysReport | null;
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

// ---------------------------------------------------------------------------
// I3 "Connection & credential health" group: 9 per-plane health zonds (one
// row per `*_status` command this shell already calls elsewhere) + 2
// staleness checks over data already read elsewhere (`money_runs`,
// `policy_list_approvals`). I15 "key lifecycle health" adds a 10th
// per-plane row (`credentials_plane_health`) plus a 3rd report-content check
// (`key_hygiene`, over the gateway's own `credentials_keys` report) further
// below, right where each is computed. Same `unknown`/`ok`/`triggered`
// contract as every zond above - "configured but unreachable" is
// `triggered`, "not configured yet" is `unknown` (never a fabricated `ok`),
// matching this whole file's fail-closed doctrine.
// ---------------------------------------------------------------------------

/** "Cloud ingest gone stale" threshold for `cloud_ingest_freshness` - the I3
 * spec's own number. */
const CLOUD_INGEST_STALE_MS = 15 * 60_000;

/** "An approval has waited too long for a human" threshold for
 * `approvals_waiting` - the I3 spec's own number. */
const APPROVAL_WAIT_STALE_MS = 60 * 60_000;

function moneyPlaneHealthFinding(input: PostureInput): PostureFinding {
  const s = input.moneyStatus;
  let state: FindingState = "unknown";
  let detail = "not yet resolved.";
  if (s?.state === "ready") {
    state = "ok";
    detail = "reachable and ready.";
  } else if (s?.state === "pairing_failed") {
    state = "triggered";
    detail = `pairing failed: ${s.reason}`;
  } else if (s?.state === "no_environment") {
    detail = "no environment discovered (not configured).";
  }
  return {
    id: "money_plane_health",
    title: "Money plane (TokenFuse Cloud)",
    severity: "medium",
    state,
    whyItMatters:
      `Whether the console can currently reach the money plane it discovered (money_status) - ${detail} ` +
      "A console-side gap here is a visibility/control loss (no read on spend/runs/incidents, no kill/budget), not " +
      "itself a fail-open: the gateway still enforces independently of whether the console can see it.",
    howToFix:
      "Confirm the discovered cloud_url is reachable and the bearer is valid (taipan up, or TOKENFUSE_CLOUD_URL/TOKENFUSE_CLOUD_KEYS).",
  };
}

function policyPlaneHealthFinding(input: PostureInput): PostureFinding {
  const s = input.policyStatus;
  let state: FindingState = "unknown";
  let detail = "not yet resolved.";
  if (s?.state === "ready") {
    state = "ok";
    detail = "reachable and ready.";
  } else if (s?.state === "unreachable") {
    state = "triggered";
    detail = `unreachable: ${s.reason}`;
  } else if (s?.state === "no_environment") {
    detail = "no environment discovered (not configured).";
  }
  return {
    id: "policy_plane_health",
    title: "Policy plane (Wardryx)",
    severity: "high", // fail-open class - see this zond's own whyItMatters and PostureFinding.severity's doc comment.
    state,
    whyItMatters:
      `Whether the console can currently reach Wardryx (policy_status) - ${detail} Fail-open-class like this ` +
      "group's governance-fail-open/keyless-admin zonds above: while Wardryx is unreachable, approvals cannot be " +
      "listed or decided, and neither of those other two zonds can even run their own check blind.",
    howToFix:
      "Confirm Wardryx is up and the discovered wardryx_url/admin key are correct (taipan up --with wardryx, or WARDRYX_URL/WARDRYX_ADMIN_KEY).",
  };
}

function identityPlaneHealthFinding(input: PostureInput): PostureFinding {
  const s = input.identityStatus;
  let state: FindingState = "unknown";
  let detail = "not yet resolved.";
  if (s?.state === "ready") {
    state = "ok";
    detail = "reachable and ready.";
  } else if (s?.state === "unreachable") {
    state = "triggered";
    detail = `unreachable: ${s.reason}`;
  } else if (s?.state === "no_environment") {
    detail = "no environment discovered (not configured).";
  }
  return {
    id: "identity_plane_health",
    title: "Identity plane (idryx)",
    severity: "medium",
    state,
    whyItMatters: `Whether the console can currently reach idryx (identity_status) - ${detail}`,
    howToFix: "Run taipan up --with idryx, or confirm the discovered idryx_url is reachable.",
  };
}

function qualityPlaneHealthFinding(input: PostureInput): PostureFinding {
  const s = input.qualityStatus;
  let state: FindingState = "unknown";
  let detail = "not yet resolved.";
  if (s?.state === "ready") {
    state = "ok";
    detail = "reachable and ready.";
  } else if (s?.state === "unreachable") {
    state = "triggered";
    detail = `unreachable: ${s.reason}`;
  } else if (s?.state === "no_environment") {
    detail = "no environment discovered (not configured).";
  }
  return {
    id: "quality_plane_health",
    title: "Quality plane (Verdryx)",
    severity: "medium",
    state,
    whyItMatters: `Whether the console can currently open verdryx.db (quality_status) - ${detail}`,
    howToFix: "Park (or symlink) verdryx.db at ~/.taipan/verdryx.db, or confirm the discovered db_path is correct.",
  };
}

/** CryptoStatus has no `unreachable` variant at all - qryx is a CLI wrapper
 * with no serve process to health-check (see `cryptoTypes.ts`'s own doc
 * comment) - so this zond can structurally only ever read `ok` or
 * `unknown`, never `triggered`. Kept in the group anyway for one uniform
 * per-plane row rather than a silent gap in the checklist. */
function cryptoPlaneHealthFinding(input: PostureInput): PostureFinding {
  const s = input.cryptoStatus;
  let state: FindingState = "unknown";
  let detail = "not yet resolved.";
  if (s?.state === "ready") {
    state = "ok";
    detail = "ready.";
  } else if (s?.state === "no_environment") {
    detail = "no environment discovered (not configured).";
  }
  return {
    id: "crypto_plane_health",
    title: "Crypto plane (Qryx)",
    severity: "medium",
    state,
    whyItMatters:
      `Whether qryx is configured for this console (crypto_status) - ${detail} CryptoStatus carries no ` +
      '"unreachable" variant (qryx has no serve process to probe), so this row can only ever read ok or unknown.',
    howToFix: "Run taipan up (mints qryx_bin/default_target), or set the equivalent env vars this box discovers from.",
  };
}

function memoryPlaneHealthFinding(input: PostureInput): PostureFinding {
  const s = input.memoryStatus;
  let state: FindingState = "unknown";
  let detail = "not yet resolved.";
  if (s?.state === "ready") {
    state = "ok";
    detail = "reachable and ready.";
  } else if (s?.state === "unreachable") {
    state = "triggered";
    detail = `unreachable: ${s.reason}`;
  } else if (s?.state === "no_environment") {
    detail = "no environment discovered (not configured).";
  }
  return {
    id: "memory_plane_health",
    title: "Memory plane (Engram)",
    severity: "medium",
    state,
    whyItMatters: `Whether engram-mcp actually spawned and completed its handshake (memory_status) - ${detail}`,
    howToFix: "Confirm engram_mcp_bin resolves and db_path is writable, or re-run taipan up --with engram.",
  };
}

/** DrillsStatus, like CryptoStatus, has no `unreachable` variant - mockryx
 * has no serve process of its own to health-check either (see
 * `drillsTypes.ts`'s own doc comment) - so this zond likewise can only ever
 * read `ok` or `unknown`, never `triggered`. */
function drillsPlaneHealthFinding(input: PostureInput): PostureFinding {
  const s = input.drillsStatus;
  let state: FindingState = "unknown";
  let detail = "not yet resolved.";
  if (s?.state === "ready") {
    state = "ok";
    detail = "ready.";
  } else if (s?.state === "no_environment") {
    detail = "no environment discovered (not configured).";
  }
  return {
    id: "drills_plane_health",
    title: "Drills plane (Mockryx)",
    severity: "medium",
    state,
    whyItMatters:
      `Whether mockryx's gateway is configured for this console (drills_status) - ${detail} DrillsStatus ` +
      'likewise carries no "unreachable" variant, so this row can only ever read ok or unknown.',
    howToFix: "Run taipan up --with mockryx (mints gateway_url/has_api_key), or set the equivalent env vars.",
  };
}

/** CopilotStatus is a flat DTO with no reachability concept at all - no
 * `state`-tagged union, no environment to discover (see `copilotTypes.ts`'s
 * own doc comment) - so "disabled" is read as `unknown` (not configured),
 * never a fabricated `triggered`: a disabled copilot is very commonly a
 * DELIBERATE choice (no BYO model connected yet), not a failure. Like
 * crypto/drills above, this zond can therefore never actually read
 * `triggered` either. */
function copilotPlaneHealthFinding(input: PostureInput): PostureFinding {
  const s = input.copilotStatus;
  let state: FindingState = "unknown";
  let detail = "not yet resolved.";
  if (s) {
    if (s.enabled) {
      state = "ok";
      detail = `enabled (${s.provider ?? "unknown provider"}${s.local ? ", local" : ""}).`;
    } else {
      detail = `disabled${s.disabled_reason ? `: ${s.disabled_reason}` : "."}`;
    }
  }
  return {
    id: "copilot_plane_health",
    title: "Copilot plane (Felyx)",
    severity: "medium",
    state,
    whyItMatters:
      `Whether a copilot provider is connected (copilot_status) - ${detail} CopilotStatus has no reachability ` +
      "concept at all, so this row can only ever read ok or unknown, never triggered.",
    howToFix: "Open the Copilot tab and Connect Felyx (Anthropic, OpenAI, OpenRouter, Ollama, or LM Studio) if you want it enabled.",
  };
}

/** Unlike the two CLI-wrapper planes above, Remote genuinely CAN be
 * "configured but unreachable": a saved environment whose WireGuard
 * bring-up failed. `disconnected`/`connecting` are normal not-yet-tried
 * states (`ok`), never `triggered` - only a DURABLE `failed` record counts
 * (see `remoteTypes.ts`'s own doc comment: "never silently reverts to
 * disconnected"). */
function remotePlaneHealthFinding(input: PostureInput): PostureFinding {
  const s = input.remoteStatus;
  let state: FindingState = "unknown";
  let detail = "not yet resolved.";
  if (s?.state === "ready") {
    if (s.environment === null) {
      detail = "no remote environment saved yet (not configured).";
    } else if (s.tunnel.state === "failed") {
      state = "triggered";
      detail = `tunnel bring-up failed: ${s.tunnel.message}`;
    } else {
      state = "ok";
      detail = `environment "${s.environment.name}" saved, tunnel ${s.tunnel.state}.`;
    }
  }
  return {
    id: "remote_plane_health",
    title: "Remote plane (Distance: WireGuard + SSH)",
    severity: "medium",
    state,
    whyItMatters: `Whether a saved remote environment's WireGuard tunnel is actually up (remote_status) - ${detail}`,
    howToFix: "Reconnect from the Remote tab, and check the peer/endpoint/keys saved in the environment form.",
  };
}

/** Credentials plane's own console-to-gateway connection (I15 "key
 * lifecycle health") - mirrors every per-plane health zond above exactly,
 * same three-way split: `ready` is `ok`, `unreachable` is `triggered`, no
 * environment discovered is `unknown` (never a fabricated `ok`). Kept as its
 * own row rather than folded into `identityPlaneHealthFinding` just because
 * both render in the same Identity tab: the Credentials card resolves a
 * genuinely separate descriptor service (`services.gateway`, not
 * `services.idryx` - see `genaryx_api::credentials`'s module doc), and can
 * be `ready` while Identity is `no_environment` or vice versa. Medium, not
 * high: same rationale as `moneyPlaneHealthFinding`'s doc comment - a
 * console-side gap here is a visibility loss (no read on key/unauthorized-
 * attempt activity), not itself a fail-open. The gateway keeps enforcing
 * `strict_mode` independently of whether this console can currently see it. */
function credentialsPlaneHealthFinding(input: PostureInput): PostureFinding {
  const s = input.credentialsStatus;
  let state: FindingState = "unknown";
  let detail = "not yet resolved.";
  if (s?.state === "ready") {
    state = "ok";
    detail = "reachable and ready.";
  } else if (s?.state === "unreachable") {
    state = "triggered";
    detail = `unreachable: ${s.reason}`;
  } else if (s?.state === "no_environment") {
    detail = "no environment discovered (not configured).";
  }
  return {
    id: "credentials_plane_health",
    title: "Credentials plane (TokenFuse gateway)",
    severity: "medium",
    state,
    whyItMatters: `Whether the console can currently reach the gateway's key-lifecycle report (credentials_status) - ${detail}`,
    howToFix: "Confirm the discovered services.gateway.url is reachable (taipan up), and that the gateway process is actually running.",
  };
}

/** Whether TokenFuse Cloud is still receiving fresh run activity, judged
 * from `money_runs`' own `last_seen` (the same field `spendSeries` buckets
 * for the Money/Overview hero sparkline) - the in-console productization of
 * the `genaryx-web doctor` idea (I3 spec). An EMPTY run list is left
 * `unknown` rather than `triggered`: the I3 spec's own condition is "runs
 * exist BUT the newest is stale", and with zero runs there is genuinely
 * nothing to call fresh or stale (could mean "no traffic yet" just as
 * easily as "ingest broken") - a different, deliberately more cautious
 * call than `bus_stale` above, which DOES treat an empty bus feed as
 * `triggered`; the two zonds read different data with different priors, so
 * they need not agree. */
function cloudIngestFreshnessFinding(input: PostureInput): PostureFinding {
  let state: FindingState = "unknown";
  let detail = "no run data loaded yet.";
  if (input.moneyRuns !== null) {
    if (input.moneyRuns.length === 0) {
      detail = "no runs in the current window.";
    } else {
      const times = input.moneyRuns.map((r) => Date.parse(r.last_seen)).filter((ms) => Number.isFinite(ms));
      if (times.length === 0) {
        detail = "every run has an unparseable last_seen.";
      } else {
        const ageMs = Math.max(0, input.nowMs - Math.max(...times));
        state = ageMs > CLOUD_INGEST_STALE_MS ? "triggered" : "ok";
        detail = `newest run last_seen ${formatAgeShort(ageMs)} ago.`;
      }
    }
  }
  return {
    id: "cloud_ingest_freshness",
    title: "Cloud ingest freshness",
    severity: "medium",
    state,
    whyItMatters: `Whether TokenFuse Cloud is still receiving fresh run activity, judged from money_runs' own last_seen - ${detail}`,
    howToFix: "Check the gateway is still forwarding traffic to Cloud, and that this box's discovered cloud_url is still correct.",
  };
}

/** How long the oldest PENDING approval (`policy_list_approvals`) has
 * waited for a human decision. Zero pending approvals is a vacuous, clean
 * `ok` (mirrors `attestationCoverageFinding`'s identical "0 privileged
 * identities is a vacuous pass, not unknown" precedent above) - unlike
 * [`cloudIngestFreshnessFinding`], "the list is empty" is unambiguous here:
 * nothing is waiting, which is unambiguously fine, not merely "no data". */
function approvalsWaitingFinding(input: PostureInput): PostureFinding {
  let state: FindingState = "unknown";
  let detail = "no approvals data loaded yet.";
  if (input.approvals !== null) {
    const pending = input.approvals.filter((a) => a.pending);
    if (pending.length === 0) {
      state = "ok";
      detail = "no approvals currently pending.";
    } else {
      const times = pending.map((a) => Date.parse(a.requested_at)).filter((ms) => Number.isFinite(ms));
      if (times.length === 0) {
        detail = "every pending approval has an unparseable requested_at.";
      } else {
        const ageMs = Math.max(0, input.nowMs - Math.min(...times));
        state = ageMs > APPROVAL_WAIT_STALE_MS ? "triggered" : "ok";
        detail = `oldest pending approval requested ${formatAgeShort(ageMs)} ago.`;
      }
    }
  }
  return {
    id: "approvals_waiting",
    title: "Approvals waiting",
    severity: "medium",
    state,
    whyItMatters: `How long the oldest pending approval has waited for a human decision (policy_list_approvals) - ${detail}`,
    howToFix: "Open the Policy panel's Approvals inbox and grant or deny the oldest pending request.",
  };
}

/** Gateway key hygiene (I15 "key lifecycle health" spec, verbatim
 * precedence): triggered when the latest `GatewayKeysReport` shows a
 * dangling key (bound but not configured - a map entry with no live secret
 * behind it anymore), OR the identity map is NOT in `"off"` mode and at
 * least one key is unbound (configured but not bound, while
 * `identity_map_configured` is true - mirrors `lib/credentials.ts`'s
 * `deriveKeyStatus` doc comment: "never fires when the map itself is off,
 * there is nothing to be unbound FROM"), OR any unauthorized attempt has
 * been recorded against the gateway since it started. `unknown` until a
 * report has loaded at least once - never a fabricated `ok` on absent data,
 * matching this whole file's fail-closed doctrine.
 *
 * Medium, not high: this is a hygiene/coverage check over a list (the same
 * bucket `attestationCoverageFinding` and the two staleness checks just
 * above sit in), not a structural fail-open like `governance_fail_open`/
 * `idryx_exposed`/`wardryx_keyless_admin` (all "zero authentication or zero
 * policy required at all", a categorically different, always-high class).
 * A single stale dangling map entry and an active credential-stuffing
 * attempt both trip this same zond; medium keeps either from over- or
 * under-stating the other. */
function keyHygieneFinding(input: PostureInput): PostureFinding {
  const report = input.keysReport;
  let state: FindingState = "unknown";
  let detail = "no key-lifecycle report loaded yet.";
  if (report !== null) {
    const dangling = report.keys.filter((k) => k.bound && !k.configured).length;
    const unbound = report.identity_map_configured
      ? report.keys.filter((k) => k.configured && !k.bound).length
      : 0;
    const strictOn = report.strict_mode !== "off";
    const unauthorized = report.unauthorized_since_startup.attempts;
    const triggered = dangling > 0 || (strictOn && unbound > 0) || unauthorized > 0;
    state = triggered ? "triggered" : "ok";
    const parts: string[] = [];
    if (dangling > 0) parts.push(`${dangling} dangling`);
    if (strictOn && unbound > 0) parts.push(`${unbound} unbound (strict_mode=${report.strict_mode})`);
    if (unauthorized > 0) parts.push(`${unauthorized} unauthorized attempt${unauthorized === 1 ? "" : "s"} since startup`);
    detail = parts.length > 0 ? `${parts.join(", ")}.` : "no dangling or unbound keys, no unauthorized attempts.";
  }
  return {
    id: "key_hygiene",
    title: "Gateway key hygiene",
    severity: "medium",
    state,
    whyItMatters: `Whether the gateway's key-lifecycle report (credentials_keys) shows a dangling or unbound client key, or any unauthorized attempt since startup - ${detail}`,
    howToFix:
      "Edit TOKENFUSE_CLIENT_KEYS and the identity map together (remove the dangling entry, or add the missing binding), then restart the gateway so it picks up the change.",
  };
}

/** I3's original 11-zond "Connection & credential health" group plus I15's
 * 2 more, 13 total, in the order listed above: the 10 per-plane health rows
 * (money/policy/identity/quality/crypto/memory/drills/copilot/remote/
 * credentials, in that fixed order) then the 3 report-content checks
 * (cloud ingest freshness, approvals waiting, gateway key hygiene). Same
 * purity/totality guarantee as
 * [`computeStackPostureFindings`]/[`computeIdentityPostureFindings`]: never
 * throws, always returns exactly 13 findings.
 *
 * Explicit non-goals (I3 spec, still true after I15's addition): this group
 * never opens a socket, calls a provider, or shells a binary itself - every
 * row above reads only a `*_status`/list command this shell ALREADY calls
 * for its own panels, at whatever cadence `lib/usePostureData.ts` already
 * reads them. It specifically does NOT: probe connectivity live from the
 * browser/console process; validate provider-key VALIDITY (TokenFuse stores
 * no provider keys by design - not a gap, a deliberate privacy feature, so
 * there is no key here to check in the first place); or check MCP
 * secret-handle freshness or JWKS age (both need a gateway/cloud surface
 * this console does not have a read on today - named follow-ups, not
 * silently skipped). */
export function computeConnectionHealthFindings(input: PostureInput): PostureFinding[] {
  return [
    moneyPlaneHealthFinding(input),
    policyPlaneHealthFinding(input),
    identityPlaneHealthFinding(input),
    qualityPlaneHealthFinding(input),
    cryptoPlaneHealthFinding(input),
    memoryPlaneHealthFinding(input),
    drillsPlaneHealthFinding(input),
    copilotPlaneHealthFinding(input),
    remotePlaneHealthFinding(input),
    credentialsPlaneHealthFinding(input),
    cloudIngestFreshnessFinding(input),
    approvalsWaitingFinding(input),
    keyHygieneFinding(input),
  ];
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

/** All 20 zonds together, Posture-lite's 4, then the identity-plane 5, then
 * I3's connection/credential-health 11, in one fixed order - for any caller
 * that just wants "the whole board" as a single list (e.g. `lib/incidents.ts`'s
 * Incident Center aggregation, which only cares which findings are
 * `triggered`, not which of the three groups they came from).
 * `PostureView.tsx` itself calls the three groups above separately (see that
 * component), so it can render each under its own subheading rather than
 * slicing this combined array by a fragile fixed index. */
export function computePostureFindings(input: PostureInput): PostureFinding[] {
  return [
    ...computeStackPostureFindings(input),
    ...computeIdentityPostureFindings(input),
    ...computeConnectionHealthFindings(input),
  ];
}
