/**
 * Credentials wire types + fetch helpers + pure key-status derivation (I15
 * "key lifecycle health"). Unlike `identityTypes.ts` + `lib/identity.ts`
 * (types and fetchers split into two files), this plane keeps both together
 * in one file per the architect's spec - the DTOs are small enough, and
 * `deriveKeyStatus` below is the one piece of real logic this module owns,
 * so there is little to gain from a second file.
 *
 * Mirrors `lib/identity.ts`'s conventions: types mirror the Rust DTOs
 * field-for-field (`crates/api/src/credentials/commands.rs`), including the
 * exact serde tag/rename_all shape of every enum; fetchers go through
 * `invokeBackend` and never throw a raw transport error (each normalizes to
 * a `CredentialsError`); `fetchCredentialsStatus` never throws at all,
 * mirroring `fetchIdentityStatus`'s identical "always renderable" contract.
 */
import { hasBackend, invokeBackend } from "./transport";

// ============================================================================
// Wire types (mirrors crates/api/src/credentials/{env,commands}.rs and
// crates/connectors/src/gateway.rs field-for-field)
// ============================================================================

/** Mirrors `credentials::env::EnvSource` (`#[serde(tag = "source", rename_all = "snake_case")]`). */
export type EnvSource = { source: "taipan"; name: string };

/** Mirrors `credentials::commands::CredentialsStatusDto`
 * (`#[serde(tag = "state", rename_all = "snake_case")]`). */
export type CredentialsStatus =
  | { state: "bootstrapping" }
  | { state: "no_environment" }
  | { state: "unreachable"; source: EnvSource; gateway_url: string; reason: string }
  | { state: "ready"; source: EnvSource; gateway_url: string };

/** Mirrors `credentials::commands::CredentialsError` (`#[serde(tag = "kind", rename_all = "snake_case")]`). */
export type CredentialsError =
  | { kind: "bootstrapping" }
  | { kind: "no_environment" }
  | { kind: "unreachable"; reason: string }
  | { kind: "gateway"; status: number | null; message: string };

/** Mirrors `genaryx_connectors::GatewayUnauthorized`. */
export interface GatewayUnauthorized {
  attempts: number;
  last_millis: number | null;
}

/** Mirrors `genaryx_connectors::GatewayKeyStats` - shared shape for
 * `since_startup` (never carries `first_seen_millis` on the real wire) and
 * `history`. */
export interface GatewayKeyStats {
  calls: number;
  identity_mismatches: number;
  first_seen_millis: number | null;
  last_seen_millis: number | null;
}

/** Mirrors `genaryx_connectors::GatewayKeyEntry`. */
export interface GatewayKeyEntry {
  key_id: string;
  configured: boolean;
  bound: boolean;
  unit: string | null;
  agents: string[];
  /** `"YYYY-MM-DD"` when the onboard wizard stamped one, else `null`. */
  created: string | null;
  since_startup: GatewayKeyStats;
  history: GatewayKeyStats | null;
}

/** Mirrors `genaryx_connectors::GatewayKeysReport`, `GET /v1/keys`'s
 * top-level shape (docs/22-key-lifecycle.md in tokenfuse). */
export interface GatewayKeysReport {
  strict_mode: "off" | "warn" | "enforce" | (string & {});
  identity_map_configured: boolean;
  history_available: boolean;
  unauthorized_since_startup: GatewayUnauthorized;
  keys: GatewayKeyEntry[];
}

// ============================================================================
// Fetch helpers (mirrors lib/identity.ts's call()/toIdentityError() shape)
// ============================================================================

/** Thrown by every fetcher below when there is no backend to talk to (a
 * plain `vite build`/browser preview outside mock mode) - mirrors
 * `lib/identity.ts`'s identical `NO_ENVIRONMENT_ERROR` guard. */
const NO_ENVIRONMENT_ERROR: CredentialsError = { kind: "no_environment" };

function toCredentialsError(err: unknown): CredentialsError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as CredentialsError;
  }
  return {
    kind: "gateway",
    status: null,
    message: err instanceof Error ? err.message : String(err),
  };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!hasBackend()) throw NO_ENVIRONMENT_ERROR;
  try {
    return await invokeBackend<T>(command, args);
  } catch (err) {
    throw toCredentialsError(err);
  }
}

/** Whole-panel connection state. Never throws: outside a backend (or on any
 * IPC failure) it resolves to a renderable status instead - mirrors
 * `lib/identity.ts`'s `fetchIdentityStatus` exactly. */
export async function fetchCredentialsStatus(): Promise<CredentialsStatus> {
  if (!hasBackend()) return { state: "no_environment" };
  try {
    return await invokeBackend<CredentialsStatus>("credentials_status");
  } catch (err) {
    return {
      state: "unreachable",
      source: { source: "taipan", name: "" },
      gateway_url: "",
      reason: err instanceof Error ? err.message : String(err),
    };
  }
}

/** `GET /v1/keys` via the console's own command layer. */
export const fetchCredentialsKeys = (): Promise<GatewayKeysReport> =>
  call<GatewayKeysReport>("credentials_keys");

/** Human-readable text for any `CredentialsError` - mirrors
 * `lib/identity.ts`'s `describeIdentityError`. */
export function describeCredentialsError(err: CredentialsError): string {
  switch (err.kind) {
    case "bootstrapping":
      return "Still connecting to the gateway.";
    case "no_environment":
      return "No gateway found in this environment.";
    case "unreachable":
      return `Could not reach the gateway: ${err.reason}`;
    case "gateway":
      return err.status !== null ? `Gateway error ${err.status}: ${err.message}` : err.message;
  }
}

// ============================================================================
// Key status derivation (pure - no Date.now(), `nowMillis` is a parameter)
// ============================================================================

/** One key's lifecycle-health status, worst-first in this exact order - see
 * {@link deriveKeyStatus}'s doc comment for the precedence and
 * {@link KEY_STATUS_ORDER} for the shared array both the precedence check
 * and the table's sort reuse. */
export type KeyStatus =
  | "removed"
  | "dangling"
  | "unbound"
  | "mismatching"
  | "never-used"
  | "stale"
  | "active";

/** Worst-first order (I15 spec, verbatim): the precedence
 * {@link deriveKeyStatus} checks in, and the sort order
 * `CredentialsKeysTable` ranks rows by - one array, reused both places so the
 * two can never quietly disagree. */
export const KEY_STATUS_ORDER: readonly KeyStatus[] = [
  "removed",
  "dangling",
  "unbound",
  "mismatching",
  "never-used",
  "stale",
  "active",
];

const KEY_STATUS_RANK: Readonly<Record<KeyStatus, number>> = Object.fromEntries(
  KEY_STATUS_ORDER.map((s, i) => [s, i]),
) as Record<KeyStatus, number>;

/** Sort comparator: worst-first, by {@link KEY_STATUS_ORDER}. */
export function keyStatusRank(status: KeyStatus): number {
  return KEY_STATUS_RANK[status];
}

/** "Key issues" (I15 spec): the statuses that count toward the HeroBand's
 * KpiTile and the `key_hygiene` posture zond - the four worst entries of
 * {@link KEY_STATUS_ORDER}, never `never-used`/`stale`/`active` (those are
 * hygiene notes, not issues). */
const ISSUE_STATUSES: ReadonlySet<KeyStatus> = new Set([
  "removed",
  "dangling",
  "unbound",
  "mismatching",
]);

export function isKeyIssue(status: KeyStatus): boolean {
  return ISSUE_STATUSES.has(status);
}

const SEVEN_DAYS_MS = 7 * 24 * 60 * 60 * 1000;

/** The later of `since_startup.last_seen_millis`/`history.last_seen_millis`,
 * or `null` when neither key ever recorded one - shared by
 * {@link deriveKeyStatus}'s "stale" check and the table's "last seen" column
 * so the two never compute it differently. */
export function maxLastSeenMillis(entry: GatewayKeyEntry): number | null {
  const a = entry.since_startup.last_seen_millis;
  const b = entry.history?.last_seen_millis ?? null;
  if (a === null) return b;
  if (b === null) return a;
  return Math.max(a, b);
}

/**
 * Derive one key's lifecycle-health status. Pure and deterministic: no
 * `Date.now()` inside, `nowMillis` is a parameter (the caller ticks it on an
 * interval, same convention `lib/posture.ts`'s age-based zonds use).
 *
 * Precedence, checked in this EXACT order (I15 spec, verbatim) - the first
 * match wins, later checks never override an earlier one:
 *
 * 1. `removed`      - not configured, not bound, but carries history (a
 *                      fully decommissioned key with an audit trail).
 * 2. `dangling`      - bound (the identity map still references this
 *                      `key_id`) but not configured (the secret is gone) -
 *                      a map hygiene gap.
 * 3. `unbound`       - configured but not bound, AND the environment HAS an
 *                      identity map (`report.identity_map_configured`) - a
 *                      real key with no attribution. Never fires when the
 *                      map itself is off: there is nothing to be unbound
 *                      FROM.
 * 4. `mismatching`   - any `identity_mismatches` > 0, in either stats block,
 *                      regardless of `configured`/`bound` (a security
 *                      signal that can outlive a key's own configuration).
 * 5. `never-used`    - configured, and zero calls in both stats blocks.
 * 6. `stale`         - the later of the two `last_seen_millis` fields is
 *                      more than 7 days old.
 * 7. `active`        - none of the above.
 */
export function deriveKeyStatus(
  entry: GatewayKeyEntry,
  report: GatewayKeysReport,
  nowMillis: number,
): KeyStatus {
  if (!entry.configured && !entry.bound && entry.history !== null) return "removed";
  if (entry.bound && !entry.configured) return "dangling";
  if (entry.configured && !entry.bound && report.identity_map_configured) return "unbound";
  if (entry.since_startup.identity_mismatches > 0 || (entry.history?.identity_mismatches ?? 0) > 0) {
    return "mismatching";
  }
  if (entry.configured && entry.since_startup.calls === 0 && (entry.history?.calls ?? 0) === 0) {
    return "never-used";
  }
  const lastSeen = maxLastSeenMillis(entry);
  if (lastSeen !== null && nowMillis - lastSeen > SEVEN_DAYS_MS) return "stale";
  return "active";
}

/** Total calls across both stats blocks - the table's "calls" column. */
export function totalCalls(entry: GatewayKeyEntry): number {
  return entry.since_startup.calls + (entry.history?.calls ?? 0);
}

/** Compact relative-age label ("just now"/"5m ago"/"3h ago"/"2d ago") -
 * mirrors `lib/posture.ts`'s private `formatAgeShort` in spirit (deliberately
 * coarse, the point is "roughly how stale") but with its own wording, since
 * that helper is not exported and this module has no reason to import a
 * component-adjacent `lib/*` file just for a one-line formatter. */
export function humanizeAge(ms: number): string {
  const clamped = Math.max(0, ms);
  const s = Math.round(clamped / 1000);
  if (s < 45) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.round(h / 24);
  return `${d}d ago`;
}

/** "Last seen" label for a table row: {@link humanizeAge} over
 * {@link maxLastSeenMillis}, or "never" when the key has no recorded call at
 * all in either stats block. */
export function lastSeenLabel(entry: GatewayKeyEntry, nowMillis: number): string {
  const ms = maxLastSeenMillis(entry);
  return ms === null ? "never" : humanizeAge(nowMillis - ms);
}
