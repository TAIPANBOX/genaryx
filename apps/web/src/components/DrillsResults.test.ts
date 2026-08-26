import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { DrillsResults } from "./DrillsResults";
import { formatTimestamp } from "../lib/format";
import type { MockryxFinding, MockryxReport, MockryxResult } from "../drillsTypes";

/**
 * What the drill report SAYS, rendered.
 *
 * Server-rendered rather than mounted: this repo has no DOM in its test
 * environment (`vitest.config.ts` is `environment: "node"`, and there is no
 * jsdom in the tree), and `DrillsResults` takes its whole world as one prop,
 * so static markup is the real output and not a stand-in for it.
 */

function finding(over: Partial<MockryxFinding> = {}): MockryxFinding {
  return {
    scenario: "budget-ceiling",
    step: "second call",
    attempt: 2,
    expect_status: 402,
    expect_header: null,
    got_status: 200,
    got_headers: null,
    detail: "the gateway allowed a call past the ceiling",
    expect_event_source: null,
    expect_event_type: null,
    ...over,
  };
}

function result(over: Partial<MockryxResult> = {}): MockryxResult {
  return {
    scenario: "budget-ceiling",
    status: "failed",
    findings: [finding()],
    skipped_findings: [],
    metrics: { calls: 3, budget_burned_usd: 0.41 },
    ...over,
  };
}

function report(over: Partial<MockryxReport> = {}): MockryxReport {
  return {
    run_id: "drill-9f21c4",
    gateway: "http://127.0.0.1:8080",
    generated_at: "2026-08-25T09:14:02.113Z",
    results: [result()],
    ...over,
  };
}

const render = (r: MockryxReport) => renderToStaticMarkup(createElement(DrillsResults, { report: r }));

describe("DrillsResults", () => {
  // `generated_at` is the report's OWN clock, and the only field that ties
  // what is on screen to the JSON file `--save` wrote. The view's other
  // timestamp is `Date.now()` at the moment the click returned, which is a
  // different measurement of a different thing.
  it("says when the run happened, from the report's own clock", () => {
    const r = report();
    expect(render(r)).toContain(formatTimestamp(r.generated_at));
  });

  // A report with no clock in it is a report with no clock in it. "Invalid
  // Date", or the word "undefined", would be this panel answering a question
  // nobody could answer.
  it("says the report carried no time rather than printing a broken one", () => {
    const html = render(report({ generated_at: undefined as unknown as string }));
    expect(html).toContain("time not recorded");
    expect(html).not.toContain("Invalid Date");
    expect(html).not.toContain("undefined");
  });

  // The finding carries its own `scenario`, and inside a card already titled
  // with that name it is the same word twice. It is worth exactly one thing:
  // saying so when the two DISAGREE, which is the runner attributing a
  // mismatch to a scenario this card is not about.
  it("names a finding whose scenario is not the card's", () => {
    const html = render(report({ results: [result({ findings: [finding({ scenario: "egress-allowlist" })] })] }));
    expect(html).toContain("egress-allowlist");
  });

  it("does not repeat the scenario name when the finding agrees with the card", () => {
    const html = render(report());
    const occurrences = html.split("budget-ceiling").length - 1;
    expect(occurrences).toBe(1);
  });
});
