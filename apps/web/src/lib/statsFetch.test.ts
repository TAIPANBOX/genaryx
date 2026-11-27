/**
 * The Statistics panel's read and its period split (`lib/stats.ts`): the read
 * asks for every event in the window, refuses an answer it cannot read rather
 * than show "never stopped", and a Cloud that cannot fold by period gives no
 * period split at all, so the panel cannot put a period label over lifetime
 * numbers. Rows with no value sort last whichever way the column is sorted.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const box = vi.hoisted(() => ({
  present: true,
  calls: [] as { command: string; args: unknown }[],
  answer: (): unknown => ({ measured: true }),
}));

vi.mock("./transport", () => ({
  hasBackend: () => box.present,
  isWebShell: () => box.present,
  webApiBase: () => "",
  subscribeBackend: async () => () => {},
  requiredRoleFromCommandError: () => null,
  invokeBackend: async (command: string, args?: unknown) => {
    box.calls.push({ command, args });
    const a = box.answer();
    if (a instanceof Error || (a && typeof a === "object" && "kind" in a)) throw a;
    return a;
  },
}));

import { STATS_SCAN, fetchStats, sortRows, windowedSpendByAgent, type StatsRow } from "./stats";

beforeEach(() => {
  box.present = true;
  box.calls = [];
  box.answer = () => ({ measured: true });
});

describe("fetchStats", () => {
  it("asks for the whole scan over the window it was given", async () => {
    expect(await fetchStats()).toEqual({ measured: true });
    expect(await fetchStats(500, 30)).toEqual({ measured: true });
    expect(box.calls).toEqual([
      { command: "stats_counts", args: { scan: STATS_SCAN, window_days: 0 } },
      { command: "stats_counts", args: { scan: 500, window_days: 30 } },
    ]);
  });

  it("refuses an answer that is not a stats panel instead of reporting nothing stopped", async () => {
    for (const answer of [null, "text", { rows: [] }]) {
      box.answer = () => answer;
      await expect(fetchStats()).rejects.toMatchObject({
        kind: "backend",
        message: expect.stringContaining("not a report that your agents were never stopped"),
      });
    }
  });

  it("passes the plane's own error through and wraps anything else", async () => {
    box.answer = () => ({ kind: "store_closed", message: "s" });
    await expect(fetchStats()).rejects.toEqual({ kind: "store_closed", message: "s" });
    box.answer = () => new Error("socket");
    await expect(fetchStats()).rejects.toEqual({ kind: "backend", message: "socket" });
  });

  it("with no box refuses before asking anything", async () => {
    box.present = false;
    await expect(fetchStats()).rejects.toEqual({ kind: "no_environment" });
    expect(box.calls).toEqual([]);
  });
});

describe("windowedSpendByAgent", () => {
  it("gives no split at all when the Cloud cannot fold by period", () => {
    expect(windowedSpendByAgent(null)).toBeNull();
  });

  it("converts micro-dollars and leaves the Cloud's overflow bucket out", () => {
    const m = windowedSpendByAgent({
      agents: [
        { agent_id: "agent://a/x", spent_microusd: 2_500_000, calls: 7 },
        { agent_id: "", spent_microusd: 99_000_000, calls: 400 },
      ],
    } as never)!;
    expect([...m.entries()]).toEqual([["agent://a/x", { spentUsd: 2.5, calls: 7 }]]);
  });
});

describe("sortRows", () => {
  const row = (label: string, anomalies: number | null, unattributed = false) =>
    ({ key: label, label, anomalies, unattributed }) as unknown as StatsRow;

  it("puts rows with no value last in both directions, and the unattributed row after everything", () => {
    const rows = [row("b", null), row("a", 3), row("zz", 9, true), row("c", 1), row("d", null)];
    const asc = sortRows(rows, "anomalies" as never, false).map((r) => r.label);
    const desc = sortRows(rows, "anomalies" as never, true).map((r) => r.label);
    expect(asc.slice(0, 2)).toEqual(["c", "a"]);
    expect(desc.slice(0, 2)).toEqual(["a", "c"]);
    expect(asc.slice(2, 4).sort()).toEqual(["b", "d"]);
    expect(desc.slice(2, 4).sort()).toEqual(["b", "d"]);
    expect(asc[4]).toBe("zz");
    expect(desc[4]).toBe("zz");
  });

  it("sorts by label both ways and leaves the input untouched", () => {
    const rows = [row("beta", 0), row("alpha", 0), row("gamma", 0)];
    expect(sortRows(rows, "label", false).map((r) => r.label)).toEqual(["alpha", "beta", "gamma"]);
    expect(sortRows(rows, "label", true).map((r) => r.label)).toEqual(["gamma", "beta", "alpha"]);
    expect(rows.map((r) => r.label)).toEqual(["beta", "alpha", "gamma"]);
  });
});
