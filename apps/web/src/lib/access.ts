/**
 * Access matrix (I5): which permissions each agent holds vs actually uses,
 * which MCP tools it reaches (sanctioned vs shadow), and which Wardryx
 * policies overlay it. Entirely derived, client-side, from data the console
 * already fetches elsewhere - `fetchIdentities()`/`fetchAlerts()`
 * (`lib/identity.ts`) and `fetchPolicies()` (`lib/policy.ts`) - exactly like
 * `Agent360.tsx` assembles its own sections from the same per-plane reads.
 * No new backend command, no new wire type: everything below is a pure
 * function over `IdryxIdentity[]`/`IdryxAlert[]`/`PolicyRecord[]`.
 *
 * Every non-trivial rule here is a faithful port of a real server-side
 * behavior, read directly from the two upstream repos rather than guessed:
 *
 * - The glob matcher and the deny_tool/allow_domains/max_steps/
 *   require_human_above_usd/deny_above_usd/deny_if_unattested composition
 *   mirror `~/Development/wardryx/internal/policy/policy.go`'s
 *   `compileGlob`/`normalize` and `~/Development/wardryx/internal/pdp/pdp.go`'s
 *   `Decide` helpers (`deniedTool`, `deniedDomain`, `exceededMaxSteps`,
 *   `overThreshold`, `deniedAboveCeiling`, `unattestedDenied`) - see each
 *   function below for which one it mirrors.
 * - The permission honesty gate mirrors
 *   `~/Development/Idryx/internal/detect/detectors/least_privilege.go`'s own
 *   "only fires for identities that have usage data" rule.
 * - The shadow/sanctioned MCP split mirrors
 *   `~/Development/Idryx/internal/detect/detectors/shadow_mcp.go` (a
 *   `mcp_server` identity is shadow exactly when idryx's `shadow_mcp`
 *   detector fired for it - idryx exposes no shadow flag over REST, only
 *   this alert) and `agent_shadow_tool.go` (the name-intersection join
 *   between an agent's permissions and a shadow server's).
 *
 * This module never calls idryx's or wardryx's own decision/detection logic
 * - it is a read-only, client-side re-derivation for display, built to
 * agree with what those servers would report, never a replacement
 * enforcement or detection path.
 */
import type { IdryxAlert, IdryxIdentity, IdryxPermission } from "../identityTypes";
import type { PolicyRecord } from "../policyTypes";

// ============================================================================
// Wardryx glob (policy.go's compileGlob, verbatim algorithm)
// ============================================================================

/** Characters `regexp.QuoteMeta` escapes in Go, mirrored 1:1 - escaping a
 * different set would change which literal characters in a `target` glob
 * need backslashing before the `*`/`?` substitution below runs. */
const GLOB_SPECIAL = /[\\^$.*+?()[\]{}|]/g;

function quoteMeta(s: string): string {
  return s.replace(GLOB_SPECIAL, "\\$&");
}

/**
 * Compile an `agent://` glob into an anchored `RegExp`, faithfully porting
 * `policy.go`'s `compileGlob`: escape every regex metacharacter, THEN turn
 * the now-escaped `\*`/`\?` back into `.*`/`.` (in that order, exactly as
 * the Go source does it) so the result matches whatever
 * `regexp.Compile("^" + escaped + "$")` would. `.*` matches across `/` -
 * deliberate, per the Go source, not a path-scoped glob - and `?` matches
 * exactly one character. Both anchors are literal string-start/end,
 * matching Go RE2's default (no multiline flag either side, and JS `.`
 * already excludes line terminators the same way RE2's does without a
 * dotAll flag). Returns `null` for an empty pattern, mirroring
 * `compileGlob`'s own `"empty target glob"` error: wardryx's own `validate`
 * rejects an empty `target` before it can ever reach a live policy set, so
 * `null` here only matters for a malformed record this console did not
 * itself write - treated as "matches nothing", never a thrown error, since
 * that malformed record was never actually enforced live either.
 */
export function compileGlob(pattern: string): RegExp | null {
  if (pattern === "") return null;
  const escaped = quoteMeta(pattern).replace(/\\\*/g, ".*").replace(/\\\?/g, ".");
  return new RegExp(`^${escaped}$`);
}

/** Whether `value` (an agent id) matches `pattern` (a wardryx `target` glob) - see {@link compileGlob}. */
export function matchesGlob(value: string, pattern: string): boolean {
  const re = compileGlob(pattern);
  return re !== null && re.test(value);
}

// ============================================================================
// Policy matching + effective overlay (pdp.go's Decide, read-only re-derivation)
// ============================================================================

/** `policy.go`'s `normalize`: `Name` defaults to `Target` when blank, ONLY
 * for ordering purposes here - a policy's own `name` field is rendered
 * verbatim everywhere else (see `wardryx.rs`'s `Policy.name` doc comment:
 * a `PUT` with no name round-trips as `""`, never silently rewritten). */
function effectivePolicyName(p: PolicyRecord): string {
  return p.name === "" ? p.target : p.name;
}

/**
 * Every policy in `policies` whose `target` glob matches `agentId`, sorted
 * by (target, name) - `policy.go`'s `normalize` sorts the whole compiled
 * set this way before `Match` ever runs, so two policy lists that are
 * equivalent modulo source order always compare equal here too.
 * `GET /v1/policies` itself returns id order (`wardryx.rs`'s
 * `list_policies` doc comment), so this function - not the caller - is what
 * makes the result deterministic and wardryx-shaped.
 */
export function matchedPolicies(agentId: string, policies: readonly PolicyRecord[]): PolicyRecord[] {
  return policies
    .filter((p) => matchesGlob(agentId, p.target))
    .sort((a, b) => {
      if (a.target !== b.target) return a.target < b.target ? -1 : 1;
      const an = effectivePolicyName(a);
      const bn = effectivePolicyName(b);
      return an < bn ? -1 : an > bn ? 1 : 0;
    });
}

/** Dedupe + sort, dropping blanks - mirrors `policy.go`'s own `sortedUnique`
 * (used there for `DenyTool`/`AllowDomains` at normalize time). */
function sortedUnique(values: readonly string[]): string[] {
  return [...new Set(values.filter((v) => v !== ""))].sort();
}

/** The smallest strictly-positive value in `values`, or `null` if none is
 * positive - `pdp.go`'s own rule for `max_steps`/`require_human_above_usd`/
 * `deny_above_usd`: zero means "unset" (`policy.go`'s doc comments on those
 * fields), and where more than one matched policy sets one, the STRICTEST
 * (lowest) wins - see `exceededMaxSteps`/`overThreshold`/
 * `deniedAboveCeiling` in `pdp.go`, all of which pick the smallest positive
 * value a request exceeds. */
function minPositive(values: readonly number[]): number | null {
  let best: number | null = null;
  for (const v of values) {
    if (v > 0 && (best === null || v < best)) best = v;
  }
  return best;
}

/** Intersection of every list in `lists` (each already guaranteed non-empty
 * by the caller) - mirrors `pdp.go`'s `deniedDomain` doc comment: "when more
 * than one matched policy declares a non-empty AllowDomains, a domain must
 * appear in every one of them: allow-lists compose by intersection, not
 * union". An empty `lists` returns `[]` (never called with one; guarded
 * only for safety). */
function intersectNonEmpty(lists: readonly (readonly string[])[]): string[] {
  if (lists.length === 0) return [];
  let acc = new Set(lists[0]);
  for (let i = 1; i < lists.length && acc.size > 0; i++) {
    const next = new Set(lists[i]);
    acc = new Set([...acc].filter((v) => next.has(v)));
  }
  return [...acc].sort();
}

/** One matched policy's own non-empty `allow_domains`, for the drilldown. */
export interface PerPolicyDomains {
  policyId: string;
  policyName: string;
  policyTarget: string;
  domains: readonly string[];
}

/**
 * The domain overlay across every matched policy - `pdp.go`'s `deniedDomain`
 * doc comment, verbatim: "a policy whose AllowDomains is empty imposes no
 * restriction" (skipped entirely, never treated as "allow nothing"), and a
 * domain must appear in EVERY non-empty matched list. `{ kind: "unrestricted"
 * }` is the honest state for "no matched policy sets allow_domains at all" -
 * deliberately distinct from `{ kind: "restricted", domains: [] }`, which
 * means the opposite: at least one matched policy DOES restrict domains,
 * and the matched policies' allow-lists share no domain at all, so nothing
 * a caller could ever declare would pass every one of them at once (a real
 * policy contradiction). Collapsing those two into one falsy "nothing here"
 * reading would hide that contradiction behind the same look as "no
 * restriction was ever configured".
 */
export interface AllowDomainsOverlay {
  perPolicy: readonly PerPolicyDomains[];
  effective: { kind: "unrestricted" } | { kind: "restricted"; domains: readonly string[] };
}

export interface PolicyOverlay {
  /** Union of every matched policy's `deny_tool`, deduped and sorted - any
   * ONE matched policy's deny-list blocking a tool is enough
   * (`deniedTool`'s first-hit-wins loop runs across ALL matched policies),
   * so the full denied surface is the union, never any single policy's own
   * list alone. Shown verbatim, without case-folding: `deniedTool`'s live
   * match IS case/whitespace-insensitive (`containsFold`), but that is an
   * enforcement nicety for near-duplicate spellings, not something this
   * read-only display surface needs to reproduce. */
  deniedTools: readonly string[];
  allowDomains: AllowDomainsOverlay;
  maxSteps: number | null;
  requireHumanAboveUsd: number | null;
  denyAboveUsd: number | null;
  /** True if ANY matched policy sets it - `unattestedDenied` denies on the
   * first matched policy that does, so one is exactly as binding as all of
   * them. */
  denyIfUnattested: boolean;
}

/**
 * Compose the effective overlay `matched` policies place on one agent -
 * mirrors `pdp.go`'s per-field composition rules (see each field's own doc
 * comment on {@link PolicyOverlay}/{@link AllowDomainsOverlay}). Pure: never
 * re-matches `matched` against anything, just folds the already-matched
 * list - pair with {@link matchedPolicies} to go from an agent id to an
 * overlay.
 */
export function effectiveOverlay(matched: readonly PolicyRecord[]): PolicyOverlay {
  const nonEmptyAllow = matched.filter((p) => p.allow_domains.length > 0);
  const perPolicy: PerPolicyDomains[] = nonEmptyAllow.map((p) => ({
    policyId: p.id,
    policyName: p.name,
    policyTarget: p.target,
    domains: sortedUnique(p.allow_domains),
  }));
  return {
    deniedTools: sortedUnique(matched.flatMap((p) => p.deny_tool)),
    allowDomains: {
      perPolicy,
      effective:
        nonEmptyAllow.length === 0
          ? { kind: "unrestricted" }
          : { kind: "restricted", domains: intersectNonEmpty(nonEmptyAllow.map((p) => p.allow_domains)) },
    },
    maxSteps: minPositive(matched.map((p) => p.max_steps)),
    requireHumanAboveUsd: minPositive(matched.map((p) => p.require_human_above_usd)),
    denyAboveUsd: minPositive(matched.map((p) => p.deny_above_usd)),
    denyIfUnattested: matched.some((p) => p.deny_if_unattested),
  };
}

// ============================================================================
// Permission rollup + honesty gate (least_privilege.go's own "only fires
// with usage data" rule)
// ============================================================================

export interface PermissionRollup {
  granted: number;
  used: number;
  unused: readonly IdryxPermission[];
  adminUnused: readonly IdryxPermission[];
  /** Mirrors `least_privilege.go`'s `hasUsage`: true only when at least one
   * permission carries `used === true`. When false, `unused`/`adminUnused`
   * above are still populated (every permission trivially has
   * `used === false`), but the caller MUST render a neutral "no usage
   * signal" state instead of a red/escalated highlight - idryx's own
   * detector stays silent in exactly this case ("without usage data it
   * stays silent to avoid recommending removal of permissions that may
   * simply be unobserved"), and this field exists so the UI can make the
   * same call, never to quietly relabel "unobserved" as "unused". */
  hasUsageSignal: boolean;
}

export function permissionRollup(permissions: readonly IdryxPermission[]): PermissionRollup {
  const unused = permissions.filter((p) => !p.used);
  return {
    granted: permissions.length,
    used: permissions.length - unused.length,
    unused,
    adminUnused: unused.filter((p) => p.admin),
    hasUsageSignal: permissions.some((p) => p.used),
  };
}

// ============================================================================
// MCP reach: shadow derivation from alerts + the agent<->server join
// ============================================================================

/** idryx exposes no shadow flag over REST - a `mcp_server` identity is
 * shadow exactly when its own id carries a `shadow_mcp` alert
 * (`shadow_mcp.go`'s detector fires once per shadow server, i.e. one alert
 * per identity). Recomputed fresh from `alerts` every call: the alert IS
 * the signal, there is nothing else on the identity itself to cache this
 * against. */
export function shadowServerIds(alerts: readonly IdryxAlert[]): ReadonlySet<string> {
  return new Set(alerts.filter((a) => a.detector === "shadow_mcp").map((a) => a.identity));
}

/** The `mcp_server` identities in `identities` - split out so a caller
 * building many agents' rows off one identities list filters it once and
 * passes the same array into every {@link mcpReachForAgent} call, rather
 * than each row re-filtering the whole list. */
export function mcpServerIdentities(identities: readonly IdryxIdentity[]): IdryxIdentity[] {
  return identities.filter((i) => i.type === "mcp_server");
}

/** One MCP server this agent reaches, and the permission names it shares
 * with that server - idryx's `calls_tool` edge is permission-NAME equality
 * (this task's own spec: there is no other join key on the wire). */
export interface McpServerMatch {
  serverId: string;
  tools: readonly string[];
}

export interface McpReach {
  sanctionedServers: readonly McpServerMatch[];
  shadowServers: readonly McpServerMatch[];
  sanctionedTools: readonly string[];
  shadowTools: readonly string[];
}

/**
 * This agent's MCP reach, split sanctioned/shadow - generalizes
 * `agent_shadow_tool.go`'s own join (a map of shadow servers' permission
 * names, then each agent's permissions checked against it) to ALSO report
 * the sanctioned side, which idryx's own detector never needed (it only
 * ever flags the shadow half). `mcpServers` should already be
 * `type === "mcp_server"` identities - see {@link mcpServerIdentities}.
 */
export function mcpReachForAgent(
  agentPermissionNames: readonly string[],
  mcpServers: readonly IdryxIdentity[],
  shadowIds: ReadonlySet<string>,
): McpReach {
  const agentNames = new Set(agentPermissionNames);
  const sanctionedServers: McpServerMatch[] = [];
  const shadowServers: McpServerMatch[] = [];
  for (const server of mcpServers) {
    const serverNames = new Set(server.permissions.map((p) => p.name));
    const tools = sortedUnique([...agentNames].filter((n) => serverNames.has(n)));
    if (tools.length === 0) continue;
    const match: McpServerMatch = { serverId: server.id, tools };
    (shadowIds.has(server.id) ? shadowServers : sanctionedServers).push(match);
  }
  const byServerId = (a: McpServerMatch, b: McpServerMatch) => (a.serverId < b.serverId ? -1 : a.serverId > b.serverId ? 1 : 0);
  sanctionedServers.sort(byServerId);
  shadowServers.sort(byServerId);
  return {
    sanctionedServers,
    shadowServers,
    sanctionedTools: sortedUnique(sanctionedServers.flatMap((s) => s.tools)),
    shadowTools: sortedUnique(shadowServers.flatMap((s) => s.tools)),
  };
}

// ============================================================================
// Fleet matrix row model
// ============================================================================

/** The one idryx identity `type` the console treats as an agent - mirrors
 * idryx's own `Identity.IsAgent()` (`internal/model/identity.go`:
 * `i.Type == IdentityAgent`, i.e. exactly `"agent"`, nothing broader) and
 * this console's existing convention: neither `IdentityView.tsx` nor
 * `Agent360.tsx` define any wider "agent-like" classification today. */
const AGENT_IDENTITY_TYPE = "agent";

export function isAgentIdentity(identity: IdryxIdentity): boolean {
  return identity.type === AGENT_IDENTITY_TYPE;
}

export interface AccessRow {
  identity: IdryxIdentity;
  permissions: PermissionRollup;
  mcpReach: McpReach;
  /** `null` exactly when the policy plane has not been loaded - see
   * {@link buildAccessRows}'s doc comment. Never conflate this with "loaded,
   * and zero policies matched" (a `PolicyOverlay` with every field at its
   * own vacuous zero/unrestricted/null/false value): that is a real,
   * meaningful answer ("this agent is ungoverned"), while `null` is "we do
   * not know yet" - callers must render the two differently. */
  policy: { matched: readonly PolicyRecord[]; overlay: PolicyOverlay } | null;
  /** Count of this identity's own `agent_shadow_tool` alerts - informational
   * only. Per this task's own spec this never DERIVES
   * `mcpReach.shadowTools` (that is always the name-intersection join
   * above); idryx's real detector and this join should simply agree in
   * practice when both look at the same snapshot. */
  agentShadowToolAlertCount: number;
}

/**
 * One row per agent identity, built entirely from `identities`/`alerts`
 * (idryx) and `policies` (wardryx) - the exact reads `IdentityView.tsx` and
 * `PolicyView.tsx` already make elsewhere, assembled here the same way
 * `Agent360.tsx` assembles its own sections from equally-already-fetched
 * data. `policies === null` means the policy plane has not (yet, or ever)
 * been loaded - every row's `policy` field is `null` in that case, never a
 * fabricated all-zero overlay (see {@link AccessRow.policy}'s doc comment).
 * Unsorted: pair with {@link sortAccessRowsWorstFirst} (or a caller's own
 * order) on top.
 */
export function buildAccessRows(
  identities: readonly IdryxIdentity[],
  alerts: readonly IdryxAlert[],
  policies: readonly PolicyRecord[] | null,
): AccessRow[] {
  const shadowIds = shadowServerIds(alerts);
  const mcpServers = mcpServerIdentities(identities);
  return identities.filter(isAgentIdentity).map((identity) => {
    const agentShadowToolAlertCount = alerts.filter(
      (a) => a.identity === identity.id && a.detector === "agent_shadow_tool",
    ).length;
    const permissionNames = identity.permissions.map((p) => p.name);
    let policy: AccessRow["policy"] = null;
    if (policies !== null) {
      const matched = matchedPolicies(identity.id, policies);
      policy = { matched, overlay: effectiveOverlay(matched) };
    }
    return {
      identity,
      permissions: permissionRollup(identity.permissions),
      mcpReach: mcpReachForAgent(permissionNames, mcpServers, shadowIds),
      policy,
      agentShadowToolAlertCount,
    };
  });
}

/**
 * Worst-first default order (I5 spec, verbatim): shadow-tool count desc,
 * then unused-admin-permission count desc, then unused-permission count
 * desc - mirrors `lib/credentials.ts`'s `KEY_STATUS_ORDER`/`keyStatusRank`
 * precedent of a fixed, worst-first rank the table sorts by. A final id
 * tie-break keeps the order fully deterministic (never dependent on the
 * input array's own order or the sort algorithm's stability).
 */
export function sortAccessRowsWorstFirst(rows: readonly AccessRow[]): AccessRow[] {
  return [...rows].sort((a, b) => {
    const shadow = b.mcpReach.shadowTools.length - a.mcpReach.shadowTools.length;
    if (shadow !== 0) return shadow;
    const adminUnused = b.permissions.adminUnused.length - a.permissions.adminUnused.length;
    if (adminUnused !== 0) return adminUnused;
    const unused = b.permissions.unused.length - a.permissions.unused.length;
    if (unused !== 0) return unused;
    return a.identity.id < b.identity.id ? -1 : a.identity.id > b.identity.id ? 1 : 0;
  });
}
