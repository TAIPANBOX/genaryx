import { describe, expect, it } from "vitest";
import { groupRows, rowsFromOwners, sortRows, NO_CHAIN_KEY, NO_OWNER_KEY, NO_UNIT_KEY } from "./stats";
import { toCsv } from "./download";
import type { AgentStats } from "../statsTypes";
import type { IdryxIdentity } from "../identityTypes";
import type { Run } from "../moneyTypes";

// ---------------------------------------------------------------------------
// Fixtures. Three agents in two teams, two of them owned, one not.
// ---------------------------------------------------------------------------

const FRAUD_BOT = "agent://acme.local/fraud/scorer";
const KYC_BOT = "agent://acme.local/kyc-aml/checker";
const ORPHAN = "agent://acme.local/sre/janitor";

function run(agentId: string, spent: number, calls: number, budget: number | null = null): Run {
  return {
    run_id: `run-${agentId}-${spent}`,
    model: "test",
    agent_id: agentId,
    spent_usd: spent,
    budget_usd: budget,
    calls,
    cache_hits: 0,
    steps: 1,
    last_seen: "2026-08-09T10:00:00Z",
    killed: false,
  };
}

function identity(id: string, owner: string): IdryxIdentity {
  return {
    id,
    type: "agent",
    source: "agents",
    owner,
    privileged: false,
    perms: [],
    events: 0,
    alerts: 0,
    on_behalf_of: [],
  } as unknown as IdryxIdentity;
}

function counts(agentId: string, over: Partial<AgentStats> = {}): AgentStats {
  return {
    agent_id: agentId,
    blocked: 0,
    blocked_by_operator: 0,
    anomalies: 0,
    budget_events: 0,
    worst_overshoot_microusd: null,
    by_type: {},
    by_detector: {},
    last_seen: "2026-08-09T10:00:00Z",
    ...over,
  };
}

const RUNS: Run[] = [run(FRAUD_BOT, 10, 100), run(KYC_BOT, 5, 50), run(ORPHAN, 2, 20)];

// `d.hayes` as a full URI and `m.okafor` as a bare handle: idryx records both
// shapes, and both must land on the same key style.
const IDENTITIES: IdryxIdentity[] = [
  identity(FRAUD_BOT, "user://acme.local/d.hayes"),
  identity(KYC_BOT, "d.hayes"),
];

const COUNTS: AgentStats[] = [
  counts(FRAUD_BOT, {
    blocked: 3,
    blocked_by_operator: 1,
    anomalies: 1,
    budget_events: 2,
    worst_overshoot_microusd: 250_000,
  }),
  counts(KYC_BOT, { blocked: 1 }),
  counts(ORPHAN, { blocked: 7, anomalies: 2 }),
];

describe("groupRows", () => {
  it("gives every agent its own row when grouping by agent", () => {
    const rows = groupRows(RUNS, IDENTITIES, COUNTS, "agent");
    expect(rows).toHaveLength(3);
    const fraud = rows.find((r) => r.key === FRAUD_BOT)!;
    expect(fraud.spentUsd).toBe(10);
    expect(fraud.blocked).toBe(3);
    expect(fraud.agentCount).toBe(1);
    expect(rows.every((r) => !r.unattributed)).toBe(true);
  });

  it("rolls both of an owner's agents into one row, whichever shape idryx spelled the owner in", () => {
    const rows = groupRows(RUNS, IDENTITIES, COUNTS, "owner");
    const hayes = rows.find((r) => r.key === "d.hayes");
    expect(hayes, "user://acme.local/d.hayes and d.hayes are one person").toBeDefined();
    expect(hayes!.agentCount).toBe(2);
    expect(hayes!.spentUsd).toBe(15);
    expect(hayes!.blocked).toBe(4);
    expect(
      hayes!.blockedByOperator,
      "one of the four was a person's decision, the rest were the services'",
    ).toBe(1);
  });

  // The property that keeps this table's totals honest against the Money tab.
  it("keeps an unowned agent as a visible row rather than dropping it", () => {
    const rows = groupRows(RUNS, IDENTITIES, COUNTS, "owner");
    const orphan = rows.find((r) => r.key === NO_OWNER_KEY);
    expect(orphan, "an agent idryx has no owner for must still appear").toBeDefined();
    expect(orphan!.unattributed).toBe(true);
    expect(orphan!.blocked).toBe(7);
    expect(rows.reduce((a, r) => a + r.spentUsd, 0)).toBe(17);
  });

  it("groups by business unit, folding the two financial-crime teams into one", () => {
    const rows = groupRows(RUNS, IDENTITIES, COUNTS, "unit");
    const fc = rows.find((r) => r.key === "financial-crime");
    expect(fc, "fraud and kyc-aml both map to financial-crime").toBeDefined();
    expect(fc!.agentCount).toBe(2);
    expect(fc!.spentUsd).toBe(15);
    expect(rows.find((r) => r.key === "sre")).toBeDefined();
    expect(rows.find((r) => r.key === NO_UNIT_KEY)).toBeUndefined();
  });

  // Idryx ships one wire type for twenty-five detectors, so the count alone
  // says nothing. The names have to survive the grouping to be worth anything.
  it("sums idryx detector names across a group rather than dropping them", () => {
    const rows = groupRows(
      RUNS,
      IDENTITIES,
      [
        counts(FRAUD_BOT, { anomalies: 2, by_detector: { impossible_travel: 1, mfa_fatigue: 1 } }),
        counts(KYC_BOT, { anomalies: 1, by_detector: { impossible_travel: 1 } }),
      ],
      "owner",
    );
    const hayes = rows.find((r) => r.key === "d.hayes")!;
    expect(
      hayes.detectors.impossible_travel,
      "the same detector on two of their agents is two",
    ).toBe(2);
    expect(hayes.detectors.mfa_fatigue).toBe(1);
  });

  it("shows an agent that only the bus knows about, with zero spend", () => {
    const busOnly = "agent://acme.local/sre/ghost";
    const rows = groupRows(RUNS, IDENTITIES, [...COUNTS, counts(busOnly, { blocked: 9 })], "agent");
    const ghost = rows.find((r) => r.key === busOnly);
    expect(ghost, "an agent blocked 9 times with no run is exactly the row to show").toBeDefined();
    expect(ghost!.spentUsd).toBe(0);
    expect(ghost!.blocked).toBe(9);
  });

  // `null + null` must stay null. Turning "nobody wrote it down" into 0 would
  // invent a measurement, and 0 in this column reads as "never went over".
  it("takes the worst breach in a group, never the sum, and never invents a zero", () => {
    const rows = groupRows(RUNS, IDENTITIES, COUNTS, "owner");
    const hayes = rows.find((r) => r.key === "d.hayes")!;
    expect(hayes.worstOvershootMicrousd).toBe(250_000);

    const orphan = rows.find((r) => r.key === NO_OWNER_KEY)!;
    expect(orphan.worstOvershootMicrousd, "no event carried amounts, so not zero").toBeNull();
  });
});

describe("sortRows", () => {
  it("pins the unattributed row last however the table is sorted", () => {
    const rows = groupRows(RUNS, IDENTITIES, COUNTS, "owner");
    for (const [key, desc] of [
      ["blocked", true],
      ["blocked", false],
      ["spentUsd", true],
      ["label", false],
    ] as const) {
      const sorted = sortRows(rows, key, desc);
      expect(
        sorted[sorted.length - 1].unattributed,
        `sorting by ${key} ${desc ? "desc" : "asc"} must not let "(no owner in idryx)" top the table`,
      ).toBe(true);
    }
  });

  it("sorts an absent overshoot after any number, never as zero", () => {
    const rows = groupRows(RUNS, IDENTITIES, COUNTS, "agent");
    const sorted = sortRows(rows, "worstOvershootMicrousd", false);
    expect(sorted[0].worstOvershootMicrousd).toBe(250_000);
    expect(sorted[sorted.length - 1].worstOvershootMicrousd).toBeNull();
  });
});

describe("export", () => {
  const META = {
    subject: "test",
    environment: "localhost",
    takenAt: "2026-08-09T10:00:00.000Z",
    windows: ["spend: money plane", "counts: bus, since start"],
    caveats: ["an empty cell is not a zero"],
  };

  it("carries the provenance block, so the file is readable away from the console", () => {
    const csv = toCsv([{ key: "name" as const, header: "name" }], [{ name: "a" }], META);
    expect(csv).toContain("# subject: test");
    expect(csv).toContain("# window: spend: money plane");
    expect(csv).toContain("# caveat: an empty cell is not a zero");
  });

  it("writes an unrecorded value as an empty cell, not as 0", () => {
    const csv = toCsv(
      [
        { key: "name" as const, header: "name" },
        { key: "over" as const, header: "over" },
      ],
      [{ name: "a", over: null }],
      META,
    );
    expect(csv.trim().endsWith("a,")).toBe(true);
  });

  it("quotes a value carrying a comma so the row cannot split", () => {
    const csv = toCsv(
      [{ key: "name" as const, header: "name" }],
      [{ name: 'fraud, kyc-aml and "ops"' }],
      META,
    );
    expect(csv).toContain('"fraud, kyc-aml and ""ops"""');
  });
});

describe("rowsFromOwners", () => {
  const OWNERS = [
    {
      owner: "user://acme.local/d.hayes",
      spent_usd: 15,
      calls: 150,
      runs: 4,
      agents: 2,
      last_seen: "2026-08-09T10:00:00Z",
      tool_calls: 60,
    },
    {
      owner: "unassigned",
      spent_usd: 2,
      calls: 20,
      runs: 1,
      agents: 1,
      last_seen: "2026-08-09T09:00:00Z",
      tool_calls: 8,
    },
  ];

  it("passes the money plane's own totals through without re-folding them", () => {
    const rows = rowsFromOwners(OWNERS);
    const hayes = rows.find((r) => r.key === "user://acme.local/d.hayes")!;
    expect(hayes.spentUsd).toBe(15);
    expect(hayes.calls).toBe(150);
    expect(hayes.runs).toBe(4);
    expect(hayes.agentCount, "distinct agents that ran on their behalf").toBe(2);
  });

  // The property that keeps this grouping from claiming something nobody
  // measured: the bus counts are per agent, and this rollup has no agent list.
  it("marks the count columns inapplicable rather than reporting zero", () => {
    for (const row of rowsFromOwners(OWNERS)) {
      expect(
        row.countsApply,
        "a 0 in the blocked column here would say 'never stopped', which nothing measured",
      ).toBe(false);
    }
  });

  it("renders the plane's 'unassigned' as a sentence and pins it last", () => {
    const rows = rowsFromOwners(OWNERS);
    const none = rows.find((r) => r.key === "unassigned")!;
    expect(none.unattributed).toBe(true);
    expect(none.label).toBe(NO_CHAIN_KEY);
    expect(none.label).not.toBe("unassigned");

    const sorted = sortRows(rows, "spentUsd", false);
    expect(sorted[sorted.length - 1].unattributed).toBe(true);
  });

  // The two owner answers are separate on purpose. A test that asserted they
  // MATCH would be wrong: an agent owned by one person and run on another's
  // behalf lands under different names, and that is the point.
  it("is a different question from the idryx owner grouping, and stays one", () => {
    const viaIdryx = groupRows(RUNS, IDENTITIES, COUNTS, "owner");
    const viaChain = rowsFromOwners(OWNERS);
    expect(viaIdryx.some((r) => r.countsApply)).toBe(true);
    expect(viaChain.every((r) => !r.countsApply)).toBe(true);
  });
});
