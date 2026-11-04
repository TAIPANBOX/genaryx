import { describe, expect, it } from "vitest";

import { caseIncidentId, coverage, factsOf, fetchCase, shortDigest } from "./caseFile";
import type { CaseFile } from "../caseTypes";

describe("the case reader", () => {
  it("a_case_starts_only_from_an_incident_with_an_event_row", () => {
    expect(caseIncidentId({ id: "bus:7", source: "bus" })).toBe("bus:7");
    expect(caseIncidentId({ id: "verdryx:9", source: "verdryx" })).toBe("verdryx:9");
    for (const source of ["money", "idryx", "posture"] as const) {
      expect(caseIncidentId({ id: `${source}:x`, source })).toBeNull();
    }
  });

  it("with_no_box_to_ask_the_reader_says_so_and_invents_no_case", async () => {
    await expect(fetchCase("bus:1")).resolves.toEqual({ kind: "no_backend" });
  });

  it("every_line_of_a_case_is_in_exactly_one_count", () => {
    const c = {
      lines: [
        { section: "event", plane: "bus", state: "answered", facts: 1, reason: null, upstream: null },
        { section: "spend", plane: "tokenfuse", state: "empty", facts: 0, reason: "x", upstream: null },
        { section: "policy", plane: "wardryx", state: "unavailable", facts: 0, reason: "x", upstream: null },
        { section: "decisions", plane: "wardryx", state: "not_asked", facts: 0, reason: "x", upstream: null },
      ],
    } as Pick<CaseFile, "lines">;
    const n = coverage(c);
    expect(n).toEqual({ answered: 1, empty: 1, unavailable: 1, not_asked: 1 });
    expect(n.answered + n.empty + n.unavailable + n.not_asked).toBe(c.lines.length);
  });

  it("a_section_lists_only_its_own_facts_and_a_digest_is_shortened_for_reading", () => {
    const c = {
      facts: [
        { section: "spend", plane: "tokenfuse", row_id: "r", digest: "sha256:" + "0".repeat(64), label: "l", trust: "data", row: {} },
        { section: "policy", plane: "wardryx", row_id: "v", digest: "sha256:" + "1".repeat(64), label: "l", trust: "data", row: {} },
      ],
    } as Pick<CaseFile, "facts">;
    expect(factsOf(c, "spend").map((f) => f.row_id)).toEqual(["r"]);
    expect(factsOf(c, "event")).toEqual([]);
    expect(shortDigest("sha256:" + "ab".repeat(32))).toBe("sha256:abababababab");
  });
});
