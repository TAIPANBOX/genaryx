/**
 * BASELINE STUB. This file currently mirrors, field for field, exactly what the
 * five Identity-view tables render on screen today:
 *
 *   IdentityList.tsx:111        type id source owner privileged perms events alerts on_behalf_of
 *   IdentityAlerts.tsx:140      severity detector identity time summary
 *   IdentityView.tsx:575-595    identity kind explanation
 *   AccessMatrixTable.tsx:148   agent granted used unused mcp-sanctioned mcp-shadow denied-tools domains flags
 *   CredentialsKeysTable.tsx:95 status key_id unit agents created last-seen calls
 *
 * It is committed in this shape on purpose, as the thing the new suite is run
 * against first: every field the wire carries and the view drops is a failing
 * assertion here before it is a passing one.
 */
import type { ExportMeta } from "./download";
import type { IdryxIdentity, IdryxRecommendation, IdryxAlert } from "../identityTypes";
import type { AccessRow } from "./access";
import type { GatewayKeysReport } from "./credentials";
import { deriveKeyStatus, keyStatusRank, lastSeenLabel, totalCalls } from "./credentials";

export interface ExportColumn<T> {
  key: keyof T & string;
  header: string;
}

// ---------------------------------------------------------------- identities

export interface IdentityExportRow {
  id: string;
  type: string;
  privileged: boolean;
  source: string;
  owner: string | null;
  permissions_granted: number;
  events: number;
  alerts: number;
  on_behalf_of: string | null;
}

export const IDENTITY_COLUMNS: ExportColumn<IdentityExportRow>[] = [
  { key: "id", header: "id" },
  { key: "type", header: "type" },
  { key: "privileged", header: "privileged" },
  { key: "source", header: "source" },
  { key: "owner", header: "owner" },
  { key: "permissions_granted", header: "permissions_granted" },
  { key: "events", header: "events" },
  { key: "alerts", header: "alerts" },
  { key: "on_behalf_of", header: "on_behalf_of" },
];

export function identityExportRows(identities: readonly IdryxIdentity[]): IdentityExportRow[] {
  return identities.map((i) => ({
    id: i.id,
    type: i.type,
    privileged: i.privileged,
    source: i.source,
    owner: i.owner || null,
    permissions_granted: i.permissions.length,
    events: i.events,
    alerts: i.alerts,
    on_behalf_of: i.on_behalf_of.length > 0 ? i.on_behalf_of.join(" -> ") : null,
  }));
}

export interface SnapshotMetaInput {
  environment: string;
  takenAt: string;
  /** When the console last read the idryx REST snapshot, `null` if never. */
  snapshotAt: string | null;
  /** When the freshest `idryx detect` Rescan replaced the alert list. */
  rescanAt: string | null;
}

export function identityExportMeta(
  input: SnapshotMetaInput & { identities: readonly IdryxIdentity[] },
): ExportMeta {
  return {
    subject: "Genaryx identities (idryx snapshot)",
    environment: input.environment,
    takenAt: input.takenAt,
    windows: [`identities: the idryx snapshot read at ${input.snapshotAt ?? "an unrecorded time"}`],
    caveats: [],
  };
}

// -------------------------------------------------------------------- alerts

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
    windows: [`alerts: read at ${input.snapshotAt ?? "an unrecorded time"}`],
    caveats: [],
  };
}

// -------------------------------------------------------------- remediations

export interface RemediationExportRow {
  identity: string;
  kind: string;
  explanation: string;
}

export const REMEDIATION_COLUMNS: ExportColumn<RemediationExportRow>[] = [
  { key: "identity", header: "identity" },
  { key: "kind", header: "kind" },
  { key: "explanation", header: "explanation" },
];

export function remediationExportRows(
  remediations: readonly IdryxRecommendation[],
): RemediationExportRow[] {
  return remediations.map((r) => ({
    identity: r.identity,
    kind: r.kind,
    explanation: r.explanation,
  }));
}

export function remediationExportMeta(input: SnapshotMetaInput): ExportMeta {
  return {
    subject: "Genaryx identity remediations (idryx)",
    environment: input.environment,
    takenAt: input.takenAt,
    windows: [`remediations: the idryx snapshot read at ${input.snapshotAt ?? "an unrecorded time"}`],
    caveats: [],
  };
}

// -------------------------------------------------------------- access matrix

export interface AccessExportRow {
  agent: string;
  permissions_granted: number;
  permissions_used: number;
  permissions_unused: number;
  mcp_sanctioned_tools: number;
  mcp_shadow_tools: number;
  denied_tools: number | null;
  allow_domains: string | null;
  max_steps: number | null;
}

export const ACCESS_COLUMNS: ExportColumn<AccessExportRow>[] = [
  { key: "agent", header: "agent" },
  { key: "permissions_granted", header: "granted" },
  { key: "permissions_used", header: "used" },
  { key: "permissions_unused", header: "unused" },
  { key: "mcp_sanctioned_tools", header: "mcp_sanctioned_tools" },
  { key: "mcp_shadow_tools", header: "mcp_shadow_tools" },
  { key: "denied_tools", header: "denied_tools" },
  { key: "allow_domains", header: "allow_domains" },
  { key: "max_steps", header: "max_steps" },
];

export function accessExportRows(rows: readonly AccessRow[]): AccessExportRow[] {
  return rows.map((r) => ({
    agent: r.identity.id,
    permissions_granted: r.permissions.granted,
    permissions_used: r.permissions.used,
    permissions_unused: r.permissions.unused.length,
    mcp_sanctioned_tools: r.mcpReach.sanctionedTools.length,
    mcp_shadow_tools: r.mcpReach.shadowTools.length,
    denied_tools: r.policy === null ? null : r.policy.overlay.deniedTools.length,
    allow_domains:
      r.policy === null
        ? null
        : r.policy.overlay.allowDomains.effective.kind === "unrestricted"
          ? "unrestricted"
          : r.policy.overlay.allowDomains.effective.domains.join("; "),
    max_steps: r.policy === null ? null : r.policy.overlay.maxSteps,
  }));
}

export function accessExportMeta(
  input: SnapshotMetaInput & { policyNote: string | null },
): ExportMeta {
  return {
    subject: "Genaryx access matrix",
    environment: input.environment,
    takenAt: input.takenAt,
    windows: [`identities and alerts: the idryx snapshot read at ${input.snapshotAt ?? "an unrecorded time"}`],
    caveats: [],
  };
}

// ---------------------------------------------------------------------- keys

export interface KeyExportRow {
  key_id: string;
  status: string;
  unit: string | null;
  agents: string | null;
  created: string | null;
  last_seen: string;
  calls: number;
}

export const KEY_COLUMNS: ExportColumn<KeyExportRow>[] = [
  { key: "status", header: "status" },
  { key: "key_id", header: "key_id" },
  { key: "unit", header: "unit" },
  { key: "agents", header: "agents" },
  { key: "created", header: "created" },
  { key: "last_seen", header: "last seen" },
  { key: "calls", header: "calls" },
];

export function keyExportRows(report: GatewayKeysReport, nowMillis: number): KeyExportRow[] {
  return [...report.keys]
    .map((entry) => ({ entry, status: deriveKeyStatus(entry, report, nowMillis) }))
    .sort(
      (a, b) =>
        keyStatusRank(a.status) - keyStatusRank(b.status) ||
        a.entry.key_id.localeCompare(b.entry.key_id),
    )
    .map(({ entry, status }) => ({
      key_id: entry.key_id,
      status,
      unit: entry.unit,
      agents: entry.agents.length > 0 ? entry.agents.join("; ") : null,
      created: entry.created,
      last_seen: lastSeenLabel(entry, nowMillis),
      calls: totalCalls(entry),
    }));
}

export function keyExportMeta(input: {
  environment: string;
  takenAt: string;
  report: GatewayKeysReport;
}): ExportMeta {
  return {
    subject: "Genaryx gateway client keys",
    environment: input.environment,
    takenAt: input.takenAt,
    windows: ["keys: the gateway's GET /v1/keys"],
    caveats: [],
  };
}
