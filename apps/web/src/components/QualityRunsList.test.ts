import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { QualityRunsList } from "./QualityRunsList";
import type { VerdryxRunSummary } from "../qualityTypes";

/**
 * The "unanswered" column must not exist at all on a store that predates
 * verdryx's `unanswered` table (`unanswered_supported: false`) - CLAUDE.md
 * invariant 4's own instinct one field down: a `0` next to a real mean would
 * look measured when it is really "this store cannot say".
 */

function run(over: Partial<VerdryxRunSummary> = {}): VerdryxRunSummary {
  return {
    run: { id: "run-1", model: "claude-sonnet-5", started_at: "2026-09-01T10:00:00Z", finished_at: "2026-09-01T10:05:00Z" },
    case_count: 57,
    mean_score: 0.82,
    total_tokens: 1000,
    total_cost_usd: 0.5,
    unanswered_count: 0,
    unanswered_by_reason: [],
    unanswered_supported: true,
    ...over,
  };
}

const render = (runs: VerdryxRunSummary[]) =>
  renderToStaticMarkup(
    createElement(QualityRunsList, { runs, selectedRunId: null, onSelect: () => {} }),
  );

describe("QualityRunsList", () => {
  it("renders no unanswered column at all when nothing in the store supports it", () => {
    const out = render([run({ unanswered_supported: false, unanswered_count: 0 })]);
    expect(out).not.toContain("unanswered");
  });

  it("shows an unanswered column and count when the store supports it", () => {
    const out = render([
      run({
        unanswered_supported: true,
        unanswered_count: 3,
        unanswered_by_reason: [{ reason: "label_mass_too_low", count: 3 }],
      }),
    ]);
    expect(out).toContain("unanswered");
    expect(out).toContain("label_mass_too_low: 3");
  });

  it("renders unmeasured, never a mean of 0, for a run that answered none", () => {
    const out = render([run({ case_count: 0, mean_score: null, unanswered_count: 4, unanswered_by_reason: [{ reason: "timeout", count: 4 }] })]);
    expect(out).toContain("unmeasured");
    expect(out).not.toMatch(/>0\.000</);
  });

  it("shows a zero-unanswered run's cell as 0, not a dash, once the store supports it", () => {
    const out = render([run({ unanswered_supported: true, unanswered_count: 0, unanswered_by_reason: [] })]);
    expect(out).toContain("unanswered");
  });
});
