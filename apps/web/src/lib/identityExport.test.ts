/**
 * What the Identity view's exports must carry, and what they must NOT say when
 * a field is absent.
 *
 * The rule under every case here: a field the wire carries and the view drops
 * is a gap, and the fix is to render it - but a field the wire does NOT carry
 * must come out as an empty cell, never as a zero. `0` and `never` are
 * measurements somebody took. `null` is the absence of one, and the CSV writes
 * it as an empty cell on purpose (`lib/download.ts::csvCell`).
 *
 * Run against the baseline commit first, where this module mirrored the five
 * tables as they render on screen. Every assertion below was red there.
 */
import { describe, expect, it } from "vitest";
import { toCsv } from "./download";
import { buildAccessRows } from "./access";
import type { IdryxAlert, IdryxIdentity, IdryxRecommendation, IdryxRemediation } from "../identityTypes";
import type { GatewayKeyEntry, GatewayKeyStats, GatewayKeysReport } from "./credentials";
import type { PolicyRecord } from "../policyTypes";
import {
  ACCESS_COLUMNS,
  ALERT_COLUMNS,
  IDENTITY_COLUMNS,
  KEY_COLUMNS,
  REMEDIATION_COLUMNS,
  accessExportMeta,
  accessExportRows,
  alertExportMeta,
  alertExportRows,
  identityExportMeta,
  identityExportRows,
  keyExportMeta,
  keyExportRows,
  remediationExportMeta,
  remediationExportRows,
  type ExportColumn,
} from "./identityExport";

// ---------------------------------------------------------------- fixtures --
// Built here rather than imported: `mockData.ts` is fixture ROW data and
// invariant 4's gate keeps it out of everything but `lib/recentEvents.ts`.

function identity(over: Partial<IdryxIdentity> = {}): IdryxIdentity {
  return {
    id: "agent://acme.local/reporter",
    type: "agent",
    privileged: false,
    source: "agents",
    owner: "platform",
    created: "2026-01-02 03:04:05 UTC",
    last_used: "2026-08-01 09:00:00 UTC",
    runtime: "python3.12",
    on_behalf_of: [],
    permissions: [],
    remediation: null,
    rotation: null,
    events: 0,
    alerts: 0,
    ...over,
  };
}

function remediation(over: Partial<IdryxRemediation> = {}): IdryxRemediation {
  return {
    kind: "right_size",
    explanation: "3 of 9 permissions unused",
    code: 'resource "aws_iam_policy" {}',
    created_at: "2026-01-02 03:04:05 UTC",
    ...over,
  };
}

function stats(over: Partial<GatewayKeyStats> = {}): GatewayKeyStats {
  return { calls: 0, identity_mismatches: 0, first_seen_millis: null, last_seen_millis: null, ...over };
}

function keyEntry(over: Partial<GatewayKeyEntry> = {}): GatewayKeyEntry {
  return {
    key_id: "k-1",
    configured: true,
    bound: true,
    unit: "finops",
    agents: ["agent://acme.local/reporter"],
    created: "2026-01-02",
    since_startup: stats(),
    history: null,
    ...over,
  };
}

function report(over: Partial<GatewayKeysReport> = {}): GatewayKeysReport {
  return {
    strict_mode: "enforce",
    identity_map_configured: true,
    history_available: true,
    unauthorized_since_startup: { attempts: 0, last_millis: null },
    keys: [keyEntry()],
    ...over,
  };
}

const META = {
  environment: "console.acme.local",
  takenAt: "2026-08-26T12:00:00.000Z",
  snapshotAt: "2026-08-26T11:58:00.000Z",
  rescanAt: null,
};

/** Every caveat and window line of a meta, as one lower-cased blob - the
 * assertions below are about WHETHER the file states a thing, not about which
 * of the two lists it landed in. */
function saidIn(meta: { windows: string[]; caveats?: string[] }): string {
  return [...meta.windows, ...(meta.caveats ?? [])].join("\n").toLowerCase();
}

function cells(csv: string, rowIndex: number): string[] {
  const lines = csv.split("\n").filter((l) => !l.startsWith("#"));
  return lines[rowIndex].split(",");
}

// ------------------------------------------------------------- identities --

describe("the identities export carries what idryx sent", () => {
  it("carries created, last_used and runtime, which the table never rendered", () => {
    const [row] = identityExportRows([identity()]);
    expect(row.created).toBe("2026-01-02 03:04:05 UTC");
    expect(row.last_used).toBe("2026-08-01 09:00:00 UTC");
    expect(row.runtime).toBe("python3.12");
  });

  it("turns an unrecorded last_used into an empty cell, never into never or 0", () => {
    const [row] = identityExportRows([identity({ last_used: "", created: "", runtime: "" })]);
    expect(row.last_used).toBeNull();
    expect(row.created).toBeNull();
    expect(row.runtime).toBeNull();

    const csv = toCsv(IDENTITY_COLUMNS, identityExportRows([identity({ last_used: "" })]), identityExportMeta({ ...META, identities: [] }));
    const lastUsedAt = IDENTITY_COLUMNS.findIndex((c) => c.key === "last_used");
    expect(cells(csv, 1)[lastUsedAt]).toBe("");
  });

  it("carries both suggestion records whole, and nulls the one idryx did not send", () => {
    const [row] = identityExportRows([identity({ remediation: remediation(), rotation: null })]);
    expect(row.remediation_kind).toBe("right_size");
    expect(row.remediation_explanation).toBe("3 of 9 permissions unused");
    expect(row.remediation_code).toBe('resource "aws_iam_policy" {}');
    expect(row.remediation_created_at).toBe("2026-01-02 03:04:05 UTC");
    expect(row.rotation_kind).toBeNull();
    expect(row.rotation_explanation).toBeNull();
    expect(row.rotation_code).toBeNull();
    expect(row.rotation_created_at).toBeNull();
  });

  it("nulls a suggestion's created_at when idryx omitted it", () => {
    const [row] = identityExportRows([identity({ rotation: remediation({ kind: "rotation", created_at: "" }) })]);
    expect(row.rotation_kind).toBe("rotation");
    expect(row.rotation_created_at).toBeNull();
  });

  it("names the permissions, not only how many there are", () => {
    const [row] = identityExportRows([
      identity({
        permissions: [
          { name: "s3:GetObject", admin: false, used: true },
          { name: "iam:PassRole", admin: true, used: false },
        ],
      }),
    ]);
    expect(row.permissions_granted).toBe(2);
    expect(row.permissions_used).toBe(1);
    expect(row.permissions).toBe("iam:PassRole; s3:GetObject");
  });

  it("says an unused permission is unobserved when idryx captured no usage at all", () => {
    const noSignal = identityExportMeta({
      ...META,
      identities: [identity({ permissions: [{ name: "s3:GetObject", admin: false, used: false }] })],
    });
    expect(saidIn(noSignal)).toContain("unobserved");

    const withSignal = identityExportMeta({
      ...META,
      identities: [identity({ permissions: [{ name: "s3:GetObject", admin: false, used: true }] })],
    });
    expect(saidIn(withSignal)).not.toContain("unobserved");
  });

  it("says the console does not interpret the code field", () => {
    const meta = identityExportMeta({ ...META, identities: [identity({ remediation: remediation() })] });
    expect(saidIn(meta)).toContain("does not interpret");
  });

  it("says the file is every identity, not the filtered view", () => {
    expect(saidIn(identityExportMeta({ ...META, identities: [identity()] }))).toContain("filter");
  });
});

// ----------------------------------------------------------------- alerts --

describe("the alerts export says which pass produced it", () => {
  const alert: IdryxAlert = {
    detector: "stale_nhi",
    identity: "agent://acme.local/reporter",
    severity: "high",
    time: "2026-08-20T10:00:00Z",
    summary: "no activity in 90 days",
  };

  it("keeps every alert field", () => {
    expect(alertExportRows([alert])[0]).toMatchObject(alert);
  });

  it("says the alerts are the REST snapshot when no rescan has run", () => {
    const said = saidIn(alertExportMeta({ ...META, rescanAt: null }));
    expect(said).toContain("2026-08-26t11:58:00.000z");
    expect(said).not.toContain("detect pass");
  });

  it("says a rescan replaced them, and names when", () => {
    const said = saidIn(alertExportMeta({ ...META, rescanAt: "2026-08-26T12:30:00.000Z" }));
    expect(said).toContain("detect pass");
    expect(said).toContain("2026-08-26t12:30:00.000z");
  });

  it("says severity is per alert, not per detector", () => {
    expect(saidIn(alertExportMeta(META))).toContain("severity");
  });
});

// ----------------------------------------------------------- remediations --

describe("the remediations export carries the two fields the table dropped", () => {
  const rec: IdryxRecommendation = {
    identity: "svc://acme.local/reporter",
    kind: "right_size",
    explanation: "3 of 9 permissions unused",
    code: 'resource "aws_iam_policy" {}',
    created_at: "2026-01-02 03:04:05 UTC",
  };

  it("carries code and created_at", () => {
    const [row] = remediationExportRows([rec]);
    expect(row.code).toBe('resource "aws_iam_policy" {}');
    expect(row.created_at).toBe("2026-01-02 03:04:05 UTC");
  });

  it("nulls created_at when idryx omitted it", () => {
    const [row] = remediationExportRows([{ ...rec, created_at: "" }]);
    expect(row.created_at).toBeNull();
  });

  it("quotes a code block that carries a comma or a quote rather than splitting the row", () => {
    const csv = toCsv(
      REMEDIATION_COLUMNS,
      remediationExportRows([{ ...rec, code: 'resource "x" { a = 1, b = 2 }' }]),
      remediationExportMeta(META),
    );
    expect(csv).toContain('"resource ""x"" { a = 1, b = 2 }"');
  });
});

// ---------------------------------------------------------- access matrix --

describe("the access matrix export never turns an unread plane into a zero", () => {
  const agent = identity({
    id: "agent://acme.local/reporter",
    permissions: [{ name: "s3:GetObject", admin: false, used: true }],
  });

  function policy(over: Partial<PolicyRecord> = {}): PolicyRecord {
    return {
      id: "p-1",
      name: "reporter",
      target: "agent://acme.local/*",
      allow_domains: [],
      deny_tool: ["shell"],
      max_steps: 5,
      require_human_above_usd: 0,
      deny_above_usd: 0,
      deny_if_unattested: false,
      updated_at: null,
      ...over,
    };
  }

  it("leaves every wardryx column empty when the policy plane was never read", () => {
    const [row] = accessExportRows(buildAccessRows([agent], [], null));
    expect(row.denied_tools).toBeNull();
    expect(row.allow_domains_mode).toBeNull();
    expect(row.allow_domains).toBeNull();
    expect(row.max_steps).toBeNull();
    expect(row.deny_if_unattested).toBeNull();
  });

  it("writes those empties as empty cells, not as 0", () => {
    const csv = toCsv(
      ACCESS_COLUMNS,
      accessExportRows(buildAccessRows([agent], [], null)),
      accessExportMeta({ ...META, policyNote: "policy plane not configured", rows: [] }),
    );
    const at = ACCESS_COLUMNS.findIndex((c) => c.key === "denied_tools");
    expect(cells(csv, 1)[at]).toBe("");
  });

  it("separates unrestricted domains from a policy contradiction", () => {
    const unrestricted = accessExportRows(buildAccessRows([agent], [], [policy()]))[0];
    expect(unrestricted.allow_domains_mode).toBe("unrestricted");
    expect(unrestricted.allow_domains).toBeNull();

    const contradiction = accessExportRows(
      buildAccessRows([agent], [], [
        policy({ id: "p-a", allow_domains: ["a.example"] }),
        policy({ id: "p-b", allow_domains: ["b.example"] }),
      ]),
    )[0];
    expect(contradiction.allow_domains_mode).toBe("restricted");
    expect(contradiction.allow_domains).toBeNull();
  });

  it("says in the file that the wardryx columns were not checked", () => {
    const said = saidIn(
      accessExportMeta({ ...META, policyNote: "policy plane not configured", rows: [] }),
    );
    expect(said).toContain("policy plane not configured");
    expect(said).toContain("not zero");
  });

  it("says the two halves came from different passes after a rescan", () => {
    const said = saidIn(
      accessExportMeta({ ...META, rescanAt: "2026-08-26T12:30:00.000Z", policyNote: null, rows: [] }),
    );
    expect(said).toContain("detect pass");
  });

  it("flags the contradiction when a row actually has one", () => {
    const rows = accessExportRows(
      buildAccessRows([agent], [], [
        policy({ id: "p-a", allow_domains: ["a.example"] }),
        policy({ id: "p-b", allow_domains: ["b.example"] }),
      ]),
    );
    expect(saidIn(accessExportMeta({ ...META, policyNote: null, rows }))).toContain("no domain in common");
    expect(saidIn(accessExportMeta({ ...META, policyNote: null, rows: [] }))).not.toContain("no domain in common");
  });
});

// ------------------------------------------------------------------- keys --

describe("the keys export says which window each number covers", () => {
  const NOW = Date.parse("2026-08-26T12:00:00.000Z");

  it("leaves the history columns empty when this key has none, rather than zeroing them", () => {
    const [row] = keyExportRows(report({ keys: [keyEntry({ history: null, since_startup: stats({ calls: 4 }) })] }), NOW);
    expect(row.calls_history).toBeNull();
    expect(row.identity_mismatches_history).toBeNull();
    expect(row.last_seen_history).toBeNull();
    expect(row.calls_since_startup).toBe(4);
    expect(row.calls).toBe(4);
  });

  it("merges the two windows into calls, and says both halves", () => {
    const [row] = keyExportRows(
      report({
        keys: [
          keyEntry({
            since_startup: stats({ calls: 4, last_seen_millis: NOW - 1000 }),
            history: stats({ calls: 96, identity_mismatches: 2, last_seen_millis: NOW - 90_000 }),
          }),
        ],
      }),
      NOW,
    );
    expect(row.calls).toBe(100);
    expect(row.calls_since_startup).toBe(4);
    expect(row.calls_history).toBe(96);
    expect(row.identity_mismatches_history).toBe(2);
  });

  it("writes last seen as a timestamp a reader can use later, not as an age", () => {
    const [seen] = keyExportRows(
      report({ keys: [keyEntry({ since_startup: stats({ calls: 1, last_seen_millis: Date.parse("2026-08-25T08:00:00.000Z") }) })] }),
      NOW,
    );
    expect(seen.last_seen).toBe("2026-08-25T08:00:00.000Z");

    const [never] = keyExportRows(report({ keys: [keyEntry()] }), NOW);
    expect(never.last_seen).toBeNull();
  });

  it("says the status is this console's derivation, not the gateway's word", () => {
    expect(saidIn(keyExportMeta({ ...META, report: report(), nowMillis: NOW }))).toContain("derived by this console");
  });

  it("names strict_mode, whatever it is set to", () => {
    expect(saidIn(keyExportMeta({ ...META, report: report({ strict_mode: "warn" }), nowMillis: NOW }))).toContain("warn");
    expect(saidIn(keyExportMeta({ ...META, report: report({ strict_mode: "off" }), nowMillis: NOW }))).toContain("off");
  });

  it("warns that a busy key reads as never used when the gateway keeps no history", () => {
    const without = saidIn(keyExportMeta({ ...META, report: report({ history_available: false }), nowMillis: NOW }));
    expect(without).toContain("since the gateway process started");
    expect(without).toContain("never used");

    const with_ = saidIn(keyExportMeta({ ...META, report: report({ history_available: true }), nowMillis: NOW }));
    expect(with_).not.toContain("never used");
  });

  it("says the bound, unit and agents columns are vacuous when there is no identity map", () => {
    const off = saidIn(keyExportMeta({ ...META, report: report({ identity_map_configured: false }), nowMillis: NOW }));
    expect(off).toContain("no identity map");
    expect(off).toContain("unbound");

    const on = saidIn(keyExportMeta({ ...META, report: report({ identity_map_configured: true }), nowMillis: NOW }));
    expect(on).not.toContain("no identity map");
  });

  it("accounts for the unauthorized attempts that belong to no row", () => {
    const said = saidIn(
      keyExportMeta({
        ...META,
        report: report({ unauthorized_since_startup: { attempts: 7, last_millis: NOW } }),
        nowMillis: NOW,
      }),
    );
    expect(said).toContain("7 unauthorized");
    expect(said).toContain("no row");
  });

  it("dates the staleness cutoff, because stale is relative to when this was taken", () => {
    expect(saidIn(keyExportMeta({ ...META, report: report(), nowMillis: NOW }))).toContain("2026-08-26t12:00:00.000z");
  });
});

// ------------------------------------------------------------- structural --

describe("the file a reader opens", () => {
  it("leads with the provenance block on every table", () => {
    const csv = toCsv(IDENTITY_COLUMNS, identityExportRows([identity()]), identityExportMeta({ ...META, identities: [identity()] }));
    expect(csv.startsWith("# subject: ")).toBe(true);
    expect(csv).toContain("# environment: console.acme.local");
    expect(csv).toContain("# taken_at: 2026-08-26T12:00:00.000Z");
  });

  it("has a row key behind every column it prints", () => {
    const pairs: [string, ExportColumn<Record<string, unknown>>[], Record<string, unknown>[]][] = [
      ["identities", IDENTITY_COLUMNS as never, identityExportRows([identity()]) as never],
      ["alerts", ALERT_COLUMNS as never, alertExportRows([
        { detector: "stale_nhi", identity: "a", severity: "high", time: "t", summary: "s" },
      ]) as never],
      ["remediations", REMEDIATION_COLUMNS as never, remediationExportRows([
        { identity: "a", kind: "rotation", explanation: "e", code: "c", created_at: "" },
      ]) as never],
      ["access", ACCESS_COLUMNS as never, accessExportRows(buildAccessRows([identity()], [], null)) as never],
      ["keys", KEY_COLUMNS as never, keyExportRows(report(), Date.now()) as never],
    ];
    for (const [name, columns, rows] of pairs) {
      expect(rows.length, `${name} produced no row to check`).toBeGreaterThan(0);
      for (const column of columns) {
        expect(Object.prototype.hasOwnProperty.call(rows[0], column.key), `${name}.${column.key}`).toBe(true);
      }
    }
  });
});
