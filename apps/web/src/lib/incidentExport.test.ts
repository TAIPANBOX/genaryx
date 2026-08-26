import { describe, expect, it } from "vitest";

import {
  anomalyListMeta,
  incidentLinkTarget,
  incidentExport,
  incidentExportMeta,
  incidentExportName,
} from "./incidentExport";
import type { UnifiedIncident } from "./incidents";

const ROW = {
  id: "money:inc-1",
  source: "money",
  severity: "high",
  title: "budget exhausted",
  detail: "run-42 stopped at the cap",
  ts: "2026-08-26T12:00:00Z",
  occurrences: 3,
  ackable: true,
  explainable: true,
  raw: { incident_id: "inc-1", run_id: "run-42" },
} as unknown as UnifiedIncident;

describe("incidentExport", () => {
  it("carries what the card showed, not just the row", () => {
    const out = incidentExport({
      row: ROW,
      subject: "agent://acme.example/biller",
      chain: ["user://acme.example/alice", "agent://acme.example/biller"],
      run: { run_id: "run-42" } as never,
      record: { agent_id: "agent://acme.example/biller" } as never,
      busEvents: [{ type: "budget_exhausted" } as never],
    });
    expect(out.incident.id).toBe("money:inc-1");
    expect(out.subject).toBe("agent://acme.example/biller");
    expect(out.delegation).toEqual([
      "user://acme.example/alice",
      "agent://acme.example/biller",
    ]);
    expect(out.run).not.toBeNull();
    expect(out.agent_record).not.toBeNull();
    expect(out.bus_events).toHaveLength(1);
  });

  it("says what it could not find rather than leaving a hole", () => {
    // The whole reason this file has a caveats list. A card that showed
    // "no run recorded" and a file that just omits `run` are different
    // statements, and the second one reads as though nobody looked.
    const meta = incidentExportMeta({
      row: ROW,
      subject: null,
      chain: [],
      run: null,
      record: null,
      busEvents: null,
      environment: "demo",
      takenAt: "2026-08-26T13:00:00Z",
    });
    const joined = (meta.caveats ?? []).join(" | ");
    expect(joined).toContain("run");
    expect(joined).toContain("agent record");
    expect(joined).toContain("bus");
    expect(meta.subject).toContain("budget exhausted");
    expect(meta.takenAt).toBe("2026-08-26T13:00:00Z");
  });

  it("does not claim a delegation chain the incident never carried", () => {
    const meta = incidentExportMeta({
      row: ROW,
      subject: "agent://acme.example/biller",
      chain: [],
      run: { run_id: "run-42" } as never,
      record: { agent_id: "x" } as never,
      busEvents: [],
      environment: "demo",
      takenAt: "2026-08-26T13:00:00Z",
    });
    expect((meta.caveats ?? []).join(" | ")).toContain("delegation");
  });
});

describe("anomalyListMeta", () => {
  it("records the filters, so a subset cannot read as the whole", () => {
    // The failure this exists to prevent: somebody filters to `critical`,
    // exports, mails the file, and the reader counts two anomalies in an
    // estate that had forty.
    const meta = anomalyListMeta({
      shown: 2,
      total: 40,
      planes: ["money"],
      severities: ["critical"],
      query: "biller",
      environment: "demo",
      takenAt: "2026-08-26T13:00:00Z",
      busRead: 500,
      busTruncated: true,
    });
    const text = [meta.subject, ...meta.windows, ...(meta.caveats ?? [])].join(" | ");
    expect(text).toContain("2 of 40");
    expect(text).toContain("money");
    expect(text).toContain("critical");
    expect(text).toContain("biller");
  });

  it("says when the bus was capped, because the cap is not the estate", () => {
    const meta = anomalyListMeta({
      shown: 10,
      total: 10,
      planes: [],
      severities: [],
      query: "",
      environment: "demo",
      takenAt: "2026-08-26T13:00:00Z",
      busRead: 500,
      busTruncated: true,
    });
    expect((meta.caveats ?? []).join(" | ")).toContain("capped");
  });

  it("does not invent a filter that was not applied", () => {
    const meta = anomalyListMeta({
      shown: 10,
      total: 10,
      planes: [],
      severities: [],
      query: "",
      environment: "demo",
      takenAt: "2026-08-26T13:00:00Z",
      busRead: 10,
      busTruncated: false,
    });
    const text = [...meta.windows, ...(meta.caveats ?? [])].join(" | ");
    expect(text).not.toContain("severity");
    expect(text).not.toContain("capped");
  });
});

describe("incidentLinkTarget", () => {
  const bus = {
    source: "bus",
    id: "bus:104076",
    raw: { type: "budget_exhausted", run_id: "run-42", source: "tokenfuse" },
  } as unknown as UnifiedIncident;
  const money = {
    source: "money",
    id: "money:inc-1",
    raw: { kind: "breaker_tripped", run_id: "run-9", agent_id: "agent://a/b" },
  } as unknown as UnifiedIncident;
  const idryx = {
    source: "idryx",
    id: "idryx:1",
    raw: { detector: "excessive_privilege", identity: "agent://a/b" },
  } as unknown as UnifiedIncident;
  const posture = { source: "posture", id: "posture:x", raw: {} } as unknown as UnifiedIncident;

  it("addresses an anomaly by its EVENT TYPE, not by the console's row id", () => {
    // The row id (`bus:104076`) is this console's own bookkeeping. The link
    // scheme reads `{type}:{subject}` where the type is what a plane EMITTED,
    // so a link built from the row id lands on the overview saying it cannot
    // place the id. That is what it did, live, before this function existed.
    expect(incidentLinkTarget(bus)).toBe("budget_exhausted:run-42");
    expect(incidentLinkTarget(money)).toBe("breaker_tripped:run-9");
    expect(incidentLinkTarget(idryx)).toBe("excessive_privilege:agent://a/b");
  });

  it("refuses to address a posture finding, which has none", () => {
    // A posture finding is a computed state, not an event: no id in a store,
    // nothing to re-open. Offering a link would hand somebody an address that
    // resolves to a different answer tomorrow.
    expect(incidentLinkTarget(posture)).toBeNull();
  });

  it("refuses rather than guessing when the subject is missing", () => {
    const orphan = {
      source: "bus",
      id: "bus:1",
      raw: { type: "budget_exhausted", source: "tokenfuse" },
    } as unknown as UnifiedIncident;
    expect(incidentLinkTarget(orphan)).toBeNull();
  });
});

describe("incidentExportName", () => {
  it("names the agent and the moment, because that is how a file is found again", () => {
    // `@yurii 2026-08-26`: the saved file should carry the agent and when the
    // save was made. A name built from the console's row id says nothing to
    // somebody looking at a folder a week later.
    const name = incidentExportName(
      ROW,
      "2026-08-26T15:39:37.179Z",
      "agent://meridian.io/sre/rca-copilot",
    );
    expect(name).toContain("meridian.io-sre-rca-copilot");
    expect(name).toContain("2026-08-26T15-39-37");
    expect(name.endsWith(".json")).toBe(true);
    // Colons are legal in a URL and illegal in a Windows filename, and a
    // browser silently rewrites them, so the name says what it will be.
    expect(name).not.toContain(":");
    expect(name).not.toContain("/");
  });

  it("falls back to the incident when there is no agent to name", () => {
    const name = incidentExportName(ROW, "2026-08-26T15:39:37.179Z", null);
    expect(name).toContain("money-inc-1");
    expect(name).toContain("2026-08-26T15-39-37");
  });

  it("distinguishes two saves made in the same day", () => {
    const a = incidentExportName(ROW, "2026-08-26T15:39:37.179Z", "agent://a/b");
    const b = incidentExportName(ROW, "2026-08-26T16:02:11.000Z", "agent://a/b");
    expect(a).not.toBe(b);
  });
});
