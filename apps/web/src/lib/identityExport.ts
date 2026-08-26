/**
 * Saving the Identity view's five tables out of the console, as an access
 * review somebody reads days later and somewhere else.
 *
 * # WHY THIS IS A MODULE AND NOT FIVE `onClick` HANDLERS
 *
 * `lib/download.ts` explains why every export carries provenance. This file is
 * where that provenance is actually decided for this view, and the deciding is
 * the work: five tables, four independent planes behind them (idryx's
 * load-once snapshot, an optional `idryx detect` pass, the TokenFuse gateway,
 * wardryx), and each plane answering for a different window. A row builder is
 * three lines. Knowing that `calls` means two different things depending on
 * one boolean is the part worth testing, so it lives here as pure functions
 * with a suite over them rather than inside a component that cannot be run in
 * a test.
 *
 * # THE ONE RULE UNDER EVERY BUILDER HERE
 *
 * A field the wire did not carry becomes `null`, which `csvCell` writes as an
 * EMPTY cell. Never `0`, never `"never"`, never `"-"`. `0` is a measurement
 * somebody took; an empty cell is the absence of one. Invariant 4 is about not
 * inventing rows, and this is the same instinct one field down.
 */
import type { ExportMeta } from "./download";
import type { IdentityStatus, IdryxIdentity, IdryxRecommendation, IdryxRemediation, IdryxAlert } from "../identityTypes";
import type { AccessRow } from "./access";
import type { CredentialsStatus, GatewayKeyEntry, GatewayKeysReport } from "./credentials";
import { deriveKeyStatus, keyStatusRank, maxLastSeenMillis, totalCalls } from "./credentials";

/** One column of an export, in the shape `lib/download.ts::toCsv` takes. */
export interface ExportColumn<T> {
  key: keyof T & string;
  header: string;
}

// ---------------------------------------------------------------- helpers --

/** idryx sends `""` for a timestamp it has no value for (every one of them is
 * `#[serde(default)]` on a `String`). `""` and "not recorded" are the same
 * statement; `0` and "never" are a different one, so the empty string becomes
 * an empty cell and nothing else. */
function orNull(value: string | null | undefined): string | null {
  return value === undefined || value === null || value === "" ? null : value;
}

function isoOrNull(millis: number | null | undefined): string | null {
  return millis === null || millis === undefined ? null : new Date(millis).toISOString();
}

/** A flat list as one cell. Empty stays `null`: "no items" and "this column was
 * not filled in" are told apart by the caveats, never by an empty list looking
 * like an absent one. */
function listOrNull(values: readonly string[]): string | null {
  return values.length > 0 ? values.join("; ") : null;
}

function at(iso: string | null): string {
  return iso ?? "an unrecorded time";
}

/** The one sentence every window line here needs: whether the alert list came
 * from idryx's REST snapshot or from a `detect` pass that replaced it. */
function alertsWindow(snapshotAt: string | null, rescanAt: string | null): string {
  return rescanAt !== null
    ? `alerts: a fresh idryx detect pass, run at ${rescanAt}, which replaced the snapshot's alert list`
    : `alerts: the idryx REST snapshot read at ${at(snapshotAt)}`;
}

/** Everything a meta needs that is about WHEN rather than WHAT. `snapshotAt`
 * is when the console last read `idryx serve`'s load-once snapshot; `rescanAt`
 * is when a Rescan last replaced the alert list, `null` when none has. */
export interface SnapshotMetaInput {
  environment: string;
  takenAt: string;
  snapshotAt: string | null;
  rescanAt: string | null;
}

const SNAPSHOT_CAVEAT =
  "idryx serve loads its snapshot once at startup and never reloads on its own. This file is that snapshot, not a live read at the moment it was saved.";

/** Which box a file came from, as far as this console can actually tell.
 *
 * `window.location.host` alone is close to useless in a saved artifact: this
 * console is usually served from localhost, so every file from every operator
 * would carry the same word. The plane's own URL and the `taipan up` env name
 * are the part a reader cannot recover later, so they lead. The host stays,
 * because two consoles can point at one plane. */
function environmentLine(planeLine: string, host: string): string {
  return `${planeLine}, console at ${host !== "" ? host : "an unknown host"}`;
}

/** The environment line for anything read out of idryx (identities, alerts,
 * remediations, the access matrix's identity half). */
export function idryxEnvironment(status: IdentityStatus | null, host: string): string {
  if (status === null || status.state === "bootstrapping") {
    return environmentLine("identity plane not resolved yet", host);
  }
  if (status.state === "no_environment") {
    return environmentLine("no idryx plane in this environment", host);
  }
  const plane = `taipan env "${status.source.name}", idryx at ${status.idryx_url}`;
  return environmentLine(status.state === "unreachable" ? `${plane} (unreachable)` : plane, host);
}

/** The environment line for the gateway's key report. A separate descriptor
 * service from idryx (`useCredentialsStatus`), so a separate line: an
 * environment can have one up and the other down. */
export function gatewayEnvironment(status: CredentialsStatus | null, host: string): string {
  if (status === null || status.state === "bootstrapping") {
    return environmentLine("gateway not resolved yet", host);
  }
  if (status.state === "no_environment") {
    return environmentLine("no gateway in this environment", host);
  }
  const plane = `taipan env "${status.source.name}", gateway at ${status.gateway_url}`;
  return environmentLine(status.state === "unreachable" ? `${plane} (unreachable)` : plane, host);
}

const SPLIT_WINDOW_CAVEAT =
  "The alerts here came from a detect pass and the identities did not: the identities are still the older REST snapshot. Anything joining the two is joining two different reads.";

// ------------------------------------------------------------- identities --

export interface IdentityExportRow {
  id: string;
  type: string;
  privileged: boolean;
  source: string;
  owner: string | null;
  created: string | null;
  last_used: string | null;
  runtime: string | null;
  on_behalf_of: string | null;
  permissions_granted: number;
  permissions_used: number;
  permissions_admin: number;
  permissions: string | null;
  events: number;
  alerts: number;
  remediation_kind: string | null;
  remediation_explanation: string | null;
  remediation_code: string | null;
  remediation_created_at: string | null;
  rotation_kind: string | null;
  rotation_explanation: string | null;
  rotation_code: string | null;
  rotation_created_at: string | null;
}

export const IDENTITY_COLUMNS: ExportColumn<IdentityExportRow>[] = [
  { key: "id", header: "id" },
  { key: "type", header: "type" },
  { key: "privileged", header: "privileged" },
  { key: "source", header: "source" },
  { key: "owner", header: "owner" },
  { key: "created", header: "created" },
  { key: "last_used", header: "last_used" },
  { key: "runtime", header: "runtime" },
  { key: "on_behalf_of", header: "on_behalf_of" },
  { key: "permissions_granted", header: "permissions_granted" },
  { key: "permissions_used", header: "permissions_used" },
  { key: "permissions_admin", header: "permissions_admin" },
  { key: "permissions", header: "permissions" },
  { key: "events", header: "events" },
  { key: "alerts", header: "alerts" },
  { key: "remediation_kind", header: "remediation_kind" },
  { key: "remediation_explanation", header: "remediation_explanation" },
  { key: "remediation_code", header: "remediation_code" },
  { key: "remediation_created_at", header: "remediation_created_at" },
  { key: "rotation_kind", header: "rotation_kind" },
  { key: "rotation_explanation", header: "rotation_explanation" },
  { key: "rotation_code", header: "rotation_code" },
  { key: "rotation_created_at", header: "rotation_created_at" },
];

/** A suggestion's four fields, flattened under one prefix. `null` throughout
 * when idryx generated none: the field is `Option<Remediation>` on the wire and
 * absent means "idryx had no suggestion", which is a real answer and not a
 * suggestion with empty text. */
function suggestion(r: IdryxRemediation | null): {
  kind: string | null;
  explanation: string | null;
  code: string | null;
  created_at: string | null;
} {
  return {
    kind: r ? orNull(r.kind) : null,
    explanation: r ? orNull(r.explanation) : null,
    code: r ? orNull(r.code) : null,
    created_at: r ? orNull(r.created_at) : null,
  };
}

export function identityExportRows(identities: readonly IdryxIdentity[]): IdentityExportRow[] {
  return identities.map((i) => {
    const rem = suggestion(i.remediation);
    const rot = suggestion(i.rotation);
    return {
      id: i.id,
      type: i.type,
      privileged: i.privileged,
      source: i.source,
      owner: orNull(i.owner),
      created: orNull(i.created),
      last_used: orNull(i.last_used),
      runtime: orNull(i.runtime),
      // Root-first, with the arrow, because the ORDER is the claim: this is a
      // delegation chain, not a set of co-owners.
      on_behalf_of: i.on_behalf_of.length > 0 ? i.on_behalf_of.join(" -> ") : null,
      permissions_granted: i.permissions.length,
      permissions_used: i.permissions.filter((p) => p.used).length,
      permissions_admin: i.permissions.filter((p) => p.admin).length,
      permissions: listOrNull([...i.permissions.map((p) => p.name)].sort()),
      events: i.events,
      alerts: i.alerts,
      remediation_kind: rem.kind,
      remediation_explanation: rem.explanation,
      remediation_code: rem.code,
      remediation_created_at: rem.created_at,
      rotation_kind: rot.kind,
      rotation_explanation: rot.explanation,
      rotation_code: rot.code,
      rotation_created_at: rot.created_at,
    };
  });
}

export function identityExportMeta(
  input: SnapshotMetaInput & { identities: readonly IdryxIdentity[] },
): ExportMeta {
  const anyUsage = input.identities.some((i) => i.permissions.some((p) => p.used));
  const anySuggestion = input.identities.some((i) => i.remediation !== null || i.rotation !== null);
  return {
    subject: "Genaryx identities (idryx snapshot)",
    environment: input.environment,
    takenAt: input.takenAt,
    windows: [`identities: the idryx REST snapshot read at ${at(input.snapshotAt)}`],
    caveats: [
      "Every identity in the snapshot. The type filter chips in the console narrow what is on screen, never this file.",
      SNAPSHOT_CAVEAT,
      "created, last_used and runtime are empty when idryx recorded no value. An empty last_used is not the same as never used: idryx simply has no timestamp for it.",
      "events and alerts are counts idryx computed over its own snapshot, not the objects behind them.",
      anyUsage
        ? "permissions_used counts the permissions idryx observed in use. A permission missing from that count was not observed, which is a weaker statement than unnecessary."
        : "No identity in this file carries a single permission idryx observed in use, so every permission here is unobserved rather than unnecessary. idryx's own least_privilege detector stays silent in exactly this case, and this file makes the same claim it does: none.",
      ...(anySuggestion
        ? [
            "remediation_* and rotation_* are the suggestion idryx attached to the identity itself, empty where it attached none. remediation_code and rotation_code are the code field idryx returns beside each explanation, passed through verbatim: this console does not interpret it and makes no claim about what it is for.",
          ]
        : []),
    ],
  };
}

// ----------------------------------------------------------------- alerts --

export interface AlertExportRow {
  severity: string;
  detector: string;
  identity: string;
  time: string;
  summary: string;
}

export const ALERT_COLUMNS: ExportColumn<AlertExportRow>[] = [
  { key: "severity", header: "severity" },
  { key: "detector", header: "detector" },
  { key: "identity", header: "identity" },
  { key: "time", header: "time" },
  { key: "summary", header: "summary" },
];

export function alertExportRows(alerts: readonly IdryxAlert[]): AlertExportRow[] {
  return alerts.map((a) => ({
    severity: a.severity,
    detector: a.detector,
    identity: a.identity,
    time: a.time,
    summary: a.summary,
  }));
}

export function alertExportMeta(input: SnapshotMetaInput): ExportMeta {
  return {
    subject: "Genaryx identity alerts (idryx)",
    environment: input.environment,
    takenAt: input.takenAt,
    windows: [alertsWindow(input.snapshotAt, input.rescanAt)],
    caveats: [
      "Every alert in this pass. The severity chips and the detector selector in the console narrow what is on screen, never this file.",
      "severity is decided per alert, escalated by idryx from the detector's own base. Two alerts from one detector can carry different severities, so never read a severity off a detector id.",
      "Attestation status reaches this console only as free text inside the attestation_missing and bom_incomplete summaries. idryx has no structured attestation field, so this file has no attestation column.",
      ...(input.rescanAt !== null ? [SPLIT_WINDOW_CAVEAT] : [SNAPSHOT_CAVEAT]),
    ],
  };
}

// ----------------------------------------------------------- remediations --

export interface RemediationExportRow {
  identity: string;
  kind: string;
  explanation: string;
  code: string | null;
  created_at: string | null;
}

export const REMEDIATION_COLUMNS: ExportColumn<RemediationExportRow>[] = [
  { key: "identity", header: "identity" },
  { key: "kind", header: "kind" },
  { key: "explanation", header: "explanation" },
  { key: "code", header: "code" },
  { key: "created_at", header: "created_at" },
];

export function remediationExportRows(
  remediations: readonly IdryxRecommendation[],
): RemediationExportRow[] {
  return remediations.map((r) => ({
    identity: r.identity,
    kind: r.kind,
    explanation: r.explanation,
    code: orNull(r.code),
    created_at: orNull(r.created_at),
  }));
}

export function remediationExportMeta(input: SnapshotMetaInput): ExportMeta {
  return {
    subject: "Genaryx identity remediations (idryx)",
    environment: input.environment,
    takenAt: input.takenAt,
    windows: [`remediations: the idryx REST snapshot read at ${at(input.snapshotAt)}`],
    caveats: [
      SNAPSHOT_CAVEAT,
      "code is the code field idryx returns beside each explanation, passed through verbatim. This console does not interpret it and makes no claim about what it is for.",
      "created_at is empty where idryx sent none. That is an absent timestamp, not an old one.",
      "This is GET /api/remediations. An identity's own remediation and rotation fields are a separate read, exported with the identities table; this console does not assume the two lists hold the same records.",
    ],
  };
}

// ---------------------------------------------------------- access matrix --

export interface AccessExportRow {
  agent: string;
  permissions_granted: number;
  permissions_used: number;
  permissions_unused: number;
  permissions_unused_admin: number;
  has_usage_signal: boolean;
  mcp_sanctioned_servers: string | null;
  mcp_sanctioned_tools: number;
  mcp_shadow_servers: string | null;
  mcp_shadow_tools: number;
  agent_shadow_tool_alerts: number;
  matched_policies: number | null;
  denied_tools: number | null;
  denied_tool_names: string | null;
  /** `"unrestricted"` or `"restricted"`, and `null` when the policy plane was
   * never read. The two non-null values are NOT interchangeable with an empty
   * `allow_domains`: see this module's `accessExportMeta` caveat. */
  allow_domains_mode: string | null;
  allow_domains: string | null;
  max_steps: number | null;
  deny_if_unattested: boolean | null;
}

export const ACCESS_COLUMNS: ExportColumn<AccessExportRow>[] = [
  { key: "agent", header: "agent" },
  { key: "permissions_granted", header: "permissions_granted" },
  { key: "permissions_used", header: "permissions_used" },
  { key: "permissions_unused", header: "permissions_unused" },
  { key: "permissions_unused_admin", header: "permissions_unused_admin" },
  { key: "has_usage_signal", header: "has_usage_signal" },
  { key: "mcp_sanctioned_servers", header: "mcp_sanctioned_servers" },
  { key: "mcp_sanctioned_tools", header: "mcp_sanctioned_tools" },
  { key: "mcp_shadow_servers", header: "mcp_shadow_servers" },
  { key: "mcp_shadow_tools", header: "mcp_shadow_tools" },
  { key: "agent_shadow_tool_alerts", header: "agent_shadow_tool_alerts" },
  { key: "matched_policies", header: "matched_policies" },
  { key: "denied_tools", header: "denied_tools" },
  { key: "denied_tool_names", header: "denied_tool_names" },
  { key: "allow_domains_mode", header: "allow_domains_mode" },
  { key: "allow_domains", header: "allow_domains" },
  { key: "max_steps", header: "max_steps" },
  { key: "deny_if_unattested", header: "deny_if_unattested" },
];

/** A row whose matched policies each restrict domains and share none. Kept as
 * its own predicate because the export and the caveat must agree on what
 * counts as one. */
function isDomainContradiction(row: AccessRow): boolean {
  const eff = row.policy?.overlay.allowDomains.effective;
  return eff !== undefined && eff.kind === "restricted" && eff.domains.length === 0;
}

export function accessExportRows(rows: readonly AccessRow[]): AccessExportRow[] {
  return rows.map((r) => {
    const overlay = r.policy?.overlay ?? null;
    const eff = overlay?.allowDomains.effective ?? null;
    return {
      agent: r.identity.id,
      permissions_granted: r.permissions.granted,
      permissions_used: r.permissions.used,
      permissions_unused: r.permissions.unused.length,
      permissions_unused_admin: r.permissions.adminUnused.length,
      has_usage_signal: r.permissions.hasUsageSignal,
      mcp_sanctioned_servers: listOrNull(r.mcpReach.sanctionedServers.map((s) => s.serverId)),
      mcp_sanctioned_tools: r.mcpReach.sanctionedTools.length,
      mcp_shadow_servers: listOrNull(r.mcpReach.shadowServers.map((s) => s.serverId)),
      mcp_shadow_tools: r.mcpReach.shadowTools.length,
      agent_shadow_tool_alerts: r.agentShadowToolAlertCount,
      matched_policies: r.policy === null ? null : r.policy.matched.length,
      denied_tools: overlay === null ? null : overlay.deniedTools.length,
      denied_tool_names: overlay === null ? null : listOrNull(overlay.deniedTools),
      allow_domains_mode: eff === null ? null : eff.kind,
      allow_domains: eff === null || eff.kind === "unrestricted" ? null : listOrNull(eff.domains),
      max_steps: overlay === null ? null : overlay.maxSteps,
      deny_if_unattested: overlay === null ? null : overlay.denyIfUnattested,
    };
  });
}

export function accessExportMeta(
  input: SnapshotMetaInput & { policyNote: string | null; rows: readonly AccessRow[] },
): ExportMeta {
  const noSignal = input.rows.filter((r) => !r.permissions.hasUsageSignal).length;
  const contradictions = input.rows.filter(isDomainContradiction).length;
  return {
    subject: "Genaryx access matrix (idryx identities, wardryx policy overlay)",
    environment: input.environment,
    takenAt: input.takenAt,
    windows: [
      `identities: the idryx REST snapshot read at ${at(input.snapshotAt)}`,
      alertsWindow(input.snapshotAt, input.rescanAt),
      input.policyNote === null
        ? "policies: a one-shot wardryx read, taken independently of the idryx snapshot above"
        : `policies: NOT READ, ${input.policyNote}`,
    ],
    caveats: [
      "Only identities idryx typed as agent are rows here. Every other identity type is in the identities table instead.",
      ...(input.policyNote !== null
        ? [
            `The wardryx columns (matched_policies, denied_tools, denied_tool_names, allow_domains_mode, allow_domains, max_steps, deny_if_unattested) are empty in every row because the policy plane could not be read: ${input.policyNote}. Empty there means not checked, not zero.`,
          ]
        : [
            "allow_domains_mode=unrestricted means no matched policy restricted domains at all. allow_domains_mode=restricted with an empty allow_domains is the opposite and much worse: policies did restrict, and their allow-lists intersect to nothing.",
          ]),
      ...(contradictions > 0
        ? [
            `${contradictions} agent(s) match policies that each restrict domains but share no domain in common, so every domain-declaring action from them is denied. Those rows read allow_domains_mode=restricted with an empty allow_domains.`,
          ]
        : []),
      ...(noSignal > 0
        ? [
            `${noSignal} of ${input.rows.length} row(s) carry no usage signal at all (has_usage_signal=false). Their permissions_unused counts are unobserved permissions, not unnecessary ones, and idryx's own least_privilege detector stays silent for exactly those rows.`,
          ]
        : []),
      "mcp_shadow_tools is this console's own name-intersection join between an agent's permissions and the MCP servers idryx flagged as shadow. agent_shadow_tool_alerts is idryx's own detector count. They answer a similar question two ways and are reported separately rather than reconciled.",
      ...(input.rescanAt !== null ? [SPLIT_WINDOW_CAVEAT] : [SNAPSHOT_CAVEAT]),
    ],
  };
}

// ------------------------------------------------------------------- keys --

export interface KeyExportRow {
  key_id: string;
  status: string;
  configured: boolean;
  bound: boolean;
  unit: string | null;
  agents: string | null;
  created: string | null;
  last_seen: string | null;
  calls: number;
  calls_since_startup: number;
  calls_history: number | null;
  identity_mismatches_since_startup: number;
  identity_mismatches_history: number | null;
  last_seen_since_startup: string | null;
  last_seen_history: string | null;
  first_seen_history: string | null;
}

export const KEY_COLUMNS: ExportColumn<KeyExportRow>[] = [
  { key: "status", header: "status" },
  { key: "key_id", header: "key_id" },
  { key: "configured", header: "configured" },
  { key: "bound", header: "bound" },
  { key: "unit", header: "unit" },
  { key: "agents", header: "agents" },
  { key: "created", header: "created" },
  { key: "last_seen", header: "last_seen" },
  { key: "calls", header: "calls" },
  { key: "calls_since_startup", header: "calls_since_startup" },
  { key: "calls_history", header: "calls_history" },
  { key: "identity_mismatches_since_startup", header: "identity_mismatches_since_startup" },
  { key: "identity_mismatches_history", header: "identity_mismatches_history" },
  { key: "last_seen_since_startup", header: "last_seen_since_startup" },
  { key: "last_seen_history", header: "last_seen_history" },
  { key: "first_seen_history", header: "first_seen_history" },
];

/**
 * What the merged `last seen` and `calls` columns actually cover.
 *
 * ONE sentence, rendered on the page by `CredentialsKeysTable` and written
 * into the file by {@link keyExportMeta}. That is the point of it living here:
 * a screenshot of the table and a CSV saved from the same table must not be
 * able to disagree about what their two most-read numbers mean, and until this
 * existed the only disclosure on screen was a `title=` tooltip, which survives
 * neither.
 *
 * The `history_available` half is measured, not assumed: tokenfuse's
 * `crates/gateway/src/keysreport.rs` writes a ZEROED history block for every
 * key whenever it has a trace directory to fold, so a 0 there is a read that
 * found nothing, and a MISSING block only happens when the gateway has no
 * store at all.
 */
export function keyWindowSentence(report: GatewayKeysReport): string {
  return report.history_available
    ? "last seen and calls merge the gateway's stored call history with the current gateway process. A 0 in the history columns is a read that found no rows for that key, not a missing measurement."
    : "This gateway keeps no stored call history, so last seen and calls cover only the time since the gateway process started. A key that was in heavy use before the last restart reads here as never used.";
}

/**
 * What `strict_mode` was, and what it does to the rest of the table.
 *
 * Shown beside the key table and written into the file, from this one source.
 * It matters here because it silently changes what a DERIVED column means:
 * tokenfuse counts an identity mismatch from the `warn` and `enforce` paths
 * and deliberately not from `off` (`crates/gateway/src/keystats.rs`:
 * "Not called from the off path"), so under `off` the `mismatching` status
 * cannot fire on anything this gateway process saw, however wrong the traffic
 * was. A clean-looking column and an unchecked one render identically.
 *
 * An unrecognised value is passed through and named as unrecognised. This
 * console does not own the vocabulary and must not invent a meaning for a
 * mode a newer gateway added.
 */
export function strictModeSentence(report: GatewayKeysReport): string {
  switch (report.strict_mode) {
    case "off":
      return 'Strict mode was "off": the gateway resolved each key\'s binding for attribution but ran no check against it, and a mismatch is not counted at all while it stays off. A mismatching row in this file can therefore only come from stored history recorded when strict mode was something else.';
    case "warn":
      return 'Strict mode was "warn": a call whose key resolved to a binding the identity map did not expect was still served, with a would-block header on the response, and the mismatch was counted.';
    case "enforce":
      return 'Strict mode was "enforce": a call whose key resolved to a binding the identity map did not expect was refused with a 403 and never reached the provider, and the mismatch was counted.';
    default:
      return `Strict mode was "${report.strict_mode}", a value this console does not recognise. It is the gateway's identity-map enforcement mode, passed through here exactly as sent.`;
  }
}

/** The merged `calls` number split back into the two windows it came from, as
 * a line the table prints under the number. Never a `0` for a window the
 * gateway did not report: an absent history block is said to be absent. */
export function callsBreakdown(entry: GatewayKeyEntry, report: GatewayKeysReport): string {
  const since = `${entry.since_startup.calls.toLocaleString("en-US")} since gateway start`;
  if (entry.history !== null) {
    return `${since} + ${entry.history.calls.toLocaleString("en-US")} stored`;
  }
  return report.history_available
    ? `${since}, no stored history block for this key`
    : `${since}, no stored history on this gateway`;
}

/** Which of the two windows the `last seen` cell is actually showing, or
 * `null` when neither window recorded a call. The table prints it under the
 * age, because "3d ago" from a stored trace and "3d ago" from this process are
 * different claims about a key. */
export function lastSeenSource(entry: GatewayKeyEntry): "since gateway start" | "stored history" | null {
  const startup = entry.since_startup.last_seen_millis;
  const history = entry.history?.last_seen_millis ?? null;
  if (startup === null && history === null) return null;
  if (history === null) return "since gateway start";
  if (startup === null) return "stored history";
  return history > startup ? "stored history" : "since gateway start";
}

function keyRow(entry: GatewayKeyEntry, status: string): KeyExportRow {
  return {
    key_id: entry.key_id,
    status,
    configured: entry.configured,
    bound: entry.bound,
    unit: orNull(entry.unit),
    agents: listOrNull(entry.agents),
    created: orNull(entry.created),
    // An absolute timestamp, not the table's "3d ago": an age is only true at
    // the moment it is read, and this file is read later and elsewhere.
    last_seen: isoOrNull(maxLastSeenMillis(entry)),
    calls: totalCalls(entry),
    calls_since_startup: entry.since_startup.calls,
    // `null`, not `0`. A key with no history block was not called zero times
    // before this process started: the gateway does not know how many times it
    // was called.
    calls_history: entry.history?.calls ?? null,
    identity_mismatches_since_startup: entry.since_startup.identity_mismatches,
    identity_mismatches_history: entry.history?.identity_mismatches ?? null,
    last_seen_since_startup: isoOrNull(entry.since_startup.last_seen_millis),
    last_seen_history: isoOrNull(entry.history?.last_seen_millis ?? null),
    // `since_startup` never carries a `first_seen_millis` on the real wire
    // (`GatewayKeyStats`'s own doc comment), so there is no column for one.
    first_seen_history: isoOrNull(entry.history?.first_seen_millis ?? null),
  };
}

/** Worst-first, exactly the order `CredentialsKeysTable` puts on screen: a file
 * whose rows are in a different order from the table it was saved from is one
 * more thing for a reader to reconcile. */
export function keyExportRows(report: GatewayKeysReport, nowMillis: number): KeyExportRow[] {
  return [...report.keys]
    .map((entry) => ({ entry, status: deriveKeyStatus(entry, report, nowMillis) }))
    .sort(
      (a, b) =>
        keyStatusRank(a.status) - keyStatusRank(b.status) ||
        a.entry.key_id.localeCompare(b.entry.key_id),
    )
    .map(({ entry, status }) => keyRow(entry, status));
}

export function keyExportMeta(input: {
  environment: string;
  takenAt: string;
  report: GatewayKeysReport;
  nowMillis: number;
}): ExportMeta {
  const { report, nowMillis } = input;
  const attempts = report.unauthorized_since_startup.attempts;
  const blockless = report.keys.filter((k) => k.history === null).length;
  return {
    subject: "Genaryx gateway client keys",
    environment: input.environment,
    takenAt: input.takenAt,
    windows: [
      `keys: the gateway's GET /v1/keys, as this console last polled it, at most 30 seconds before ${input.takenAt}`,
      keyWindowSentence(report),
    ],
    caveats: [
      `status is derived by this console from the other columns in this file (lib/credentials.ts deriveKeyStatus), not sent by the gateway. Its "stale" value means last seen more than 7 days before ${new Date(nowMillis).toISOString()}, the clock this file was taken against.`,
      strictModeSentence(report),
      ...(report.identity_map_configured
        ? []
        : [
            "This environment has no identity map configured, so bound is false and unit, agents and created are empty for every key in this file. The gateway takes all three from the map's binding and from nowhere else, so that emptiness is the map being absent rather than a finding about the key, and the unbound status can never fire here.",
          ]),
      ...(report.history_available
        ? [
            ...(blockless > 0
              ? [
                  `${blockless} key(s) carry no history block at all although this gateway reports stored history as available. Their history columns are empty rather than 0, because the gateway did not say.`,
                ]
              : []),
          ]
        : [
            "The empty history columns are the store being absent, not a count of zero.",
            "A key that is in neither TOKENFUSE_CLIENT_KEYS nor the identity map reaches this report only through stored history, so with none, a fully decommissioned key has no row at all here rather than a row with empty columns. An access review cannot see what is missing.",
          ]),
      ...(attempts > 0
        ? [
            `The gateway recorded ${attempts} unauthorized attempt(s) since it started. An unauthorized request never resolved to a key_id, so those appear in no row of this file.`,
          ]
        : []),
      "configured is whether the key is in TOKENFUSE_CLIENT_KEYS right now; bound is whether the identity map matches it right now. Both are present state, not history.",
    ],
  };
}
