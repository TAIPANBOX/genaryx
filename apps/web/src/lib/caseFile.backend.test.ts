import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./transport", () => ({
  hasBackend: vi.fn(),
  invokeBackend: vi.fn(),
}));

import { hasBackend, invokeBackend } from "./transport";
import { explainCase, fetchCase, refusalSentence } from "./caseFile";

const box = vi.mocked(hasBackend);
const ask = vi.mocked(invokeBackend);

const goodCase = { incident_id: "bus:1", subject: "s", digest: "sha256:ab", lines: [], facts: [] };

beforeEach(() => {
  vi.resetAllMocks();
  box.mockReturnValue(true);
});

describe("fetchCase against a box", () => {
  it("a_whole_case_comes_back_as_a_case", async () => {
    ask.mockResolvedValue(goodCase as never);
    await expect(fetchCase("bus:1")).resolves.toEqual({ kind: "case", case: goodCase });
    expect(ask).toHaveBeenCalledWith("case_file", { incident_id: "bus:1" });
  });

  it.each([
    ["null", null],
    ["a string", "nope"],
    ["a case with no facts", { ...goodCase, facts: undefined }],
    ["a case with a numeric digest", { ...goodCase, digest: 3 }],
  ])("%s_is_not_a_case_and_is_said_to_be_unreachable", async (_n, raw) => {
    ask.mockResolvedValue(raw as never);
    const r = await fetchCase("bus:1");
    expect(r).toEqual({ kind: "unreachable", message: "the box answered with something that is not a case" });
  });

  it("a_refusal_the_command_knows_is_passed_on_whole", async () => {
    const error = { kind: "not_found", event_id: 9 };
    ask.mockRejectedValue(error);
    await expect(fetchCase("bus:9")).resolves.toEqual({ kind: "refused", error });
  });

  it.each([
    [{ kind: "no_store", reason: "r" }],
    [{ kind: "bad_incident_id", incident_id: "x" }],
    [{ kind: "unsupported_source", source: "money" }],
  ])("the_known_refusal_%j_is_passed_on_whole", async (error) => {
    ask.mockRejectedValue(error);
    await expect(fetchCase("bus:9")).resolves.toEqual({ kind: "refused", error });
  });

  it.each([
    ["an unknown refusal kind", { kind: "teapot" }],
    ["a refusal whose kind is not text", { kind: 4 }],
  ])("%s_is_an_unreachable_box_not_a_refusal", async (_n, err) => {
    ask.mockRejectedValue(err);
    const r = await fetchCase("bus:9");
    expect(r.kind).toBe("unreachable");
  });

  it("an_error_keeps_its_message_and_anything_else_gets_a_default_sentence", async () => {
    ask.mockRejectedValueOnce(new Error("socket closed"));
    await expect(fetchCase("bus:1")).resolves.toEqual({ kind: "unreachable", message: "socket closed" });
    ask.mockRejectedValueOnce(undefined);
    await expect(fetchCase("bus:1")).resolves.toEqual({
      kind: "unreachable",
      message: "the box did not answer the case command",
    });
  });
});

describe("explainCase against a box", () => {
  it("with_no_box_it_says_so", async () => {
    box.mockReturnValue(false);
    await expect(explainCase("bus:1")).resolves.toEqual({ kind: "no_backend" });
    expect(ask).not.toHaveBeenCalled();
  });

  it("an_answer_keeps_its_text_and_only_the_text_claims_it_could_not_support", async () => {
    ask.mockResolvedValue({ text: "read", unsupported_claims: ["a", 3, "b"] } as never);
    await expect(explainCase("bus:1")).resolves.toEqual({ kind: "answer", text: "read", unsupported: ["a", "b"] });
  });

  it("a_missing_or_malformed_unsupported_list_is_an_empty_one", async () => {
    ask.mockResolvedValue({ text: "read", unsupported_claims: "x" } as never);
    await expect(explainCase("bus:1")).resolves.toEqual({ kind: "answer", text: "read", unsupported: [] });
  });

  it.each([[null], ["str"], [{ text: 5 }]])("a_non_answer_%j_is_refused_in_a_sentence", async (raw) => {
    ask.mockResolvedValue(raw as never);
    const r = await explainCase("bus:1");
    expect(r).toEqual({ kind: "refused", message: "Felyx answered with something that is not an answer." });
  });

  it("a_rejection_is_shown_as_its_own_sentence_whatever_shape_it_has", async () => {
    ask.mockRejectedValueOnce("no provider");
    await expect(explainCase("bus:1")).resolves.toEqual({ kind: "refused", message: "no provider" });
    ask.mockRejectedValueOnce(new Error("boom"));
    await expect(explainCase("bus:1")).resolves.toEqual({ kind: "refused", message: "boom" });
    ask.mockRejectedValueOnce(42);
    await expect(explainCase("bus:1")).resolves.toEqual({ kind: "refused", message: "Felyx did not answer." });
  });
});

describe("refusalSentence", () => {
  it("each_refusal_has_its_own_sentence_and_an_unknown_one_says_the_build_does_not_know_it", () => {
    expect(refusalSentence({ kind: "unsupported_source", source: "money" })).toBe(
      "No case for this incident: a money incident has no event row to start from.",
    );
    expect(refusalSentence({ kind: "not_found", event_id: 7 })).toContain("event row 7");
    expect(refusalSentence({ kind: "bad_incident_id", incident_id: "x" })).toContain("not an incident id");
    expect(refusalSentence({ kind: "no_store", reason: "r" })).toContain("no readable event store");
    expect(refusalSentence({ kind: "teapot" } as never)).toContain("does not know");
  });
});
