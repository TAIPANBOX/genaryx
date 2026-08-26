/**
 * The Money tab renders what the money plane gave it, and says so when it was
 * given nothing.
 *
 * The board and the savings section are rendered for real here
 * (`react-dom/server`, no DOM needed) rather than tested through their label
 * helpers alone: a helper that returns the right string proves nothing about a
 * component that never calls it, and "the field renders nowhere" was the whole
 * defect. `RunsBoard` and `GovernedSavingsSection` take plain props and touch
 * no browser API, so a static render is the real component.
 */

import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { BOARD_MIN_WIDTH_PX, RUN_COL_MIN_PX, RUNS_BOARD_TRACKS, RunsBoard } from "../components/RunsBoard";
import { GovernedSavingsSection } from "../components/MoneyView";
import {
  cacheHitsLabel,
  countLabel,
  governedSavingsCaption,
  NOT_RECORDED,
  RUNS_EXPORT_COLUMNS,
  runModelLabel,
  runsExportMeta,
  runsExportRows,
  runUnitLabel,
} from "./moneyExport";
import { toCsv, toJson } from "./download";
import type { Run, Savings } from "../moneyTypes";

const RUN: Run = {
  run_id: "run-0217",
  unit: "financial-crime",
  model: "claude-sonnet-4",
  agent_id: "agent://meridian.example/treasury/cashflow-forecaster",
  spent_usd: 5.85,
  budget_usd: 10,
  calls: 12,
  cache_hits: 148,
  steps: 3,
  last_seen: "2026-08-20T10:00:00Z",
  killed: false,
};

const SAVINGS: Savings = {
  blocked_spend_usd: 120.5,
  cache_saved_usd: 40.25,
  router_saved_usd: 12,
  budget_breaks: 3,
  total_saved_usd: 172.75,
};

function board(runs: Run[]): string {
  return renderToStaticMarkup(
    createElement(RunsBoard, {
      runs,
      onKill: async () => {},
      onSetBudget: async () => {},
      onOpenAgentAt: () => {},
      onReplayRun: () => {},
    }),
  );
}

function savingsSection(savings: Savings): string {
  return renderToStaticMarkup(createElement(GovernedSavingsSection, { savings }));
}

/** The cell that carries calls, steps and cache hits, isolated so an assertion
 * about the cache figure cannot accidentally be satisfied by the same digits
 * appearing in a spend amount or a run id somewhere else in the row. */
function numericCell(html: string): string {
  const m = html.match(/<div class="d-num cell-r">(.*?)<\/div><div><span class="d-pill/);
  return m ? m[1] : "";
}

describe("the runs board renders the run fields that reach it", () => {
  it("shows the cache hits the money plane counted for the run", () => {
    expect(numericCell(board([RUN]))).toContain("148");
  });

  it("shows the model that priced the run and the unit that was charged", () => {
    const html = board([RUN]);
    expect(html).toContain("claude-sonnet-4");
    expect(html).toContain("financial-crime");
  });

  it("says a cache_hits of 0 is a count, not an absence", () => {
    expect(numericCell(board([{ ...RUN, cache_hits: 0 }]))).toContain("0");
    expect(numericCell(board([{ ...RUN, cache_hits: 0 }]))).not.toContain(NOT_RECORDED);
  });

  it("says so, rather than showing 0, when no cache_hits arrived at all", () => {
    // The field is non-optional in the wire type, so the only way a payload
    // can lack it is a box that is not what the type claims. That is exactly
    // the case where a confident 0 would be a fabricated measurement.
    const withoutField = { ...RUN } as Partial<Run>;
    delete withoutField.cache_hits;
    const cell = numericCell(board([withoutField as Run]));
    expect(cell).toContain(NOT_RECORDED);
    expect(cell).not.toMatch(/(^|[^\d])0([^\d]|$)/);
  });

  it("says an unresolved unit is the Cloud's answer, not a missing value", () => {
    const html = board([{ ...RUN, unit: "" }]);
    expect(html).toContain("no unit resolved");
    expect(html).not.toContain(`unit ${NOT_RECORDED}`);
  });

  it("says so when the run carried no model", () => {
    expect(board([{ ...RUN, model: "" }])).toContain(NOT_RECORDED);
  });
});

/**
 * The board is a CSS grid whose first track used to be `minmax(0, 1.5fr)`,
 * which is a track allowed to reach zero. @measured in the running mock demo
 * on 2026-08-26, in the default 1440px layout with both rails open, the runs
 * card is 760px wide and the fixed tracks plus gaps plus padding came to
 * exactly 720px: the run id and the agent name were laid out 0px wide and
 * nothing at all was drawn there. At origin/main's column widths the same
 * measurement gave 8px, so the collapse predates this branch and widening the
 * numeric column by 8px is what took it the rest of the way.
 *
 * Nothing here can measure layout: `renderToStaticMarkup` has no layout
 * engine. What these hold is the arithmetic that a browser then applies, and
 * the fault they catch is the one that actually happened, a track widened
 * without the board's own minimum being widened with it.
 */
describe("the run column cannot be laid out to nothing", () => {
  it("gives the run track a pixel floor rather than letting it reach zero", () => {
    const html = board([RUN]);
    expect(html).toContain(`minmax(${RUN_COL_MIN_PX}px, 1.5fr)`);
    expect(html).not.toContain("minmax(0,");
    expect(RUN_COL_MIN_PX).toBeGreaterThanOrEqual(180);
  });

  it("derives the board's minimum width from the tracks, so a wider column cannot drift out of step", () => {
    // `.d-th`/`.d-tr` in index.css: 14px column gap, 20px of padding a side.
    const derived =
      RUNS_BOARD_TRACKS.reduce((n, t) => n + t.minPx, 0) + 14 * (RUNS_BOARD_TRACKS.length - 1) + 20 * 2;
    expect(BOARD_MIN_WIDTH_PX).toBe(derived);
  });

  it("scrolls the board sideways instead of squeezing a column out of existence", () => {
    const html = board([RUN]);
    expect(html).toContain("overflow-x:auto");
    expect(html).toContain(`min-width:${BOARD_MIN_WIDTH_PX}px`);
  });

  it("keeps its own width off the page layout, so the floor cannot push the rail away", () => {
    // The floor raises this board's min-content width to the whole 980, and
    // `.d-main` sizes its main column from that. @measured on 2026-08-26 the
    // grid became `982px 360px` in a 946px container and the Governed savings
    // caption ended at x=1595 of a 1440px window. With containment the same
    // grid measured `570px 360px` and the caption ended at x=1183. Dropping
    // this line puts the rail back off the screen, and no other assertion in
    // this file would notice.
    expect(board([RUN])).toContain("contain:inline-size");
  });

  it("still draws the run id and the agent when the board is that narrow", () => {
    // The markup carries them either way; this is the assertion that would
    // have stayed green through the whole defect, and it is here to say so.
    const html = board([RUN]);
    expect(html).toContain("run-0217");
    expect(html).toContain("cashflow-forecaster");
  });
});

describe("the Money tab's savings section says how often a budget broke", () => {
  it("renders the budget break count that until now showed only on Overview", () => {
    expect(savingsSection(SAVINGS)).toContain("3 budget breaks");
  });

  it("keeps saying what the composition underneath is made of", () => {
    expect(savingsSection(SAVINGS)).toContain("prevented + recovered");
  });

  it("counts one break in the singular", () => {
    expect(governedSavingsCaption({ ...SAVINGS, budget_breaks: 1 })).toBe(
      "1 budget break · prevented + recovered",
    );
  });

  it("keeps zero breaks as a count: nothing tripped is a result", () => {
    expect(governedSavingsCaption({ ...SAVINGS, budget_breaks: 0 })).toBe(
      "0 budget breaks · prevented + recovered",
    );
  });

  it("says so when the break count never arrived", () => {
    const partial = { ...SAVINGS } as Partial<Savings>;
    delete partial.budget_breaks;
    expect(governedSavingsCaption(partial as Savings)).toBe(
      `budget breaks ${NOT_RECORDED} · prevented + recovered`,
    );
  });
});

describe("the labels keep a zero apart from an absence", () => {
  it("formats a real count", () => {
    expect(countLabel(1240)).toBe("1,240");
    expect(countLabel(0)).toBe("0");
  });

  it("refuses to turn a missing or unusable number into one", () => {
    expect(countLabel(undefined)).toBe(NOT_RECORDED);
    expect(countLabel(null)).toBe(NOT_RECORDED);
    expect(countLabel(Number.NaN)).toBe(NOT_RECORDED);
  });

  it("reads cache hits, the model and the unit off a run", () => {
    expect(cacheHitsLabel(RUN)).toBe("148");
    expect(runModelLabel(RUN)).toBe("claude-sonnet-4");
    expect(runUnitLabel(RUN)).toBe("financial-crime");
    expect(runUnitLabel({ ...RUN, unit: "" })).toBe("no unit resolved");
    expect(runModelLabel({ ...RUN, model: "" })).toBe(NOT_RECORDED);
  });
});

describe("the runs export is the whole list, and says what its blanks mean", () => {
  const META = { shown: 18, total: 402, environment: "console.example", takenAt: "2026-08-26T09:00:00.000Z" };

  it("carries the three fields the table itself does not print as columns", () => {
    const [row] = runsExportRows([RUN]);
    expect(row.unit).toBe("financial-crime");
    expect(row.model).toBe("claude-sonnet-4");
    expect(row.cache_hits).toBe(148);
    expect(row.last_seen).toBe("2026-08-20T10:00:00Z");
  });

  it("writes an absent value as null so the CSV leaves the cell empty", () => {
    const withoutCache = { ...RUN, unit: "", model: "", budget_usd: null } as Partial<Run>;
    delete withoutCache.cache_hits;
    const [row] = runsExportRows([withoutCache as Run]);
    expect(row.cache_hits).toBeNull();
    expect(row.unit).toBeNull();
    expect(row.model).toBeNull();
    expect(row.budget_usd).toBeNull();

    const csv = toCsv(RUNS_EXPORT_COLUMNS, [row], runsExportMeta(META));
    const lines = csv.trim().split("\n");
    const dataLine = lines[lines.length - 1];
    // run_id, agent_id, then unit and model as two empty cells in a row.
    expect(dataLine).toContain(",,,");
    expect(dataLine).not.toContain(",0,");
  });

  it("keeps a counted zero as a zero", () => {
    const [row] = runsExportRows([{ ...RUN, cache_hits: 0 }]);
    expect(row.cache_hits).toBe(0);
  });

  it("keeps an empty agent_id verbatim: that is what the plane sent", () => {
    const [row] = runsExportRows([{ ...RUN, agent_id: "" }]);
    expect(row.agent_id).toBe("");
  });

  it("states that the file is not the slice the table showed", () => {
    const meta = runsExportMeta(META);
    const joined = (meta.caveats ?? []).join(" ");
    expect(joined).toContain("402");
    expect(joined).toContain("18");
    expect(joined).toMatch(/not the .* the Runs table shows/);
  });

  it("states what an empty budget cell means, since it is the one that reads as good news", () => {
    const joined = (runsExportMeta(META).caveats ?? []).join(" ");
    expect(joined).toContain("budget_usd");
    expect(joined).toMatch(/never that the run has none/);
  });

  it("states that an empty cache_hits and a zero are different statements", () => {
    const joined = (runsExportMeta(META).caveats ?? []).join(" ");
    expect(joined).toMatch(/An empty cache_hits/);
    expect(joined).toMatch(/cache_hits of 0/);
  });

  it("never leaves the provenance block bare", () => {
    const meta = runsExportMeta(META);
    expect(meta.subject.length).toBeGreaterThan(0);
    expect(meta.environment).toBe("console.example");
    expect(meta.takenAt).toBe("2026-08-26T09:00:00.000Z");
    expect(meta.windows.length).toBeGreaterThan(0);
    expect((meta.caveats ?? []).length).toBeGreaterThan(0);

    const csv = toCsv(RUNS_EXPORT_COLUMNS, runsExportRows([RUN]), meta);
    expect(csv.startsWith("# subject: Genaryx money runs")).toBe(true);
    expect(csv).toContain("# taken_at: 2026-08-26T09:00:00.000Z");
    expect(csv).toContain("# window: runs, spend, calls and cache hits");

    const json = JSON.parse(toJson(runsExportRows([RUN]), meta)) as { meta: unknown; rows: unknown[] };
    expect(json.meta).toEqual(meta);
    expect(json.rows).toHaveLength(1);
  });
});
