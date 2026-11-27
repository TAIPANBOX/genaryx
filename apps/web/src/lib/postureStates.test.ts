/**
 * The Posture view's checks (`lib/posture.ts`) for the states the demo never
 * reaches: a plane that is unreachable or not configured, a feed that is empty,
 * and timestamps nobody can read. A check that cannot tell says "unknown"
 * rather than "ok"; an unreachable plane, an empty detector feed and a failed
 * tunnel are "triggered"; and a row whose time does not parse is never
 * counted as fresh.
 */

import { describe, expect, it } from "vitest";
import { computePostureFindings, type PostureFinding, type PostureInput } from "./posture";

const NOW = Date.parse("2026-11-27T12:00:00Z");

function input(over: Partial<PostureInput> = {}): PostureInput {
  return {
    moneyStatus: null,
    policyStatus: null,
    policies: null,
    enforcement: null,
    busLoaded: false,
    busEventCount: 0,
    lastEventAtMs: null,
    schemasSeen: new Set(),
    nowMs: NOW,
    identityStatus: null,
    identities: null,
    identityAlerts: null,
    identitySnapshotAsOfMs: null,
    qualityStatus: null,
    cryptoStatus: null,
    memoryStatus: null,
    drillsStatus: null,
    copilotStatus: null,
    remoteStatus: null,
    moneyRuns: null,
    approvals: null,
    credentialsStatus: null,
    keysReport: null,
    ...over,
  } as PostureInput;
}

function byId(findings: PostureFinding[]): Record<string, PostureFinding> {
  return Object.fromEntries(findings.map((f) => [f.id, f]));
}

const unreachable = { state: "unreachable", reason: "connection refused", source: { source: "env_fallback" } } as never;
const notConfigured = { state: "no_environment" } as never;

describe("plane health", () => {
  const planes: [string, keyof PostureInput][] = [
    ["policy_plane_health", "policyStatus"],
    ["identity_plane_health", "identityStatus"],
    ["quality_plane_health", "qualityStatus"],
    ["memory_plane_health", "memoryStatus"],
    ["credentials_plane_health", "credentialsStatus"],
  ];

  it.each(planes)("%s: unreachable is triggered and names the reason", (id, key) => {
    const f = byId(computePostureFindings(input({ [key]: unreachable } as Partial<PostureInput>)))[id];
    expect(f.state).toBe("triggered");
    expect(f.detail).toContain("connection refused");
  });

  it.each(planes)("%s: not configured is unknown, never ok", (id, key) => {
    const f = byId(computePostureFindings(input({ [key]: notConfigured } as Partial<PostureInput>)))[id];
    expect(f.state).toBe("unknown");
    expect(f.detail).toContain("not configured");
  });

  it("money: a failed pairing is triggered, an absent Cloud is unknown", () => {
    const failed = byId(computePostureFindings(input({ moneyStatus: { state: "pairing_failed", reason: "bad key" } as never })));
    expect(failed.money_plane_health.state).toBe("triggered");
    expect(failed.money_plane_health.detail).toContain("bad key");
    const absent = byId(computePostureFindings(input({ moneyStatus: notConfigured })));
    expect(absent.money_plane_health.state).toBe("unknown");
  });

  it("crypto and drills: not configured is unknown", () => {
    const f = byId(computePostureFindings(input({ cryptoStatus: notConfigured, drillsStatus: notConfigured })));
    expect(f.crypto_plane_health.state).toBe("unknown");
    expect(f.drills_plane_health.state).toBe("unknown");
  });

  it("Felyx: off says why, on says which provider", () => {
    const off = byId(
      computePostureFindings(input({ copilotStatus: { enabled: false, disabled_reason: "no key", provider: null, local: null } as never })),
    );
    expect(off.copilot_plane_health.detail).toBe("disabled: no key");
    const offQuiet = byId(computePostureFindings(input({ copilotStatus: { enabled: false, disabled_reason: null } as never })));
    expect(offQuiet.copilot_plane_health.detail).toBe("disabled.");
    const on = byId(computePostureFindings(input({ copilotStatus: { enabled: true, provider: null, local: true } as never })));
    expect(on.copilot_plane_health.state).toBe("ok");
    expect(on.copilot_plane_health.detail).toBe("enabled (unknown provider, local).");
  });

  it("remote: no saved environment is unknown, a failed tunnel is triggered", () => {
    const none = byId(computePostureFindings(input({ remoteStatus: { state: "ready", environment: null, tunnel: { state: "disconnected" } } as never })));
    expect(none.remote_plane_health.state).toBe("unknown");
    expect(none.remote_plane_health.detail).toContain("no remote environment saved");
    const failed = byId(
      computePostureFindings(
        input({ remoteStatus: { state: "ready", environment: { name: "x" }, tunnel: { state: "failed", message: "handshake timeout" } } as never }),
      ),
    );
    expect(failed.remote_plane_health.state).toBe("triggered");
    expect(failed.remote_plane_health.detail).toContain("handshake timeout");
  });
});

describe("feeds and freshness", () => {
  it("a loaded but empty bus is stale", () => {
    expect(byId(computePostureFindings(input({ busLoaded: true, busEventCount: 0 }))).bus_stale.state).toBe("triggered");
    expect(byId(computePostureFindings(input({ busLoaded: false }))).bus_stale.state).toBe("unknown");
  });

  it("an empty detector feed is triggered, and alerts whose times do not parse are not fresh", () => {
    const ready = { state: "ready" } as never;
    const empty = byId(computePostureFindings(input({ identityStatus: ready, identityAlerts: [] })));
    expect(empty.detector_feed_freshness.state).toBe("triggered");
    expect(empty.detector_feed_freshness.detail).toContain("no detector alerts recorded");
    const garbled = byId(computePostureFindings(input({ identityStatus: ready, identityAlerts: [{ time: "not a time" }] as never })));
    expect(garbled.detector_feed_freshness.state).not.toBe("ok");
    expect(garbled.detector_feed_freshness.detail).toContain("unparseable timestamp");
  });

  it("the Cloud's runs: none in the window, or none with a readable time, is never fresh", () => {
    const none = byId(computePostureFindings(input({ moneyRuns: [] })));
    expect(none.cloud_ingest_freshness.state).not.toBe("ok");
    expect(none.cloud_ingest_freshness.detail).toBe("no runs in the current window.");
    const garbled = byId(computePostureFindings(input({ moneyRuns: [{ last_seen: "?" }] as never })));
    expect(garbled.cloud_ingest_freshness.detail).toBe("every run has an unparseable last_seen.");
  });

  it("approvals: none pending is ok; pending with unreadable times says so", () => {
    expect(byId(computePostureFindings(input({ approvals: [] }))).approvals_waiting.state).toBe("ok");
    const garbled = byId(computePostureFindings(input({ approvals: [{ pending: true, requested_at: "?" }] as never })));
    expect(garbled.approvals_waiting.detail).toBe("every pending approval has an unparseable requested_at.");
  });

  it("with nothing loaded at all, no plane claims to be healthy", () => {
    const findings = computePostureFindings(input()).filter((f) => f.id.endsWith("_plane_health"));
    expect(findings.length).toBeGreaterThan(8);
    for (const f of findings) expect([f.id, f.state]).toEqual([f.id, "unknown"]);
  });
});
