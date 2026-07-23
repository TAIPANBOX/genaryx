import { describe, expect, it } from "vitest";
import type { RoutineRunDto, RoutineSummaryDto } from "../routinesTypes";
import {
  latestDetailLine,
  latestRelativeTime,
  recordStatusTone,
  ROUTINE_STATUS_ORDER,
  ROUTINE_STATUS_TONE,
  routineStatusRank,
  sortRoutinesWorstFirst,
  toUiStatus,
} from "./routines";

// ---------------------------------------------------------------------------
// Test fixtures - minimal, valid instances of each wire type, overridable.
// ---------------------------------------------------------------------------

function run(overrides: Partial<RoutineRunDto> & { routine: string }): RoutineRunDto {
  return {
    schema: overrides.schema ?? "stackup.routine-run/v1",
    routine: overrides.routine,
    started_at: overrides.started_at ?? "2026-07-23T06:07:00Z",
    finished_at: overrides.finished_at ?? "2026-07-23T06:07:04Z",
    exit_code: overrides.exit_code ?? 0,
    status: overrides.status ?? "ok",
    reason: overrides.reason ?? null,
    artifact: overrides.artifact ?? null,
    summary: overrides.summary ?? null,
  };
}

function summary(overrides: Partial<RoutineSummaryDto> & { name: string }): RoutineSummaryDto {
  return {
    name: overrides.name,
    installed: overrides.installed ?? false,
    latest: overrides.latest ?? null,
    latest_error: overrides.latest_error ?? null,
  };
}

// ---------------------------------------------------------------------------
// toUiStatus: the exact precedence (unreadable > never > the real status)
// ---------------------------------------------------------------------------

describe("toUiStatus", () => {
  it("is 'never' when there is no record at all", () => {
    expect(toUiStatus({ latest: null, latest_error: null })).toBe("never");
  });

  it("is 'unreadable' when latest_error is set, even if latest_error is the only signal", () => {
    expect(toUiStatus({ latest: null, latest_error: "could not parse status/x.json: EOF" })).toBe("unreadable");
  });

  it("latest_error wins over a present latest, defensively (the two are mutually exclusive on the real wire)", () => {
    const latest = run({ routine: "focus-export", status: "ok" });
    expect(toUiStatus({ latest, latest_error: "should not normally coexist with latest" })).toBe("unreadable");
  });

  it("maps each of the four known statuses straight through", () => {
    for (const status of ["ok", "findings", "skipped", "error"] as const) {
      const latest = run({ routine: "qryx-trend", status });
      expect(toUiStatus({ latest, latest_error: null })).toBe(status);
    }
  });

  it("maps an unrecognized status string to 'unknown', never rejecting it", () => {
    const latest = run({ routine: "idryx-detect", status: "a-future-status" });
    expect(toUiStatus({ latest, latest_error: null })).toBe("unknown");
  });
});

// ---------------------------------------------------------------------------
// routineStatusRank / ROUTINE_STATUS_ORDER
// ---------------------------------------------------------------------------

describe("routineStatusRank", () => {
  it("ranks worst-first: unreadable < error < findings < unknown < skipped < never < ok", () => {
    const ranks = ROUTINE_STATUS_ORDER.map(routineStatusRank);
    for (let i = 1; i < ranks.length; i++) {
      expect(ranks[i]).toBeGreaterThan(ranks[i - 1]);
    }
  });

  it("covers every RoutineUiStatus exactly once", () => {
    expect(new Set(ROUTINE_STATUS_ORDER).size).toBe(ROUTINE_STATUS_ORDER.length);
    expect(ROUTINE_STATUS_ORDER).toHaveLength(7);
  });
});

// ---------------------------------------------------------------------------
// sortRoutinesWorstFirst
// ---------------------------------------------------------------------------

describe("sortRoutinesWorstFirst", () => {
  it("orders unreadable, error, findings, unknown, skipped, never, ok", () => {
    const okRow = summary({ name: "focus-export", latest: run({ routine: "focus-export", status: "ok" }) });
    const errorRow = summary({ name: "idryx-detect", latest: run({ routine: "idryx-detect", status: "error" }) });
    const findingsRow = summary({
      name: "mockryx-drill",
      latest: run({ routine: "mockryx-drill", status: "findings" }),
    });
    const neverRow = summary({ name: "qryx-trend", latest: null });
    const skippedRow = summary({ name: "verdryx-drift", latest: run({ routine: "verdryx-drift", status: "skipped" }) });

    const sorted = sortRoutinesWorstFirst([okRow, errorRow, findingsRow, neverRow, skippedRow]);
    expect(sorted.map((r) => r.name)).toEqual([
      "idryx-detect", // error
      "mockryx-drill", // findings
      "verdryx-drift", // skipped
      "qryx-trend", // never
      "focus-export", // ok
    ]);
  });

  it("breaks a full tie deterministically by routine name, and never mutates the input array", () => {
    const b = summary({ name: "qryx-trend", latest: null });
    const a = summary({ name: "focus-export", latest: null });
    const rows = [b, a];
    const original = [...rows];
    const sorted = sortRoutinesWorstFirst(rows);
    expect(sorted.map((r) => r.name)).toEqual(["focus-export", "qryx-trend"]);
    expect(rows).toEqual(original);
  });

  it("ranks an unreadable status file worse than a real error", () => {
    const errorRow = summary({ name: "idryx-detect", latest: run({ routine: "idryx-detect", status: "error" }) });
    const unreadableRow = summary({ name: "focus-export", latest: null, latest_error: "could not parse" });
    const sorted = sortRoutinesWorstFirst([errorRow, unreadableRow]);
    expect(sorted.map((r) => r.name)).toEqual(["focus-export", "idryx-detect"]);
  });
});

// ---------------------------------------------------------------------------
// latestDetailLine: reason-vs-summary precedence (mirrors routines.sh's
// last_status_line)
// ---------------------------------------------------------------------------

describe("latestDetailLine", () => {
  it("prefers reason for a skipped run when a reason is recorded", () => {
    const latest = run({
      routine: "verdryx-drift",
      status: "skipped",
      reason: "ROUTINE_VERDRYX_BASELINE is not set",
      summary: "should not be shown",
    });
    expect(latestDetailLine(latest)).toBe("ROUTINE_VERDRYX_BASELINE is not set");
  });

  it("prefers reason for an error run when a reason is recorded", () => {
    const latest = run({
      routine: "focus-export",
      status: "error",
      reason: "no calls found in the trace",
      summary: "tokenfuse-gateway focus-export exited 1",
    });
    expect(latestDetailLine(latest)).toBe("no calls found in the trace");
  });

  it("falls back to summary for skipped/error when no reason was recorded", () => {
    const latest = run({ routine: "focus-export", status: "error", reason: null, summary: "exited 1" });
    expect(latestDetailLine(latest)).toBe("exited 1");
  });

  it("uses summary (never reason) for ok/findings", () => {
    const latest = run({
      routine: "mockryx-drill",
      status: "findings",
      reason: "should never be shown for findings",
      summary: "2 gap(s) found",
    });
    expect(latestDetailLine(latest)).toBe("2 gap(s) found");
  });

  it("falls back to an honest placeholder when neither reason nor summary is recorded", () => {
    const latest = run({ routine: "qryx-trend", status: "ok", summary: null });
    expect(latestDetailLine(latest)).toBe("(no detail recorded)");
  });
});

// ---------------------------------------------------------------------------
// recordStatusTone
// ---------------------------------------------------------------------------

describe("recordStatusTone", () => {
  it("maps each of the four known statuses to its own tone", () => {
    expect(recordStatusTone("ok")).toBe(ROUTINE_STATUS_TONE.ok);
    expect(recordStatusTone("findings")).toBe(ROUTINE_STATUS_TONE.findings);
    expect(recordStatusTone("skipped")).toBe(ROUTINE_STATUS_TONE.skipped);
    expect(recordStatusTone("error")).toBe(ROUTINE_STATUS_TONE.error);
  });

  it("falls back to the 'unknown' tone for an unrecognized status, never throwing", () => {
    expect(recordStatusTone("a-future-status")).toBe(ROUTINE_STATUS_TONE.unknown);
  });
});

// ---------------------------------------------------------------------------
// latestRelativeTime
// ---------------------------------------------------------------------------

describe("latestRelativeTime", () => {
  it("is 'never' when there is no record at all", () => {
    expect(latestRelativeTime(null, Date.now())).toBe("never");
  });

  it("humanizes the age from finished_at", () => {
    const now = Date.parse("2026-07-23T06:12:00Z");
    const latest = run({ routine: "focus-export", finished_at: "2026-07-23T06:07:00Z" });
    expect(latestRelativeTime(latest, now)).toBe("5m ago");
  });

  it("is 'unknown' rather than throwing when finished_at does not parse", () => {
    const latest = run({ routine: "focus-export", finished_at: "not-a-timestamp" });
    expect(latestRelativeTime(latest, Date.now())).toBe("unknown");
  });
});
