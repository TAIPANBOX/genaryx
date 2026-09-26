import { describe, expect, it } from "vitest";
import { formatQualityMean, type QualityMeanFields } from "./quality";

/**
 * `formatQualityMean` is the one sentence the Quality panel reads instead of
 * a bare number, so it is tested standalone: "mean 0.82 over 57 answered, 3
 * unanswered (label_mass_too_low: 3)" for a partly-unanswered run, and
 * "unmeasured" (never a mean of 0) for a run that answered none at all.
 */

function fields(over: Partial<QualityMeanFields> = {}): QualityMeanFields {
  return {
    mean_score: 0.82,
    case_count: 57,
    unanswered_count: 0,
    unanswered_by_reason: [],
    unanswered_supported: true,
    ...over,
  };
}

describe("formatQualityMean", () => {
  it("reads the mean, the answered count and the unanswered breakdown together", () => {
    expect(
      formatQualityMean(
        fields({ unanswered_count: 3, unanswered_by_reason: [{ reason: "label_mass_too_low", count: 3 }] }),
      ),
    ).toBe("mean 0.82 over 57 answered, 3 unanswered (label_mass_too_low: 3)");
  });

  it("joins several reasons in order", () => {
    expect(
      formatQualityMean(
        fields({
          unanswered_count: 5,
          unanswered_by_reason: [
            { reason: "label_mass_too_low", count: 3 },
            { reason: "timeout", count: 2 },
          ],
        }),
      ),
    ).toBe("mean 0.82 over 57 answered, 5 unanswered (label_mass_too_low: 3, timeout: 2)");
  });

  it("says unmeasured, never a mean of 0, for a run that answered none", () => {
    expect(formatQualityMean(fields({ mean_score: null, case_count: 0, unanswered_count: 4 }))).toBe("unmeasured");
  });

  it("omits the unanswered half entirely when the store predates the unanswered table", () => {
    expect(
      formatQualityMean(fields({ unanswered_supported: false, unanswered_count: 0 })),
    ).toBe("mean 0.82 over 57 answered");
  });

  it("omits the unanswered half when supported but zero", () => {
    expect(formatQualityMean(fields({ unanswered_count: 0, unanswered_by_reason: [] }))).toBe(
      "mean 0.82 over 57 answered",
    );
  });
});
